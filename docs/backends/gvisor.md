# gVisor Backend

The `gvisor` backend runs Linux containers through Docker using the `runsc` runtime.

## Requirements

- Linux host
- Docker daemon reachable from AgentSandbox
- gVisor installed and registered as a Docker runtime, usually `runsc`

Example daemon configuration:

```toml
[backends]
enabled = ["docker", "gvisor"]

[backends.gvisor]
socket = "/var/run/docker.sock"
runtime = "runsc"
```

Example spec routing:

```yaml
apiVersion: sandbox.ai/v1
kind: Sandbox
metadata: {}
spec:
  runtime:
    preset: python
  scheduling:
    backend: gvisor
```

## Unsupported Systems

- macOS hosts are not supported for gVisor execution
- Windows hosts are not supported for gVisor execution
- Linux hosts without a Docker runtime named `runsc` (or the configured alternative) are not supported
- Rootless operation is not supported in the current backend

## Notes

- The backend reuses the Docker execution path and only overrides the container runtime.
- If the Docker runtime is missing, `health_check()` returns an explicit `Unavailable` error with the gVisor install URL.
- Backend-specific extensions are supported through `spec.extensions.gvisor`.
- The current backend implementation supports `extensions.gvisor.network` with `sandbox`, `host`, and `none`.
- gVisor platform selection is intentionally not exposed yet because the current Docker `runsc` integration only guarantees a backend-level runtime choice via `backends.gvisor.runtime`.
