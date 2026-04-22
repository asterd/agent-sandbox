# Backend Guide

This guide is the minimum contract for adding a new backend.

## 1. Create the crate

- add `crates/agentsandbox-backend-<name>`
- depend only on `agentsandbox-sdk` plus runtime-specific crates
- avoid daemon, SQLx, and HTTP-layer dependencies

## 2. Implement the public SDK traits

- implement `BackendFactory` to expose descriptor, capabilities, and config parsing
- implement `SandboxBackend` for `create`, `exec`, `status`, `destroy`, `health_check`
- return `BackendError::NotSupported` for optional features you do not implement

## 3. Provide an extension schema

- commit `schema/extensions.schema.json`
- reject unknown fields
- reserve any runtime-controlled fields instead of letting callers override them

## 4. Ship the plugin binary

- add `src/main.rs` that calls `agentsandbox_sdk::plugin::serve_plugin(&YourBackendFactory)`
- ensure the compiled executable name starts with `agentsandbox-backend-`
- keep backend-specific config stringly-typed so the daemon can pass it through without linking your crate
- expose the schema through `GET /v1/backends/:id/extensions-schema`

## 5. Pass conformance

- add `tests/conformance.rs`
- run the shared suite from `agentsandbox-conformance`
- do not enable the backend by default until the suite passes reliably

## 6. Document the backend

- add `docs/backends/<name>.md`
- document requirements, config, example routing, and security constraints

The daemon discovers plugins from configured search directories plus `PATH`. If your backend requires daemon code changes to be loadable, the boundary is wrong.
