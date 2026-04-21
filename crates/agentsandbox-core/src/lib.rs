//! Spec parser and compile pipeline for AgentSandbox.
//!
//! This crate owns the public sandbox specification (`spec`) and the compile
//! pipeline that turns it into the backend-facing IR exposed by
//! `agentsandbox-sdk`. Backend plugins depend on the SDK, not on this crate.

pub mod compile;
pub mod schema;
pub mod spec;

pub mod backend {
    pub use agentsandbox_sdk::backend::*;
}

pub mod ir {
    pub use agentsandbox_sdk::ir::*;
}

pub use compile::{
    compile, compile_any, compile_value, detect_version, CompileError, SpecVersion, ValidationIssue,
};
pub use ir::SandboxIR;
pub use spec::{
    EgressPolicy, Metadata, NetworkSpec, ObservabilitySpec, ResourceSpec, RuntimePreset,
    RuntimeSpec, SandboxSpec, SandboxSpecBody, SchedulingSpec, SecretRef, SecretSource,
    StorageSpec, API_VERSION_V1,
};
