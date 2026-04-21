use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExecRequest {
    pub command: String,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExecResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i64,
    pub duration_ms: u64,
}

pub async fn run_server(socket_path: &str) -> std::io::Result<()> {
    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            let _ = handle_connection(stream).await;
        });
    }
}

pub async fn handle_connection(stream: UnixStream) -> std::io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let response = match serde_json::from_str::<ExecRequest>(&line) {
            Ok(request) => execute_request(request).await,
            Err(error) => ExecResponse {
                stdout: String::new(),
                stderr: format!("invalid request: {error}"),
                exit_code: -1,
                duration_ms: 0,
            },
        };
        let payload = serde_json::to_string(&response).expect("response serializable");
        writer.write_all(payload.as_bytes()).await?;
        writer.write_all(b"\n").await?;
    }

    Ok(())
}

pub async fn execute_request(request: ExecRequest) -> ExecResponse {
    let start = std::time::Instant::now();
    let timeout_ms = request.timeout_ms.unwrap_or(30_000);
    let output = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        tokio::process::Command::new("sh")
            .arg("-lc")
            .arg(&request.command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await;

    match output {
        Ok(Ok(result)) => ExecResponse {
            stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
            exit_code: result.status.code().unwrap_or(-1) as i64,
            duration_ms: start.elapsed().as_millis() as u64,
        },
        Ok(Err(error)) => ExecResponse {
            stdout: String::new(),
            stderr: error.to_string(),
            exit_code: -1,
            duration_ms: start.elapsed().as_millis() as u64,
        },
        Err(_) => ExecResponse {
            stdout: String::new(),
            stderr: format!("timeout dopo {timeout_ms}ms"),
            exit_code: -1,
            duration_ms: timeout_ms,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execute_request_captures_stdout() {
        let response = execute_request(ExecRequest {
            command: "echo hello".into(),
            timeout_ms: Some(1_000),
        })
        .await;
        assert_eq!(response.exit_code, 0);
        assert!(response.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn execute_request_times_out() {
        let response = execute_request(ExecRequest {
            command: "sleep 1".into(),
            timeout_ms: Some(10),
        })
        .await;
        assert_eq!(response.exit_code, -1);
        assert!(response.stderr.contains("timeout"));
    }
}
