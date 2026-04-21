//! Core types, spec parser and IR for AgentSandbox.
//!
//! This crate defines the public sandbox specification (`spec`), the internal
//! intermediate representation (`ir`) that backend adapters consume, the
//! `compile` pipeline that turns one into the other, and the [`SandboxAdapter`]
//! contract every backend implements. Nothing here is backend-specific:
//! Docker, Firecracker or any future backend depends on `ir::SandboxIR` and
//! never on the raw spec.

pub mod adapter;
pub mod compile;
pub mod ir;
pub mod schema;
pub mod spec;

#[cfg(feature = "conformance")]
pub mod conformance;

pub use adapter::{AdapterError, ExecResult, SandboxAdapter, SandboxInfo, SandboxStatus};
pub use compile::{
    compile, compile_any, compile_value, detect_version, CompileError, SpecVersion, ValidationIssue,
};
pub use ir::SandboxIR;
pub use spec::{
    AuditLevel, EgressMode, EgressPolicy, Metadata, NetworkSpec, ObservabilitySpec, ResourceSpec,
    RuntimePreset, RuntimeSpec, SandboxSpec, SandboxSpecBody, SchedulingPriority, SchedulingSpec,
    SecretRef, SecretSource, StorageSpec, API_VERSION_V1,
};
