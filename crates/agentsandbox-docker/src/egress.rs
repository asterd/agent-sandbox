//! Egress allowlist enforcement for Docker sandboxes.
//!
//! v1alpha1 keeps the model intentionally simple:
//! - resolve hostnames once at sandbox startup
//! - translate the resolved IPs into container-local `iptables` rules
//! - never update the rules afterwards
//!
//! This is not perfect, but it is materially safer than silently giving a
//! sandbox unrestricted outbound access when the spec asked for an allowlist.

use anyhow::{anyhow, bail, Context, Result};
use bollard::container::LogOutput;
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::Docker;
use futures::StreamExt;
use std::collections::BTreeSet;
use tokio::net::lookup_host;

/// Minimal operational contract required to apply egress rules inside a guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedEgress {
    pub allowed_ips: Vec<String>,
}

impl ResolvedEgress {
    pub fn is_empty(&self) -> bool {
        self.allowed_ips.is_empty()
    }
}

pub async fn resolve_allow_hosts(allow: &[String]) -> ResolvedEgress {
    let mut allowed_ips = BTreeSet::new();

    for host in allow {
        // La porta 443 e' un valore arbitrario: `lookup_host` richiede
        // "host:port", ma a noi interessa solo l'IP. Qualsiasi porta valida
        // produce lo stesso A/AAAA record.
        match lookup_host((host.as_str(), 443)).await {
            Ok(addrs) => {
                for addr in addrs {
                    allowed_ips.insert(addr.ip().to_string());
                }
            }
            Err(err) => {
                tracing::warn!(
                    host = %host,
                    error = %err,
                    "impossibile risolvere host egress; host ignorato"
                );
            }
        }
    }

    ResolvedEgress {
        allowed_ips: allowed_ips.into_iter().collect(),
    }
}

pub fn build_iptables_script(allowed_ips: &[String]) -> String {
    let mut script = String::from(
        "set -eu\n\
         iptables -P OUTPUT DROP\n\
         iptables -A OUTPUT -o lo -j ACCEPT\n\
         iptables -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT\n",
    );

    for ip in allowed_ips {
        script.push_str("iptables -A OUTPUT -d ");
        script.push_str(ip);
        script.push_str(" -j ACCEPT\n");
    }

    script
}

pub async fn apply_egress_rules(
    client: &Docker,
    container_name: &str,
    allow: &[String],
) -> Result<()> {
    let resolved = resolve_allow_hosts(allow).await;
    tracing::info!(
        container = %container_name,
        allowed_ips = ?resolved.allowed_ips,
        "applicazione regole egress v1alpha1"
    );

    // Se l'utente ha dichiarato una allowlist ma il DNS ha fallito per
    // tutti gli host, installare comunque `OUTPUT DROP` equivarrebbe a un
    // silent fallback a sandbox offline con `bridge + NET_ADMIN` — esattamente
    // il "nessun fallback silenzioso" che la spec v1alpha1 vieta. Fail-closed.
    if resolved.is_empty() && !allow.is_empty() {
        bail!(
            "nessun hostname di network.egress.allow e' stato risolto; \
             rifiuto di installare una policy che bloccherebbe tutto il \
             traffico invece di applicare l'allowlist richiesta"
        );
    }

    ensure_iptables_available(client, container_name).await?;

    let script = build_iptables_script(&resolved.allowed_ips);
    run_exec(
        client,
        container_name,
        vec!["sh".to_string(), "-c".to_string(), script],
    )
    .await
    .context("applicazione regole iptables fallita")?;

    Ok(())
}

async fn ensure_iptables_available(client: &Docker, container_name: &str) -> Result<()> {
    run_exec(
        client,
        container_name,
        vec![
            "sh".to_string(),
            "-c".to_string(),
            "command -v iptables >/dev/null 2>&1".to_string(),
        ],
    )
    .await
    .map_err(|err| {
        anyhow!(
            "runtime image non supporta `iptables`; impossibile applicare \
             network.egress in modo sicuro: {err}"
        )
    })?;

    Ok(())
}

async fn run_exec(client: &Docker, container_name: &str, cmd: Vec<String>) -> Result<()> {
    let exec = client
        .create_exec(
            container_name,
            CreateExecOptions {
                cmd: Some(cmd),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await
        .context("create_exec")?;

    let mut stdout = String::new();
    let mut stderr = String::new();

    if let StartExecResults::Attached { mut output, .. } = client
        .start_exec(&exec.id, None)
        .await
        .context("start_exec")?
    {
        while let Some(chunk) = output.next().await {
            match chunk.context("stream exec")? {
                LogOutput::StdOut { message } => {
                    stdout.push_str(&String::from_utf8_lossy(&message));
                }
                LogOutput::StdErr { message } => {
                    stderr.push_str(&String::from_utf8_lossy(&message));
                }
                _ => {}
            }
        }
    }

    let inspect = client
        .inspect_exec(&exec.id)
        .await
        .context("inspect_exec")?;
    let exit_code = inspect.exit_code.unwrap_or(-1);

    if exit_code != 0 {
        let output = stderr.trim();
        let fallback = stdout.trim();
        let message = if !output.is_empty() {
            output
        } else if !fallback.is_empty() {
            fallback
        } else {
            "exec terminata senza output"
        };
        bail!("exit code {exit_code}: {message}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_iptables_script_allows_loopback_established_and_ips() {
        let script = build_iptables_script(&["1.1.1.1".into(), "8.8.8.8".into()]);
        assert!(script.contains("iptables -P OUTPUT DROP"));
        assert!(script.contains("iptables -A OUTPUT -o lo -j ACCEPT"));
        assert!(script.contains("ESTABLISHED,RELATED"));
        assert!(script.contains("iptables -A OUTPUT -d 1.1.1.1 -j ACCEPT"));
        assert!(script.contains("iptables -A OUTPUT -d 8.8.8.8 -j ACCEPT"));
    }

    #[test]
    fn build_iptables_script_without_ips_only_allows_loopback_and_established() {
        let script = build_iptables_script(&[]);
        assert!(script.contains("iptables -P OUTPUT DROP"));
        assert!(!script.contains(" -d "));
    }

    #[test]
    fn resolved_egress_is_empty_reflects_allowed_ips() {
        assert!(ResolvedEgress { allowed_ips: vec![] }.is_empty());
        assert!(!ResolvedEgress {
            allowed_ips: vec!["1.1.1.1".into()],
        }
        .is_empty());
    }
}
