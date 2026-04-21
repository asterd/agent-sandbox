# AgentSandbox

AgentSandbox is a local Rust daemon with Python and TypeScript SDKs for running commands inside isolated sandboxes behind a simple HTTP API.

The goal of the project is to give an LLM agent a single primitive:

- create a sandbox
- execute commands
- inspect status and TTL
- destroy the sandbox

without exposing the underlying backend details to clients. Today the implemented backend is Docker.

## Repository status

The repository already contains a working base for:

- public `sandbox.ai/v1alpha1` spec
- `Spec -> IR` compilation in the `agentsandbox-core` crate
- local HTTP daemon with SQLite persistence
- Docker adapter for create / exec / inspect / destroy
- async Python SDK
- SDK TypeScript async

Important current limitations:

- the only available backend is Docker
- Docker egress filtering in `v1alpha1` is hostname-based, resolved once at sandbox startup, and enforced with container-local `iptables`
- the documentation in `ROADMAP.md` goes beyond the current state of the repository

## Architecture

```text
LLM agent / app
        |
        v
Python SDK / TypeScript SDK
        |
        v
AgentSandbox daemon (Rust + Axum)
        |
        v
Backend adapter
        |
        +-- Docker
```

Workspace structure:

```text
.
├── crates/
│   ├── agentsandbox-core/
│   ├── agentsandbox-daemon/
│   └── agentsandbox-docker/
├── sdks/
│   ├── python/
│   └── typescript/
├── tests/
├── ROADMAP.md
└── PROMPT.md
```

## Requirements

- recent Rust toolchain
- Docker running
- Python 3.10+
- Node.js 18+

## Quick Start

### 1. Start the daemon

From the repository root:

```bash
cargo run -p agentsandbox-daemon
```

By default the daemon:

- listens on `http://127.0.0.1:7847`
- uses a local SQLite database at `sqlite://agentsandbox.db`

Useful variables:

- `AGENTSANDBOX_ADDR`
- `AGENTSANDBOX_DB`

Example:

```bash
AGENTSANDBOX_ADDR=127.0.0.1:9000 AGENTSANDBOX_DB=sqlite://dev.db cargo run -p agentsandbox-daemon
```

### 2. Check health

```bash
curl http://127.0.0.1:7847/v1/health
```

Expected response:

```json
{ "status": "ok", "backend": "docker" }
```

## Using the Python SDK

Local editable installation:

```bash
cd sdks/python
python -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
```

Example:

```python
import asyncio
from agentsandbox import Sandbox


async def main():
    async with Sandbox(
        runtime="python",
        ttl=300,
        egress=["pypi.org", "files.pythonhosted.org"],
    ) as sb:
        result = await sb.exec("python -c 'print(42)'")
        print(result.stdout)


asyncio.run(main())
```

The Python SDK exposes:

- `Sandbox`
- `SandboxConfig`
- `SandboxInfo`
- `ExecResult`
- typed exceptions such as `SandboxNotFoundError`, `SpecInvalidError`, `LeaseInvalidError`

## Using the TypeScript SDK

Local installation:

```bash
cd sdks/typescript
npm install
npm run build
```

Example:

```ts
import { Sandbox } from "agentsandbox";

await using sb = await Sandbox.create({
  runtime: "python",
  ttl: 300,
  egress: ["pypi.org", "files.pythonhosted.org"],
});

const result = await sb.exec("python -c 'print(42)'");
console.log(result.stdout);
```

The TypeScript SDK uses native `fetch` and maps daemon errors to dedicated classes such as:

- `SandboxNotFoundError`
- `SandboxExpiredError`
- `SpecInvalidError`
- `BackendUnavailableError`
- `LeaseInvalidError`

## API HTTP

Available endpoints today:

- `GET /v1/health`
- `POST /v1/sandboxes`
- `GET /v1/sandboxes`
- `GET /v1/sandboxes/:id`
- `POST /v1/sandboxes/:id/exec`
- `DELETE /v1/sandboxes/:id`

The daemon accepts the spec in JSON or YAML. Creation also returns a lease token that must be sent in the `X-Lease-Token` header for `exec` and `destroy`.

Minimal creation example:

```bash
curl -sS \
  -H 'Content-Type: application/json' \
  -d '{
    "apiVersion": "sandbox.ai/v1alpha1",
    "kind": "Sandbox",
    "metadata": {},
    "spec": {
      "runtime": { "preset": "python" },
      "ttlSeconds": 300,
      "resources": {
        "memoryMb": 512,
        "cpuMillicores": 1000
      }
    }
  }' \
  http://127.0.0.1:7847/v1/sandboxes
```

Typical response:

```json
{
  "sandbox_id": "sb-...",
  "lease_token": "...",
  "status": "running",
  "expires_at": "2026-04-20T10:00:00+00:00",
  "backend": "docker"
}
```

`exec` example:

```bash
curl -sS \
  -H 'Content-Type: application/json' \
  -H "X-Lease-Token: <LEASE_TOKEN>" \
  -d '{ "command": "python -c '\''print(42)'\''" }' \
  http://127.0.0.1:7847/v1/sandboxes/<SANDBOX_ID>/exec
```

## Spec model

The current public spec is `sandbox.ai/v1alpha1`.

Main fields:

- `runtime.preset` oppure `runtime.image`
- `resources.cpuMillicores`
- `resources.memoryMb`
- `resources.diskMb`
- `network.egress.allow`
- `secrets`
- `ttlSeconds`

Presets currently supported in the core:

- `python` -> `python:3.12-slim`
- `node` -> `node:20-slim`
- `rust` -> `rust:1.77-slim`
- `shell` -> `ubuntu:24.04`
- `custom` -> requires `runtime.image`

YAML example:

```yaml
apiVersion: sandbox.ai/v1alpha1
kind: Sandbox
metadata:
  name: demo
spec:
  runtime:
    preset: python
    env:
      APP_ENV: dev
  resources:
    cpuMillicores: 1000
    memoryMb: 512
  network:
    egress:
      allow:
        - pypi.org
        - files.pythonhosted.org
      denyByDefault: true
  ttlSeconds: 300
```

## Development

Useful commands:

```bash
cargo check
cargo test
```

Python SDK:

```bash
cd sdks/python
pytest
```

TypeScript SDK:

```bash
cd sdks/typescript
npm test
```

Docker conformance tests:

```bash
cargo test -p agentsandbox-docker -- --ignored --test-threads=1
```

## Operational notes

- The daemon creates or opens the SQLite database and applies migrations at startup.
- The TTL reaper runs in the background inside the daemon.
- Destruction via SDK is best-effort: teardown errors do not mask the caller's main error.
- Secrets are resolved by the daemon on the host side; the SDKs send only references, not the real values.

## Roadmap

The project roadmap and planned phases are in [ROADMAP.md](/Users/ddurzo/Development/ai/agent-sandbox/ROADMAP.md).
