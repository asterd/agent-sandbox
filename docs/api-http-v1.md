# HTTP API v1

Current daemon base URL:

```text
http://127.0.0.1:7847
```

## Content Types

- `POST /v1/sandboxes` accepts `application/json`, `application/yaml`, and `text/yaml`
- all successful responses are JSON except `DELETE /v1/sandboxes/:id` (`204 No Content`)
- all errors use the same JSON envelope

## Authentication

- single-user mode: create is open, mutating sandbox operations require the lease token
- API-key mode: `X-API-Key` is required for every request and is mapped to a tenant
- `POST /v1/sandboxes` returns `lease_token`
- `POST /v1/sandboxes/:id/exec` requires `X-Lease-Token`
- `POST /v1/sandboxes/:id/files` requires `X-Lease-Token`
- `GET /v1/sandboxes/:id/files/*path` requires `X-Lease-Token`
- `POST /v1/sandboxes/:id/snapshot` requires `X-Lease-Token`
- `DELETE /v1/sandboxes/:id` requires `X-Lease-Token` when the sandbox exists

## Error Envelope

```json
{
  "error": {
    "code": "SPEC_INVALID",
    "message": "spec V1 non valida",
    "details": {}
  }
}
```

## Endpoints

### `GET /v1/health`

```bash
curl -sS http://127.0.0.1:7847/v1/health
```

Example response:

```json
{
  "status": "ok",
  "backend": "docker",
  "backends": ["docker", "podman"]
}
```

### `GET /metrics`

```bash
curl -sS http://127.0.0.1:7847/metrics
```

Example response:

```text
# HELP agentsandbox_sandboxes_created_total Total created sandboxes
# TYPE agentsandbox_sandboxes_created_total counter
agentsandbox_sandboxes_created_total 3
```

### `GET /v1/backends`

```bash
curl -sS http://127.0.0.1:7847/v1/backends
```

Example response:

```json
{
  "items": [
    {
      "id": "docker",
      "display_name": "Docker",
      "version": "0.1.0",
      "trait_version": "1",
      "capabilities": {
        "network_isolation": true,
        "memory_hard_limit": true,
        "cpu_hard_limit": true,
        "persistent_storage": false,
        "self_contained": false,
        "isolation_level": "Container",
        "supported_presets": ["python", "node", "rust", "shell"],
        "rootless": false,
        "snapshot_restore": false
      },
      "extensions_supported": true
    }
  ]
}
```

### `GET /v1/runtime-info`

Operational metadata for service deployments.

```bash
curl -sS http://127.0.0.1:7847/v1/runtime-info
```

Example response:

```json
{
  "daemon_version": "0.1.0",
  "config_profile": "internal",
  "available_backends": ["docker"],
  "auth_mode": "api_key",
  "db_path": "sqlite:///var/lib/agentsandbox/agentsandbox.db",
  "config_path": "/etc/agentsandbox/agentsandbox.internal.toml",
  "limits": {
    "max_ttl_seconds": 3600,
    "default_timeout_ms": 30000,
    "max_concurrent_sandboxes": 50,
    "max_file_bytes": 1048576
  }
}
```

### `GET /v1/backends/:id/extensions-schema`

```bash
curl -sS http://127.0.0.1:7847/v1/backends/docker/extensions-schema
```

Example response:

```json
{
  "title": "Docker Backend Extensions",
  "type": "object"
}
```

### `POST /v1/sandboxes`

Create a sandbox from a `sandbox.ai/v1` spec.

Minimal JSON request:

```bash
curl -sS \
  -H 'Content-Type: application/json' \
  -d '{
    "apiVersion": "sandbox.ai/v1",
    "kind": "Sandbox",
    "metadata": {},
    "spec": {
      "runtime": { "preset": "python" },
      "ttlSeconds": 300
    }
  }' \
  http://127.0.0.1:7847/v1/sandboxes
```

YAML request:

```bash
curl -sS \
  -H 'Content-Type: application/yaml' \
  --data-binary @- \
  http://127.0.0.1:7847/v1/sandboxes <<'EOF'
apiVersion: sandbox.ai/v1
kind: Sandbox
metadata: {}
spec:
  runtime:
    preset: python
  ttlSeconds: 300
EOF
```

Request with backend extensions:

```bash
curl -sS \
  -H 'Content-Type: application/json' \
  -d '{
    "apiVersion": "sandbox.ai/v1",
    "kind": "Sandbox",
    "metadata": {},
    "spec": {
      "runtime": { "preset": "python", "version": "3.12" },
      "network": {
        "egress": {
          "mode": "proxy",
          "allow": ["pypi.org"],
          "denyByDefault": true
        }
      },
      "scheduling": {
        "backend": "docker",
        "priority": "normal",
        "preferWarm": false
      },
      "extensions": {
        "docker": {
          "hostConfig": {
            "capDrop": ["ALL"]
          }
        }
      }
    }
  }' \
  http://127.0.0.1:7847/v1/sandboxes
```

Typical response:

```json
{
  "sandbox_id": "0a81f08d-7fa7-4f32-9363-51f7a3f82018",
  "lease_token": "b25f5b4c-0902-4b8c-9a34-f2bb8e6fbc70",
  "status": "running",
  "expires_at": "2026-04-21T08:55:00+00:00",
  "backend": "docker"
}
```

### `GET /v1/sandboxes`

Query parameters:

- `limit`: default `50`, clamped to `1..500`
- `offset`: default `0`

```bash
curl -sS 'http://127.0.0.1:7847/v1/sandboxes?limit=10&offset=0'
```

Example response:

```json
{
  "items": [
    {
      "sandbox_id": "0a81f08d-7fa7-4f32-9363-51f7a3f82018",
      "status": "running",
      "backend": "docker",
      "created_at": "2026-04-21T08:50:00+00:00",
      "expires_at": "2026-04-21T08:55:00+00:00",
      "error_message": null
    }
  ],
  "limit": 10,
  "offset": 0
}
```

### `GET /v1/sandboxes/:id`

```bash
curl -sS http://127.0.0.1:7847/v1/sandboxes/<SANDBOX_ID>
```

Example response:

```json
{
  "sandbox_id": "0a81f08d-7fa7-4f32-9363-51f7a3f82018",
  "status": "running",
  "backend": "docker",
  "created_at": "2026-04-21T08:50:00+00:00",
  "expires_at": "2026-04-21T08:55:00+00:00",
  "error_message": null
}
```

### `POST /v1/sandboxes/:id/exec`

```bash
curl -sS \
  -H 'Content-Type: application/json' \
  -H 'X-Lease-Token: <LEASE_TOKEN>' \
  -d '{"command":"python -c '\''print(42)'\''"}' \
  http://127.0.0.1:7847/v1/sandboxes/<SANDBOX_ID>/exec
```

Example response:

```json
{
  "stdout": "42\n",
  "stderr": "",
  "exit_code": 0,
  "duration_ms": 37
}
```

When called with `?stream=1`, the response is `application/x-ndjson`:

```json
{"event":"started","sandbox_id":"sb-1","backend":"docker"}
{"event":"stdout","chunk":"collecting...\n"}
{"event":"stderr","chunk":""}
{"event":"completed","exit_code":0,"duration_ms":812}
```

### `POST /v1/sandboxes/:id/files`

Uploads raw bytes into the sandbox. The target path is passed as a query parameter.

```bash
curl -sS \
  -X POST \
  -H 'X-Lease-Token: <LEASE_TOKEN>' \
  --data-binary @script.py \
  'http://127.0.0.1:7847/v1/sandboxes/<SANDBOX_ID>/files?path=script.py'
```

### `GET /v1/sandboxes/:id/files/*path`

Downloads raw bytes from the sandbox.

```bash
curl -sS \
  -H 'X-Lease-Token: <LEASE_TOKEN>' \
  http://127.0.0.1:7847/v1/sandboxes/<SANDBOX_ID>/files/result.txt
```

### `POST /v1/sandboxes/:id/snapshot`

```bash
curl -sS \
  -X POST \
  -H 'X-Lease-Token: <LEASE_TOKEN>' \
  http://127.0.0.1:7847/v1/sandboxes/<SANDBOX_ID>/snapshot
```

If the backend does not support snapshots, the daemon returns:

```json
{
  "error": {
    "code": "NOT_SUPPORTED",
    "message": "snapshot",
    "details": {}
  }
}
```

### `POST /v1/sandboxes/restore`

```json
{
  "snapshot_id": "snap-123",
  "spec": {
    "apiVersion": "sandbox.ai/v1",
    "kind": "Sandbox",
    "metadata": {},
    "spec": {
      "runtime": { "preset": "python" },
      "scheduling": { "backend": "docker" }
    }
  }
}
```

### `GET /v1/admin/tenants/:id/usage`

Returns current hourly and concurrent usage for the tenant.

### `DELETE /v1/sandboxes/:id`

```bash
curl -i \
  -H 'X-Lease-Token: <LEASE_TOKEN>' \
  -X DELETE \
  http://127.0.0.1:7847/v1/sandboxes/<SANDBOX_ID>
```

Success response:

```text
HTTP/1.1 204 No Content
```

Destroy is idempotent at the backend layer.

## Stable Error Codes

### `SPEC_INVALID` (`422 Unprocessable Entity`)

Returned when the spec is malformed or semantically invalid.

### `LEASE_INVALID` (`403 Forbidden`)

Returned when `X-Lease-Token` is missing or wrong.

### `SANDBOX_NOT_FOUND` (`404 Not Found`)

Returned when the daemon or backend cannot find the requested sandbox.

### `SANDBOX_EXPIRED` (`410 Gone`)

Returned when the sandbox exists in persistence but is no longer running.

### `BACKEND_UNAVAILABLE` (`503 Service Unavailable`)

Returned when the selected backend is unavailable.

### `EXEC_TIMEOUT` (`504 Gateway Timeout`)

Returned when the backend times out while executing a command.

### `INTERNAL_ERROR` (`500 Internal Server Error`)

Returned when the daemon or backend fails in a way that does not map to a more specific public code.

## Behavioral Notes

- the daemon stores submitted specs as JSON for audit consistency, even when the client sends YAML
- secret values never appear in API responses
- backend native handles are persisted internally but never exposed by the HTTP contract
- `exec` is delegated to the selected backend and non-zero exit codes remain successful HTTP responses
