# Docker Backend

The `docker` backend is the default container runtime for AgentSandbox.

## Requirements

- Docker daemon reachable from AgentSandbox
- Unix socket available, usually `/var/run/docker.sock`

## Daemon Configuration

```toml
[backends]
enabled = ["docker"]

[backends.docker]
socket = "/var/run/docker.sock"
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
    backend: docker
```

## Supported Extensions

Fetch the live schema from the daemon:

```bash
curl -sS http://127.0.0.1:7847/v1/backends/docker/extensions-schema
```

Common fields under `spec.extensions.docker.hostConfig`:

- `capAdd`
- `capDrop`
- `securityOpt`
- `privileged`
- `shmSizeMb`
- `sysctls`
- `binds`
- `devices`
- `ulimits`

## Security Notes

- `networkMode` is rejected in the compile pipeline: use `spec.network.egress`
- `name` is reserved and managed internally
- `privileged: true` is accepted only as an explicit extension and is recorded as a security warning in the audit log
- secret values are injected into the runtime environment but never persisted in the daemon IR snapshot
