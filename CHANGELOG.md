# Changelog

## Unreleased

### Added

- `sandbox.ai/v1beta1` support in the core compile pipeline via `compile_any()` and runtime `apiVersion` detection.
- Official JSON Schema for `v1beta1` committed at `spec/sandbox.v1beta1.schema.json`.
- Structured daemon validation errors with per-field details in `error.details.validationErrors`.

### Changed

- `POST /v1/sandboxes` now accepts both `sandbox.ai/v1alpha1` and `sandbox.ai/v1beta1`.
- `v1beta1` adds optional fields without breaking `v1alpha1`:
- `spec.runtime.version`
- `spec.resources.timeoutMs`
- `spec.network.egress.mode`
- `spec.scheduling.backend`
- `spec.scheduling.priority`
- `spec.storage.volumes`
- `spec.observability.auditLevel`
- `spec.observability.metricsEnabled`

### Compatibility

- No `v1alpha1` field was removed or renamed.
- Existing `v1alpha1` requests continue to compile to the same IR defaults.

## 0.1.0

- Hardened the public `sandbox.ai/v1alpha1` contract:
  strict unknown-field rejection, explicit `kind=Sandbox` validation, and a committed JSON Schema in `spec/`.
- Made daemon inspect/list reflect backend runtime state instead of only replaying SQLite rows.
- Extended both SDKs with additive support for `workingDir`, `diskMb`, `scheduling.preferWarm`, and file-backed secrets.
- Added daemon tests for runtime-state refresh and TTL reaping, plus a small `tests/e2e/` smoke suite for the Python and TypeScript SDKs.
- Clarified current v1alpha1 egress behavior:
  filtered egress remains image-dependent today and the planned stable replacement is the proxy L4 path documented in `ROADMAP_STABLE.md` FASE C.
