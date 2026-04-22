# Changelog

## Unreleased

### Added

- Internal-service hardening for Scenario 1: explicit `limits`, `audit`, `security`, and tenant backend allowlists in daemon config.
- `GET /v1/runtime-info`, tenant usage inspection, upload/download/snapshot/restore HTTP endpoints, and NDJSON execution streaming.
- Concurrent quota enforcement via persisted tenant usage with reconciliation on startup.
- Internal operations artifacts: systemd unit, internal config example, tenant bootstrap script, runtime export script, smoke checklist, and troubleshooting docs.
- SDK additions for Python and TypeScript: `upload_file`/`uploadFile`, `download_file`/`downloadFile`, `snapshot`, and `exec_stream`/`execStream`.
- Example `05-file-stream-demo` covering upload, stream, download, and destroy.
- `sandbox.ai/v1` support in the core compile pipeline via `compile_any()` and runtime `apiVersion` detection.
- Official JSON Schema committed at `spec/sandbox.v1.schema.json`.
- Structured daemon validation errors with per-field details in `error.details.validationErrors`.
- Release-readiness documentation for the public spec, HTTP API, Docker, Podman, gVisor, and Firecracker status.
- `BACKEND_GUIDE.md` with the minimal contract for adding a backend.
- `scripts/release_check.sh` to run the Phase K repository-side gates in one place.
- Automatic audit-log warning when backend extensions request `privileged=true`.
- Regression tests that keep `secret_env` and backend native handles out of persisted or public responses.
- Security dependency refresh: `sqlx` updated to `0.8.6` and `wasmtime` to `36.0.7`.

### Changed

- `POST /v1/sandboxes` now accepts only `sandbox.ai/v1`.
- `v1` documentation now reflects that backend extensions are sent through `spec.extensions`, not a private HTTP header.
- Release notes and docs now state explicitly that Firecracker is reserved but not yet shipped in this repository snapshot.
- Docker conformance now skips cleanly when the local test image is missing instead of failing the release gate on an environmental precondition.

## 0.1.0

- Hardened the public `sandbox.ai/v1` contract:
  strict unknown-field rejection, explicit `kind=Sandbox` validation, and a committed JSON Schema in `spec/`.
- Made daemon inspect/list reflect backend runtime state instead of only replaying SQLite rows.
- Extended both SDKs with additive support for `workingDir`, `diskMb`, `scheduling.preferWarm`, and file-backed secrets.
- Added daemon tests for runtime-state refresh and TTL reaping, plus a small `tests/e2e/` smoke suite for the Python and TypeScript SDKs.
- Clarified current `v1` egress behavior:
  filtered egress remains image-dependent today and the planned stable replacement is the proxy L4 path documented in `ROADMAP_STABLE.md` FASE C.
