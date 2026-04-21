# HTTP API v1

This document describes the current local daemon contract served by `agentsandbox-daemon`.

Base URL:

```text
http://127.0.0.1:7847
```

## Content types

- `POST /v1/sandboxes` accepts `application/json`, `application/yaml`, or `text/yaml`
- all successful responses are JSON, except `DELETE /v1/sandboxes/:id`, which returns `204 No Content`
- all errors use the same JSON envelope

## Authentication model

There is no user authentication in `v0.1.0`.

Sandbox mutation is protected by a lease token:

- `POST /v1/sandboxes` returns `lease_token`
- `POST /v1/sandboxes/:id/exec` requires `X-Lease-Token`
- `DELETE /v1/sandboxes/:id` requires `X-Lease-Token` when the sandbox exists

## Error envelope

Every daemon error has this shape:

```json
{
  "error": {
    "code": "SPEC_INVALID",
    "message": "apiVersion non supportata: sandbox.ai/v2",
    "details": {}
  }
}
```

## Endpoints

### `GET /v1/health`

Response:

```json
{
  "status": "ok",
  "backend": "docker"
}
```

### `POST /v1/sandboxes`

Create a sandbox from a `sandbox.ai/v1alpha1` or `sandbox.ai/v1beta1` spec.

Minimal request:

```bash
curl -sS \
  -H 'Content-Type: application/json' \
  -d '{
    "apiVersion": "sandbox.ai/v1alpha1",
    "kind": "Sandbox",
    "metadata": {},
    "spec": {
      "runtime": { "preset": "python" },
      "ttlSeconds": 300
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

YAML also works:

```bash
curl -sS \
  -H 'Content-Type: application/yaml' \
  --data-binary @- \
  http://127.0.0.1:7847/v1/sandboxes <<'EOF'
apiVersion: sandbox.ai/v1alpha1
kind: Sandbox
metadata: {}
spec:
  runtime:
    preset: python
  ttlSeconds: 300
EOF
```

Minimal `v1beta1` request:

```bash
curl -sS \
  -H 'Content-Type: application/json' \
  -d '{
    "apiVersion": "sandbox.ai/v1beta1",
    "kind": "Sandbox",
    "metadata": {},
    "spec": {
      "runtime": { "preset": "python", "version": "3.12" },
      "resources": { "timeoutMs": 30000 },
      "network": {
        "egress": {
          "allow": ["pypi.org"],
          "denyByDefault": true,
          "mode": "proxy"
        }
      },
      "scheduling": {
        "backend": "docker",
        "priority": "normal",
        "preferWarm": false
      },
      "storage": { "volumes": [] },
      "observability": { "auditLevel": "basic", "metricsEnabled": false }
    }
  }' \
  http://127.0.0.1:7847/v1/sandboxes
```

### `GET /v1/sandboxes`

Query parameters:

- `limit` default `50`, clamped to `1..500`
- `offset` default `0`

Example:

```bash
curl -sS 'http://127.0.0.1:7847/v1/sandboxes?limit=10&offset=0'
```

Response:

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

Inspect a sandbox.

```bash
curl -sS http://127.0.0.1:7847/v1/sandboxes/<SANDBOX_ID>
```

Response:

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

Run a shell command inside the sandbox.

Request:

```bash
curl -sS \
  -H 'Content-Type: application/json' \
  -H 'X-Lease-Token: <LEASE_TOKEN>' \
  -d '{"command":"python -c '\''print(42)'\''"}' \
  http://127.0.0.1:7847/v1/sandboxes/<SANDBOX_ID>/exec
```

Response:

```json
{
  "stdout": "42\n",
  "stderr": "",
  "exit_code": 0,
  "duration_ms": 37
}
```

Important:

- non-zero `exit_code` is not an HTTP error
- lease validation happens before execution
- execution is delegated to the backend adapter and captures `stdout` / `stderr`

### `DELETE /v1/sandboxes/:id`

Destroy a sandbox.

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

If the sandbox is already gone, destroy is idempotent at the adapter level.

## Stable error codes

### `SPEC_INVALID` (`422 Unprocessable Entity`)

Returned when the spec is malformed or semantically invalid.

Example:

```bash
curl -sS \
  -H 'Content-Type: application/json' \
  -d '{
    "apiVersion": "sandbox.ai/v2",
    "kind": "Sandbox",
    "metadata": {},
    "spec": { "runtime": { "preset": "python" } }
  }' \
  http://127.0.0.1:7847/v1/sandboxes
```

```json
{
  "error": {
    "code": "SPEC_INVALID",
    "message": "spec sandbox.ai/v1beta1 non valida",
    "details": {
      "apiVersion": "sandbox.ai/v1beta1",
      "validationErrors": [
        {
          "path": "/spec/resources/cpuMillicores",
          "message": "/spec/resources/cpuMillicores is less than the minimum of 0"
        }
      ]
    }
  }
}
```

### `LEASE_INVALID` (`403 Forbidden`)

Returned when `X-Lease-Token` is missing or wrong.

Example:

```bash
curl -sS \
  -H 'Content-Type: application/json' \
  -d '{"command":"echo hi"}' \
  http://127.0.0.1:7847/v1/sandboxes/<SANDBOX_ID>/exec
```

```json
{
  "error": {
    "code": "LEASE_INVALID",
    "message": "lease token mancante o non valido",
    "details": {}
  }
}
```

### `SANDBOX_NOT_FOUND` (`404 Not Found`)

Returned when the daemon or adapter cannot find the requested sandbox.

```json
{
  "error": {
    "code": "SANDBOX_NOT_FOUND",
    "message": "sandbox <id> non trovata",
    "details": {}
  }
}
```

### `SANDBOX_EXPIRED` (`410 Gone`)

Returned when the sandbox exists in persistence but is no longer running.

Typical message:

```json
{
  "error": {
    "code": "SANDBOX_EXPIRED",
    "message": "sandbox <id> non è in esecuzione (status=expired)",
    "details": {}
  }
}
```

### `BACKEND_UNAVAILABLE` (`503 Service Unavailable`)

Returned when the Docker backend is unavailable.

Typical message:

```json
{
  "error": {
    "code": "BACKEND_UNAVAILABLE",
    "message": "docker backend unavailable: ...",
    "details": {}
  }
}
```

### `EXEC_TIMEOUT` (`504 Gateway Timeout`)

Reserved for backend execution timeouts.

Typical envelope:

```json
{
  "error": {
    "code": "EXEC_TIMEOUT",
    "message": "exec timeout: ...",
    "details": {}
  }
}
```

### `INTERNAL_ERROR` (`500 Internal Server Error`)

Returned when the daemon or adapter fails in a way that does not map to a more specific public code.

Typical envelope:

```json
{
  "error": {
    "code": "INTERNAL_ERROR",
    "message": "errore persistenza",
    "details": {}
  }
}
```

## Behavioral notes

- The daemon stores the submitted spec as JSON for audit consistency, even if the client sent YAML.
- Secret values never appear in API responses.
- `exec` uses `/bin/sh -c <command>` in the Docker adapter.
- For `network.egress` limitations, see [spec-v1alpha1.md](spec-v1alpha1.md).
