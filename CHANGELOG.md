# Changelog

## 0.1.0

- Hardened the public `sandbox.ai/v1alpha1` contract:
  strict unknown-field rejection, explicit `kind=Sandbox` validation, and a committed JSON Schema in `spec/`.
- Made daemon inspect/list reflect backend runtime state instead of only replaying SQLite rows.
- Extended both SDKs with additive support for `workingDir`, `diskMb`, `scheduling.preferWarm`, and file-backed secrets.
- Added daemon tests for runtime-state refresh and TTL reaping, plus a small `tests/e2e/` smoke suite for the Python and TypeScript SDKs.
- Clarified current v1alpha1 egress behavior:
  filtered egress remains image-dependent today and the planned stable replacement is the proxy L4 path documented in `ROADMAP_STABLE.md` FASE C.
