use anyhow::{bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

const SOCKS_VERSION: u8 = 5;
const AUTH_METHOD_NO_AUTH: u8 = 0;
const CMD_CONNECT: u8 = 1;
const REPLY_SUCCEEDED: u8 = 0;
const REPLY_GENERAL_FAILURE: u8 = 1;
const REPLY_CONNECTION_NOT_ALLOWED: u8 = 2;
const REPLY_COMMAND_NOT_SUPPORTED: u8 = 7;
const REPLY_ADDRESS_TYPE_NOT_SUPPORTED: u8 = 8;

#[derive(Debug)]
pub struct RunningEgressProxy {
    bind_addr: SocketAddr,
    task: JoinHandle<()>,
}

impl RunningEgressProxy {
    pub fn port(&self) -> u16 {
        self.bind_addr.port()
    }

    pub fn abort(self) {
        self.task.abort();
    }
}

pub struct EgressProxy {
    resolved_hosts: HashMap<String, Vec<IpAddr>>,
    allowed_ips: HashSet<IpAddr>,
    bind_addr: SocketAddr,
    sandbox_id: String,
}

impl EgressProxy {
    pub async fn start(
        sandbox_id: String,
        allow_hostnames: Vec<String>,
    ) -> Result<RunningEgressProxy> {
        let resolved_hosts = resolve_hosts(&sandbox_id, &allow_hostnames).await;
        let allowed_ips = resolved_hosts
            .values()
            .flat_map(|ips| ips.iter().copied())
            .collect::<HashSet<_>>();

        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .context("bind egress proxy")?;
        let bind_addr = listener.local_addr().context("local_addr egress proxy")?;

        let proxy = Self {
            resolved_hosts,
            allowed_ips,
            bind_addr,
            sandbox_id,
        };

        let task = tokio::spawn(async move {
            if let Err(error) = proxy.run(listener).await {
                tracing::error!(error = %error, "egress proxy terminated with error");
            }
        });

        Ok(RunningEgressProxy { bind_addr, task })
    }

    async fn run(self, listener: TcpListener) -> Result<()> {
        tracing::info!(
            sandbox_id = %self.sandbox_id,
            addr = %self.bind_addr,
            "egress proxy started"
        );

        loop {
            let (stream, peer_addr) = listener.accept().await.context("accept egress proxy")?;
            let shared = self.shared_state();
            tokio::spawn(async move {
                if let Err(error) = shared.handle_client(stream).await {
                    tracing::debug!(peer = %peer_addr, error = %error, "egress proxy connection ended");
                }
            });
        }
    }

    fn shared_state(&self) -> Self {
        Self {
            resolved_hosts: self.resolved_hosts.clone(),
            allowed_ips: self.allowed_ips.clone(),
            bind_addr: self.bind_addr,
            sandbox_id: self.sandbox_id.clone(),
        }
    }

    async fn handle_socks5(&self, mut client: TcpStream) -> Result<()> {
        self.read_greeting(&mut client).await?;
        self.write_method_selection(&mut client).await?;

        let target = match self.read_request(&mut client).await {
            Ok(target) => target,
            Err(RequestError::CommandNotSupported) => {
                write_reply(&mut client, REPLY_COMMAND_NOT_SUPPORTED).await?;
                bail!("only CONNECT is supported");
            }
            Err(RequestError::AddressTypeNotSupported) => {
                write_reply(&mut client, REPLY_ADDRESS_TYPE_NOT_SUPPORTED).await?;
                bail!("address type is not supported");
            }
            Err(RequestError::Other(error)) => {
                write_reply(&mut client, REPLY_GENERAL_FAILURE).await?;
                return Err(error);
            }
        };

        let upstream_addr = match self.resolve_target(&target) {
            Some(addr) => addr,
            None => {
                tracing::info!(
                    sandbox_id = %self.sandbox_id,
                    target = %target.log_label(),
                    "egress connection denied"
                );
                write_reply(&mut client, REPLY_CONNECTION_NOT_ALLOWED).await?;
                return Ok(());
            }
        };

        let mut upstream = self.connect_target(&target, upstream_addr).await?;

        write_reply(&mut client, REPLY_SUCCEEDED).await?;

        tracing::debug!(
            sandbox_id = %self.sandbox_id,
            target = %target.log_label(),
            upstream = %upstream_addr,
            "egress connection allowed"
        );

        tokio::io::copy_bidirectional(&mut client, &mut upstream)
            .await
            .context("proxy relay")?;

        Ok(())
    }

    async fn handle_client(&self, client: TcpStream) -> Result<()> {
        let mut first_byte = [0u8; 1];
        let peeked = client
            .peek(&mut first_byte)
            .await
            .context("peek client preface")?;
        if peeked == 0 {
            bail!("client closed before sending a request");
        }

        if first_byte[0] == SOCKS_VERSION {
            self.handle_socks5(client).await
        } else {
            self.handle_http_connect(client).await
        }
    }

    async fn handle_http_connect(&self, mut client: TcpStream) -> Result<()> {
        let request = read_http_request(&mut client).await?;
        let target = parse_http_connect_target(&request)?;
        let upstream_addr = match self.resolve_target(&target) {
            Some(addr) => addr,
            None => {
                tracing::info!(
                    sandbox_id = %self.sandbox_id,
                    target = %target.log_label(),
                    "egress connection denied"
                );
                client
                    .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                    .await
                    .context("write HTTP deny response")?;
                return Ok(());
            }
        };

        let mut upstream = self.connect_target(&target, upstream_addr).await?;
        client
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .context("write HTTP connect response")?;

        tracing::debug!(
            sandbox_id = %self.sandbox_id,
            target = %target.log_label(),
            upstream = %upstream_addr,
            "egress HTTP CONNECT allowed"
        );

        tokio::io::copy_bidirectional(&mut client, &mut upstream)
            .await
            .context("http proxy relay")?;

        Ok(())
    }

    async fn read_greeting(&self, client: &mut TcpStream) -> Result<()> {
        let mut header = [0u8; 2];
        client.read_exact(&mut header).await?;
        if header[0] != SOCKS_VERSION {
            bail!("not a SOCKS5 client");
        }

        let method_count = header[1] as usize;
        let mut methods = vec![0u8; method_count];
        client.read_exact(&mut methods).await?;

        if !methods.contains(&AUTH_METHOD_NO_AUTH) {
            bail!("client does not support no-auth SOCKS5");
        }

        Ok(())
    }

    async fn write_method_selection(&self, client: &mut TcpStream) -> Result<()> {
        client
            .write_all(&[SOCKS_VERSION, AUTH_METHOD_NO_AUTH])
            .await
            .context("write method selection")
    }

    async fn read_request(
        &self,
        client: &mut TcpStream,
    ) -> std::result::Result<TargetAddr, RequestError> {
        let mut header = [0u8; 4];
        client
            .read_exact(&mut header)
            .await
            .map_err(|e| RequestError::Other(e.into()))?;

        if header[0] != SOCKS_VERSION {
            return Err(RequestError::Other(anyhow::anyhow!(
                "invalid SOCKS version"
            )));
        }
        if header[1] != CMD_CONNECT {
            return Err(RequestError::CommandNotSupported);
        }

        let target = match header[3] {
            1 => {
                let mut ip = [0u8; 4];
                client
                    .read_exact(&mut ip)
                    .await
                    .map_err(|e| RequestError::Other(e.into()))?;
                let port = read_port(client).await?;
                TargetAddr::Ip(IpAddr::V4(Ipv4Addr::from(ip)), port)
            }
            3 => {
                let host_len = client
                    .read_u8()
                    .await
                    .map_err(|e| RequestError::Other(e.into()))?
                    as usize;
                let mut hostname = vec![0u8; host_len];
                client
                    .read_exact(&mut hostname)
                    .await
                    .map_err(|e| RequestError::Other(e.into()))?;
                let hostname =
                    String::from_utf8(hostname).map_err(|e| RequestError::Other(e.into()))?;
                let port = read_port(client).await?;
                TargetAddr::Domain(hostname, port)
            }
            4 => {
                let mut ip = [0u8; 16];
                client
                    .read_exact(&mut ip)
                    .await
                    .map_err(|e| RequestError::Other(e.into()))?;
                let port = read_port(client).await?;
                TargetAddr::Ip(IpAddr::from(ip), port)
            }
            _ => return Err(RequestError::AddressTypeNotSupported),
        };

        Ok(target)
    }

    fn resolve_target(&self, target: &TargetAddr) -> Option<SocketAddr> {
        match target {
            TargetAddr::Ip(ip, port) if self.allowed_ips.contains(ip) => {
                Some(SocketAddr::new(*ip, *port))
            }
            TargetAddr::Domain(hostname, port) => self
                .resolved_hosts
                .get(hostname)
                .and_then(|ips| ips.first().copied())
                .map(|ip| SocketAddr::new(ip, *port)),
            _ => None,
        }
    }

    async fn connect_target(
        &self,
        target: &TargetAddr,
        upstream_addr: SocketAddr,
    ) -> Result<TcpStream> {
        TcpStream::connect(upstream_addr).await.with_context(|| {
            format!(
                "connect upstream {} for {}",
                upstream_addr,
                target.log_label()
            )
        })
    }
}

async fn resolve_hosts(
    sandbox_id: &str,
    allow_hostnames: &[String],
) -> HashMap<String, Vec<IpAddr>> {
    let mut resolved_hosts = HashMap::new();

    for host in allow_hostnames {
        match tokio::net::lookup_host((host.as_str(), 443)).await {
            Ok(addrs) => {
                let mut unique = Vec::new();
                let mut seen = HashSet::new();
                for addr in addrs {
                    if seen.insert(addr.ip()) {
                        unique.push(addr.ip());
                    }
                }

                if unique.is_empty() {
                    tracing::warn!(sandbox_id = %sandbox_id, host = %host, "allowlisted host resolved to no IPs");
                    continue;
                }

                tracing::debug!(sandbox_id = %sandbox_id, host = %host, ips = ?unique, "allowlisted host resolved");
                resolved_hosts.insert(host.clone(), unique);
            }
            Err(error) => {
                tracing::warn!(
                    sandbox_id = %sandbox_id,
                    host = %host,
                    error = %error,
                    "allowlisted host could not be resolved and will be denied"
                );
            }
        }
    }

    resolved_hosts
}

async fn read_port(client: &mut TcpStream) -> std::result::Result<u16, RequestError> {
    let mut port = [0u8; 2];
    client
        .read_exact(&mut port)
        .await
        .map_err(|e| RequestError::Other(e.into()))?;
    Ok(u16::from_be_bytes(port))
}

async fn write_reply(client: &mut TcpStream, reply: u8) -> Result<()> {
    client
        .write_all(&[SOCKS_VERSION, reply, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .context("write SOCKS5 reply")
}

async fn read_http_request(client: &mut TcpStream) -> Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];

    loop {
        let read = client.read(&mut chunk).await.context("read HTTP request")?;
        if read == 0 {
            bail!("incomplete HTTP proxy request");
        }
        request.extend_from_slice(&chunk[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(request);
        }
        if request.len() > 16 * 1024 {
            bail!("HTTP proxy request too large");
        }
    }
}

fn parse_http_connect_target(request: &[u8]) -> Result<TargetAddr> {
    let request = std::str::from_utf8(request).context("HTTP proxy request is not valid UTF-8")?;
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing HTTP request line"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let authority = parts.next().unwrap_or_default();

    if !method.eq_ignore_ascii_case("CONNECT") {
        bail!("HTTP proxy only supports CONNECT");
    }

    let authority = authority
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("CONNECT authority must be host:port"))?;
    let port = authority
        .1
        .parse::<u16>()
        .with_context(|| format!("invalid CONNECT port '{}'", authority.1))?;

    if let Ok(ip) = authority.0.parse::<IpAddr>() {
        Ok(TargetAddr::Ip(ip, port))
    } else {
        Ok(TargetAddr::Domain(authority.0.to_string(), port))
    }
}

#[derive(Debug)]
enum RequestError {
    CommandNotSupported,
    AddressTypeNotSupported,
    Other(anyhow::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TargetAddr {
    Ip(IpAddr, u16),
    Domain(String, u16),
}

impl TargetAddr {
    fn log_label(&self) -> String {
        match self {
            Self::Ip(ip, port) => format!("{ip}:{port}"),
            Self::Domain(host, port) => format!("{host}:{port}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    fn proxy_with(hosts: &[(&str, &[IpAddr])]) -> EgressProxy {
        let resolved_hosts = hosts
            .iter()
            .map(|(host, ips)| ((*host).to_string(), ips.to_vec()))
            .collect::<HashMap<_, _>>();
        let allowed_ips = hosts
            .iter()
            .flat_map(|(_, ips)| ips.iter().copied())
            .collect::<HashSet<_>>();

        EgressProxy {
            resolved_hosts,
            allowed_ips,
            bind_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            sandbox_id: "test".into(),
        }
    }

    async fn bind_test_listener() -> Result<Option<TcpListener>> {
        match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await {
            Ok(listener) => Ok(Some(listener)),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn start_test_proxy(sandbox_id: &str) -> Result<Option<RunningEgressProxy>> {
        match EgressProxy::start(sandbox_id.into(), vec!["localhost".into()]).await {
            Ok(proxy) => Ok(Some(proxy)),
            Err(error)
                if error
                    .chain()
                    .any(|cause| cause.to_string().contains("Operation not permitted")) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    #[test]
    fn domain_targets_use_startup_resolved_ips() {
        let proxy = proxy_with(&[("pypi.org", &[IpAddr::V4(Ipv4Addr::new(151, 101, 0, 223))])]);

        let addr = proxy.resolve_target(&TargetAddr::Domain("pypi.org".into(), 443));
        assert_eq!(addr, Some(SocketAddr::from(([151, 101, 0, 223], 443))));
    }

    #[test]
    fn direct_ip_targets_must_be_in_allowlist() {
        let proxy = proxy_with(&[("pypi.org", &[IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))])]);

        assert_eq!(
            proxy.resolve_target(&TargetAddr::Ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 443)),
            Some(SocketAddr::from(([1, 1, 1, 1], 443)))
        );
        assert_eq!(
            proxy.resolve_target(&TargetAddr::Ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 443)),
            None
        );
    }

    #[test]
    fn unresolved_domain_is_denied() {
        let proxy = proxy_with(&[]);
        assert_eq!(
            proxy.resolve_target(&TargetAddr::Domain("pypi.org".into(), 443)),
            None
        );
    }

    #[tokio::test]
    async fn socks5_connects_to_allowlisted_domain() {
        let Some(upstream) = bind_test_listener().await.unwrap() else {
            return;
        };
        let upstream_addr = upstream.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = upstream.accept().await.unwrap();
            let mut buf = [0u8; 4];
            stream.read_exact(&mut buf).await.unwrap();
            assert_eq!(&buf, b"ping");
            stream.write_all(b"pong").await.unwrap();
        });

        let Some(proxy) = start_test_proxy("sandbox-1").await.unwrap() else {
            return;
        };
        let mut client = TcpStream::connect(SocketAddr::from((Ipv4Addr::LOCALHOST, proxy.port())))
            .await
            .unwrap();

        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0u8; 2];
        client.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [5, 0]);

        client
            .write_all(&[
                5, 1, 0, 3, 9, b'l', b'o', b'c', b'a', b'l', b'h', b'o', b's', b't',
            ])
            .await
            .unwrap();
        client
            .write_all(&upstream_addr.port().to_be_bytes())
            .await
            .unwrap();

        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[1], REPLY_SUCCEEDED);

        client.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        client.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"pong");

        proxy.abort();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn socks5_rejects_non_allowlisted_domain() {
        let Some(proxy) = start_test_proxy("sandbox-2").await.unwrap() else {
            return;
        };
        let mut client = TcpStream::connect(SocketAddr::from((Ipv4Addr::LOCALHOST, proxy.port())))
            .await
            .unwrap();

        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0u8; 2];
        client.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [5, 0]);

        client
            .write_all(&[
                5, 1, 0, 3, 11, b'e', b'x', b'a', b'm', b'p', b'l', b'e', b'.', b'c', b'o', b'm',
            ])
            .await
            .unwrap();
        client.write_all(&443u16.to_be_bytes()).await.unwrap();

        let mut reply = [0u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[1], REPLY_CONNECTION_NOT_ALLOWED);

        proxy.abort();
    }

    #[tokio::test]
    async fn http_connects_to_allowlisted_domain() {
        let Some(upstream) = bind_test_listener().await.unwrap() else {
            return;
        };
        let upstream_addr = upstream.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = upstream.accept().await.unwrap();
            let mut buf = [0u8; 4];
            stream.read_exact(&mut buf).await.unwrap();
            assert_eq!(&buf, b"ping");
            stream.write_all(b"pong").await.unwrap();
        });

        let Some(proxy) = start_test_proxy("sandbox-http").await.unwrap() else {
            return;
        };
        let mut client = TcpStream::connect(SocketAddr::from((Ipv4Addr::LOCALHOST, proxy.port())))
            .await
            .unwrap();

        client
            .write_all(
                format!(
                    "CONNECT localhost:{} HTTP/1.1\r\nHost: localhost\r\n\r\n",
                    upstream_addr.port()
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        let mut response = [0u8; 39];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"HTTP/1.1 200 Connection established\r\n\r\n");

        client.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        client.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"pong");

        proxy.abort();
        server.await.unwrap();
    }
}
