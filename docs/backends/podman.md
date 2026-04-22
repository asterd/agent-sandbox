# Podman Backend

The `podman` backend reuses the Docker execution path against a Podman socket and reports itself as rootless by default.

## Requirements

- Podman socket reachable from AgentSandbox
- Rootless socket path such as `${XDG_RUNTIME_DIR}/podman/podman.sock` or `/run/user/<uid>/podman/podman.sock`

## Daemon Configuration

```toml
[backends]
enabled = ["podman"]

[backends.podman]
socket = "/run/user/1000/podman/podman.sock"
```

## Example Spec Routing

```yaml
apiVersion: sandbox.ai/v1
kind: Sandbox
metadata: {}
spec:
  runtime:
    preset: python
  scheduling:
    backend: podman
```

## Extensions

`podman` currently mirrors the Docker extension schema. Query it directly:

```bash
curl -sS http://127.0.0.1:7847/v1/backends/podman/extensions-schema
```

## Notes

- `health_check()` rewrites socket errors to mention Podman explicitly
- `status()` rewrites `backend_id` to `podman`
- `networkMode` remains forbidden in extensions: use `spec.network.egress`
