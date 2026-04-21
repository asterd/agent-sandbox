//! Integration tests for the egress allowlist enforcement.
//!
//! These tests require a real Docker daemon and network access to `alpine:3.20`
//! plus the Alpine package mirrors (the happy-path test needs to `apk add
//! iptables` inside the guest). They are ignored by default — run them with:
//!
//!     cargo test -p agentsandbox-docker --test egress -- --ignored --test-threads=1
//!
//! The fail-closed test does not need network access inside the guest, only
//! the daemon.

use std::time::Duration;

use agentsandbox_docker::egress::apply_egress_rules;
use bollard::container::{
    Config, CreateContainerOptions, LogOutput, RemoveContainerOptions, StartContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::HostConfig;
use bollard::Docker;
use futures::StreamExt;

const ALPINE_IMAGE: &str = "alpine:3.20";
const FORCE_INTEGRATION_ENV: &str = "AGENTSANDBOX_INTEGRATION";

struct ExecOutcome {
    exit_code: i64,
    stdout: String,
    stderr: String,
}

async fn client_or_skip() -> Option<Docker> {
    let docker = Docker::connect_with_local_defaults().ok()?;
    docker.ping().await.ok()?;
    Some(docker)
}

async fn client_or_fail() -> Docker {
    match client_or_skip().await {
        Some(c) => c,
        None => panic!(
            "Docker non disponibile. Imposta {FORCE_INTEGRATION_ENV}=1 per forzare \
             il fallimento del test, altrimenti rimuovi il forcing."
        ),
    }
}

fn integration_forced() -> bool {
    std::env::var(FORCE_INTEGRATION_ENV).as_deref() == Ok("1")
}

async fn docker_or_skip() -> Option<Docker> {
    if integration_forced() {
        Some(client_or_fail().await)
    } else {
        let c = client_or_skip().await;
        if c.is_none() {
            eprintln!("skip: Docker non disponibile");
        }
        c
    }
}

async fn create_container(
    docker: &Docker,
    name: &str,
    cap_add: Option<Vec<String>>,
) -> anyhow::Result<()> {
    let host_config = HostConfig {
        network_mode: Some("bridge".to_string()),
        cap_add,
        auto_remove: Some(false),
        ..Default::default()
    };

    let config = Config {
        image: Some(ALPINE_IMAGE.to_string()),
        cmd: Some(vec!["sleep".to_string(), "120".to_string()]),
        host_config: Some(host_config),
        ..Default::default()
    };

    docker
        .create_container(
            Some(CreateContainerOptions {
                name,
                platform: None,
            }),
            config,
        )
        .await?;
    docker
        .start_container(name, None::<StartContainerOptions<String>>)
        .await?;
    Ok(())
}

async fn remove_container(docker: &Docker, name: &str) {
    let _ = docker
        .remove_container(
            name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;
}

async fn exec(docker: &Docker, name: &str, cmd: &str) -> anyhow::Result<ExecOutcome> {
    let exec = docker
        .create_exec(
            name,
            CreateExecOptions {
                cmd: Some(vec!["sh".to_string(), "-c".to_string(), cmd.to_string()]),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await?;

    let mut stdout = String::new();
    let mut stderr = String::new();

    if let StartExecResults::Attached { mut output, .. } =
        docker.start_exec(&exec.id, None).await?
    {
        while let Some(chunk) = output.next().await {
            match chunk? {
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

    let inspect = docker.inspect_exec(&exec.id).await?;
    Ok(ExecOutcome {
        exit_code: inspect.exit_code.unwrap_or(-1),
        stdout,
        stderr,
    })
}

/// Happy path: allowlist esplicita.
///
/// * installa `iptables` nel guest via apk,
/// * applica la policy per `1.1.1.1`,
/// * verifica che `1.1.1.1` passi (echo request accettato)
///   e `8.8.8.8` venga bloccato.
///
/// Usiamo `1.1.1.1` come stringa: `lookup_host` la accetta come literal IP e
/// restituisce se stesso, quindi la policy contiene esattamente quell'IP e il
/// test e' deterministico (nessuna dipendenza da DNS esterni).
#[tokio::test]
#[ignore = "richiede Docker + rete verso i mirror Alpine"]
async fn apply_egress_rules_blocks_non_allowlisted_ips() {
    let Some(docker) = docker_or_skip().await else {
        return;
    };
    let name = "agentsandbox-egress-allow-it";
    remove_container(&docker, name).await;
    create_container(&docker, name, Some(vec!["NET_ADMIN".into()]))
        .await
        .expect("create container");

    // Attendere che la rete sia su prima di fare apk.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let install = exec(&docker, name, "apk add --no-cache iptables")
        .await
        .expect("apk add");
    if install.exit_code != 0 {
        // Ambienti con MITM TLS, proxy corporate o senza uscita verso i
        // mirror Alpine non possono installare iptables nel guest: il test
        // di enforcement end-to-end non e' eseguibile, ma non e' un bug
        // del sandbox — skip esplicito invece di falso negativo.
        eprintln!(
            "skip: impossibile installare iptables nel guest (apk add exit \
             {}). stderr={}",
            install.exit_code, install.stderr
        );
        remove_container(&docker, name).await;
        return;
    }

    apply_egress_rules(&docker, name, &["1.1.1.1".into()])
        .await
        .expect("apply_egress_rules deve riuscire");

    // `ping` con un singolo pacchetto e timeout 2s. 1.1.1.1 e' nell'allowlist.
    let allowed = exec(&docker, name, "ping -c 1 -W 2 1.1.1.1").await.unwrap();
    assert_eq!(
        allowed.exit_code, 0,
        "1.1.1.1 deve essere raggiungibile; stderr={}",
        allowed.stderr
    );

    // 8.8.8.8 NON e' nell'allowlist: deve fallire (exit != 0).
    let blocked = exec(&docker, name, "ping -c 1 -W 2 8.8.8.8").await.unwrap();
    assert_ne!(
        blocked.exit_code, 0,
        "8.8.8.8 doveva essere bloccato; stdout={}, stderr={}",
        blocked.stdout, blocked.stderr
    );

    remove_container(&docker, name).await;
}

/// Fail-closed: se il guest non ha `iptables`, `apply_egress_rules` deve
/// ritornare errore. Alpine 3.20 senza `apk add` non include iptables.
#[tokio::test]
#[ignore = "richiede Docker"]
async fn apply_egress_rules_fails_when_iptables_missing() {
    let Some(docker) = docker_or_skip().await else {
        return;
    };
    let name = "agentsandbox-egress-failclosed-it";
    remove_container(&docker, name).await;
    create_container(&docker, name, Some(vec!["NET_ADMIN".into()]))
        .await
        .expect("create container");

    let result = apply_egress_rules(&docker, name, &["1.1.1.1".into()]).await;
    assert!(
        result.is_err(),
        "apply_egress_rules doveva fallire perche' iptables manca"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("iptables"),
        "l'errore deve menzionare iptables; msg={msg}"
    );

    remove_container(&docker, name).await;
}

/// Guard lato libreria: se tutti gli host di `allow` sono irresolvibili,
/// l'apply deve rifiutarsi invece di installare un `OUTPUT DROP` silenzioso.
///
/// Non tocchiamo iptables dentro al container: l'errore deve arrivare prima
/// di `ensure_iptables_available`. Creiamo comunque un container valido per
/// avere un `container_name` reale a cui bollard possa fare `create_exec`
/// senza 404 spuri.
#[tokio::test]
#[ignore = "richiede Docker"]
async fn apply_egress_rules_refuses_empty_allowlist_after_dns_failure() {
    let Some(docker) = docker_or_skip().await else {
        return;
    };
    let name = "agentsandbox-egress-emptyresolve-it";
    remove_container(&docker, name).await;
    create_container(&docker, name, Some(vec!["NET_ADMIN".into()]))
        .await
        .expect("create container");

    // `.invalid` e' un TLD riservato (RFC 6761): la risoluzione fallisce
    // deterministicamente su qualsiasi resolver conforme.
    let result = apply_egress_rules(
        &docker,
        name,
        &["host-che-non-esiste.invalid".into()],
    )
    .await;
    assert!(
        result.is_err(),
        "apply_egress_rules doveva fallire per allowlist vuota post-DNS"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("nessun hostname"),
        "errore inatteso: {msg}"
    );

    remove_container(&docker, name).await;
}
