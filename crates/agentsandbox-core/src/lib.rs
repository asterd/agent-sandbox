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
    AuditLevel, EgressMode, EgressPolicy, EgressPolicyV1Beta1, Metadata, NetworkSpec,
    NetworkSpecV1Beta1, ObservabilitySpec, ResourceSpec, ResourceSpecV1Beta1, RuntimePreset,
    RuntimeSpec, RuntimeSpecV1Beta1, SandboxSpec, SandboxSpecBody, SandboxSpecBodyV1Beta1,
    SchedulingPriority, SchedulingSpec, SchedulingSpecV1Beta1, SecretRef, SecretSource,
    SpecV1Alpha1, SpecV1Beta1, StorageSpec, API_VERSION_V1ALPHA1, API_VERSION_V1BETA1,
};
