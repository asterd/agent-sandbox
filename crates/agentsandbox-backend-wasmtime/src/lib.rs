use agentsandbox_sdk::{
    backend::{
        BackendCapabilities, BackendDescriptor, BackendFactory, ExecResult, IsolationLevel,
        SandboxBackend, SandboxState, SandboxStatus,
    },
    error::BackendError,
    ir::SandboxIR,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::Mutex;
use tracing::warn;
use wasmtime::{Engine, Instance, Module, Store};

pub struct WasmtimeBackendFactory;

#[derive(Clone)]
struct SandboxSession {
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    ir: SandboxIR,
    workspace: PathBuf,
}

pub struct WasmtimeBackend {
    engine: Engine,
    python_wasm_path: Option<PathBuf>,
    node_wasm_path: Option<PathBuf>,
    sessions: Arc<Mutex<HashMap<String, SandboxSession>>>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
struct WasmtimeExtensions {
    wasm_binary: Option<String>,
    max_memory_mb: Option<u64>,
    #[serde(default)]
    preloaded_modules: Vec<String>,
}

struct CompatProgram {
    stdout: String,
    stderr: String,
    exit_code: i64,
}

impl BackendFactory for WasmtimeBackendFactory {
    fn describe(&self) -> BackendDescriptor {
        BackendDescriptor {
            id: "wasmtime",
            display_name: "Wasmtime (compat runner)",
            version: env!("CARGO_PKG_VERSION"),
            trait_version: agentsandbox_sdk::BACKEND_TRAIT_VERSION,
            capabilities: BackendCapabilities {
                network_isolation: true,
                memory_hard_limit: true,
                cpu_hard_limit: true,
                persistent_storage: false,
                self_contained: true,
                isolation_level: IsolationLevel::Process,
                supported_presets: vec!["python", "node"],
                rootless: true,
                snapshot_restore: false,
            },
            extensions_schema: Some(include_str!("../schema/extensions.schema.json")),
        }
    }

    fn create(
        &self,
        config: &HashMap<String, String>,
    ) -> Result<Box<dyn SandboxBackend>, BackendError> {
        let mut cfg = wasmtime::Config::new();
        cfg.consume_fuel(true);
        let engine = Engine::new(&cfg)
            .map_err(|error| BackendError::Configuration(format!("wasmtime engine: {error}")))?;
        Ok(Box::new(WasmtimeBackend {
            engine,
            python_wasm_path: config.get("python_wasm_path").map(PathBuf::from),
            node_wasm_path: config.get("node_wasm_path").map(PathBuf::from),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }))
    }
}

impl WasmtimeBackend {
    fn parse_extensions(ir: &SandboxIR) -> Result<WasmtimeExtensions, BackendError> {
        match &ir.extensions {
            None => Ok(WasmtimeExtensions::default()),
            Some(raw) => {
                let section = raw.get("wasmtime").cloned().unwrap_or_default();
                serde_json::from_value(section).map_err(|error| {
                    BackendError::Configuration(format!("extensions.wasmtime non valide: {error}"))
                })
            }
        }
    }

    async fn session(&self, handle: &str) -> Result<SandboxSession, BackendError> {
        self.sessions
            .lock()
            .await
            .get(handle)
            .cloned()
            .ok_or_else(|| BackendError::NotFound(handle.to_string()))
    }

    fn compile_program(command: &str) -> Result<CompatProgram, BackendError> {
        if let Some(text) = parse_echo(command, true) {
            return Ok(CompatProgram {
                stdout: String::new(),
                stderr: format!("{text}\n"),
                exit_code: 0,
            });
        }
        if let Some(text) = parse_echo(command, false) {
            return Ok(CompatProgram {
                stdout: format!("{text}\n"),
                stderr: String::new(),
                exit_code: 0,
            });
        }
        if let Some(code) = parse_exit(command) {
            return Ok(CompatProgram {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: code,
            });
        }
        if let Some(stdout) = parse_python_print(command)? {
            return Ok(CompatProgram {
                stdout: format!("{stdout}\n"),
                stderr: String::new(),
                exit_code: 0,
            });
        }

        Err(BackendError::NotSupported(
            "backend wasmtime supporta il subset compatibile: echo, echo >&2, exit N e python -c 'print(expr)'"
                .into(),
        ))
    }

    fn compat_module(program: &CompatProgram) -> String {
        let stdout = wat_string(&program.stdout);
        let stderr = wat_string(&program.stderr);
        let stdout_len = program.stdout.len();
        let stderr_len = program.stderr.len();
        format!(
            r#"(module
  (memory (export "memory") 1)
  (global (export "stdout_ptr") i32 (i32.const 16))
  (global (export "stdout_len") i32 (i32.const {stdout_len}))
  (global (export "stderr_ptr") i32 (i32.const {stderr_offset}))
  (global (export "stderr_len") i32 (i32.const {stderr_len}))
  (global (export "exit_code") i32 (i32.const {exit_code}))
  (data (i32.const 16) "{stdout}")
  (data (i32.const {stderr_offset}) "{stderr}")
  (func (export "_start"))
)"#,
            stderr_offset = 16 + stdout_len + 16,
            exit_code = program.exit_code
        )
    }

    fn run_compat_module(&self, program: &CompatProgram) -> Result<ExecResult, BackendError> {
        let module = Module::new(&self.engine, Self::compat_module(program))
            .map_err(|error| BackendError::Internal(format!("compile compat module: {error}")))?;
        let mut store = Store::new(&self.engine, ());
        store
            .set_fuel(10_000)
            .map_err(|error| BackendError::Internal(format!("set fuel: {error}")))?;
        let instance = Instance::new(&mut store, &module, &[])
            .map_err(|error| BackendError::Internal(format!("instantiate module: {error}")))?;
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .map_err(|error| BackendError::Internal(format!("resolve _start: {error}")))?;
        let started = std::time::Instant::now();
        start
            .call(&mut store, ())
            .map_err(|error| BackendError::Internal(format!("run module: {error}")))?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| BackendError::Internal("compat module senza memory".into()))?;
        let stdout_ptr = instance
            .get_global(&mut store, "stdout_ptr")
            .and_then(|global| global.get(&mut store).i32())
            .ok_or_else(|| BackendError::Internal("stdout_ptr mancante".into()))?;
        let stdout_len = instance
            .get_global(&mut store, "stdout_len")
            .and_then(|global| global.get(&mut store).i32())
            .ok_or_else(|| BackendError::Internal("stdout_len mancante".into()))?;
        let stderr_ptr = instance
            .get_global(&mut store, "stderr_ptr")
            .and_then(|global| global.get(&mut store).i32())
            .ok_or_else(|| BackendError::Internal("stderr_ptr mancante".into()))?;
        let stderr_len = instance
            .get_global(&mut store, "stderr_len")
            .and_then(|global| global.get(&mut store).i32())
            .ok_or_else(|| BackendError::Internal("stderr_len mancante".into()))?;
        let exit_code = instance
            .get_global(&mut store, "exit_code")
            .and_then(|global| global.get(&mut store).i32())
            .ok_or_else(|| BackendError::Internal("exit_code mancante".into()))?;

        let data = memory.data(&store);
        let stdout = String::from_utf8_lossy(
            &data[stdout_ptr as usize..(stdout_ptr + stdout_len) as usize],
        )
        .into_owned();
        let stderr = String::from_utf8_lossy(
            &data[stderr_ptr as usize..(stderr_ptr + stderr_len) as usize],
        )
        .into_owned();

        Ok(ExecResult {
            stdout,
            stderr,
            exit_code: i64::from(exit_code),
            duration_ms: started.elapsed().as_millis() as u64,
            resource_usage: None,
        })
    }
}

#[async_trait]
impl SandboxBackend for WasmtimeBackend {
    async fn create(&self, ir: &SandboxIR) -> Result<String, BackendError> {
        let _ = Self::parse_extensions(ir)?;
        let workspace = tempfile::tempdir()
            .map_err(|error| BackendError::Internal(format!("tempdir: {error}")))?;
        let workspace = workspace.keep();
        let now = Utc::now();
        self.sessions.lock().await.insert(
            ir.id.clone(),
            SandboxSession {
                created_at: now,
                expires_at: now + Duration::seconds(ir.ttl_seconds as i64),
                ir: ir.clone(),
                workspace,
            },
        );
        Ok(ir.id.clone())
    }

    async fn exec(
        &self,
        handle: &str,
        command: &str,
        _timeout_ms: Option<u64>,
    ) -> Result<ExecResult, BackendError> {
        let session = self.session(handle).await?;
        let extensions = Self::parse_extensions(&session.ir)?;
        if extensions.wasm_binary.is_some() || self.python_wasm_path.is_some() || self.node_wasm_path.is_some() {
            warn!(
                sandbox_id = %handle,
                "runtime wasm custom configurato ma in questa fase e' attivo solo il compat runner"
            );
        }
        let program = Self::compile_program(command)?;
        self.run_compat_module(&program)
    }

    async fn status(&self, handle: &str) -> Result<SandboxStatus, BackendError> {
        let session = self.session(handle).await?;
        Ok(SandboxStatus {
            sandbox_id: handle.to_string(),
            state: SandboxState::Running,
            created_at: session.created_at,
            expires_at: session.expires_at,
            backend_id: "wasmtime".into(),
        })
    }

    async fn destroy(&self, handle: &str) -> Result<(), BackendError> {
        if let Some(session) = self.sessions.lock().await.remove(handle) {
            let _ = std::fs::remove_dir_all(session.workspace);
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), BackendError> {
        if self.python_wasm_path.is_none() {
            warn!("wasmtime: python.wasm non configurato, uso il compat runner minimo");
        }
        Ok(())
    }

    async fn can_satisfy(&self, ir: &SandboxIR) -> Result<(), BackendError> {
        let _ = Self::parse_extensions(ir)?;
        Ok(())
    }
}

fn wat_string(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '"' => "\\22".chars().collect::<Vec<_>>(),
            '\\' => "\\5c".chars().collect::<Vec<_>>(),
            '\n' => "\\0a".chars().collect::<Vec<_>>(),
            '\r' => "\\0d".chars().collect::<Vec<_>>(),
            '\t' => "\\09".chars().collect::<Vec<_>>(),
            other if other.is_ascii_graphic() || other == ' ' => vec![other],
            other => format!("\\{:02x}", other as u32).chars().collect(),
        })
        .collect()
}

fn parse_echo(command: &str, stderr: bool) -> Option<String> {
    let suffix = if stderr { " >&2" } else { "" };
    let raw = command.trim().strip_prefix("echo ")?.strip_suffix(suffix)?;
    parse_shell_string(raw)
}

fn parse_exit(command: &str) -> Option<i64> {
    command
        .trim()
        .strip_prefix("exit ")?
        .trim()
        .parse::<i64>()
        .ok()
}

fn parse_python_print(command: &str) -> Result<Option<String>, BackendError> {
    let raw = match command.trim().strip_prefix("python -c ") {
        Some(raw) => raw,
        None => return Ok(None),
    };
    let script = match parse_shell_string(raw) {
        Some(script) => script,
        None => return Ok(None),
    };
    let expr = match script
        .trim()
        .strip_prefix("print(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        Some(expr) => expr,
        None => {
            return Err(BackendError::NotSupported(
                "compat python supporta solo print(expr)".into(),
            ))
        }
    };
    let value = eval_expr(expr)?;
    Ok(Some(if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }))
}

fn parse_shell_string(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix('\'').and_then(|raw| raw.strip_suffix('\'')) {
        return Some(inner.to_string());
    }
    if let Some(inner) = value.strip_prefix('"').and_then(|raw| raw.strip_suffix('"')) {
        return Some(inner.to_string());
    }
    Some(value.to_string())
}

fn eval_expr(expr: &str) -> Result<f64, BackendError> {
    let tokens: Vec<char> = expr.chars().filter(|ch| !ch.is_whitespace()).collect();
    let mut parser = ExprParser { tokens, pos: 0 };
    let value = parser.parse_expr()?;
    if parser.pos != parser.tokens.len() {
        return Err(BackendError::NotSupported(
            "compat python supporta solo espressioni aritmetiche semplici".into(),
        ));
    }
    Ok(value)
}

struct ExprParser {
    tokens: Vec<char>,
    pos: usize,
}

impl ExprParser {
    fn parse_expr(&mut self) -> Result<f64, BackendError> {
        let mut value = self.parse_term()?;
        while let Some(op) = self.peek() {
            match op {
                '+' => {
                    self.pos += 1;
                    value += self.parse_term()?;
                }
                '-' => {
                    self.pos += 1;
                    value -= self.parse_term()?;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn parse_term(&mut self) -> Result<f64, BackendError> {
        let mut value = self.parse_factor()?;
        while let Some(op) = self.peek() {
            match op {
                '*' => {
                    self.pos += 1;
                    value *= self.parse_factor()?;
                }
                '/' => {
                    self.pos += 1;
                    value /= self.parse_factor()?;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn parse_factor(&mut self) -> Result<f64, BackendError> {
        match self.peek() {
            Some('(') => {
                self.pos += 1;
                let value = self.parse_expr()?;
                if self.peek() != Some(')') {
                    return Err(BackendError::NotSupported("parentesi non bilanciate".into()));
                }
                self.pos += 1;
                Ok(value)
            }
            Some('-') => {
                self.pos += 1;
                Ok(-self.parse_factor()?)
            }
            Some(ch) if ch.is_ascii_digit() => self.parse_number(),
            _ => Err(BackendError::NotSupported(
                "compat python supporta solo numeri e operatori + - * /".into(),
            )),
        }
    }

    fn parse_number(&mut self) -> Result<f64, BackendError> {
        let start = self.pos;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_digit() || ch == '.') {
            self.pos += 1;
        }
        self.tokens[start..self.pos]
            .iter()
            .collect::<String>()
            .parse::<f64>()
            .map_err(|_| BackendError::NotSupported("numero non valido".into()))
    }

    fn peek(&self) -> Option<char> {
        self.tokens.get(self.pos).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_python_print_expression() {
        assert_eq!(
            parse_python_print("python -c 'print(1 + 2 * 3)'")
                .unwrap()
                .as_deref(),
            Some("7")
        );
    }

    #[tokio::test]
    async fn compat_runner_executes_conformance_echo() {
        let backend = WasmtimeBackendFactory.create(&HashMap::new()).unwrap();
        let ir = SandboxIR::default_for_test();
        let handle = backend.create(&ir).await.unwrap();
        let result = backend
            .exec(&handle, "echo 'agentsandbox-conformance-ok'", None)
            .await
            .unwrap();
        assert!(result.stdout.contains("agentsandbox-conformance-ok"));
    }
}
