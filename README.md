# AgentSandbox

AgentSandbox is a local Rust daemon plus Python and TypeScript SDKs for running agent-generated code inside isolated sandboxes behind a small HTTP API.

The project exposes one primitive:

- create a sandbox
- execute commands inside it
- inspect state and TTL
- destroy it

Clients do not need to know whether the backend is Docker or something else. In `v0.1.0`, the only implemented backend is Docker.

## Status

Implemented today:

- public `sandbox.ai/v1` spec
- `spec -> IR` compilation in `agentsandbox-core`
- local HTTP daemon with Axum + SQLite persistence
- Docker adapter for create / exec / inspect / destroy
- async Python SDK
- async TypeScript SDK

Current limits that matter:

- Docker is the only backend
- `network.egress` in `v1` is hostname-based and resolved once at sandbox creation
- egress enforcement relies on `iptables` inside the guest when an allowlist is configured
- the planned stable replacement for filtered egress is the proxy L4 path in `ROADMAP_STABLE.md` FASE C
- the SDKs are local workspace packages in this repository; registry publishing is not part of the current phase

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

## Requirements

- Rust toolchain
- Docker running locally
- Python 3.10+
- Node.js 18+

## Quickstart

### 1. Start the daemon

From the repository root:

```bash
cargo run -p agentsandbox-daemon
```

By default the daemon loads `agentsandbox.toml` from the repo root and:

- listens on `http://127.0.0.1:7847`
- stores state in `sqlite://agentsandbox.db`

Useful environment variables:

- `AS_CONFIG`
- `AS_DAEMON_PORT`
- `AS_DATABASE_URL`

Example:

```bash
AS_DAEMON_PORT=9000 AS_DATABASE_URL=sqlite://dev.db cargo run -p agentsandbox-daemon
```

### 2. Check health

```bash
curl http://127.0.0.1:7847/v1/health
```

Expected response:

```json
{"status":"ok","backend":"docker"}
```

### 3. Use the Python SDK

```bash
cd sdks/python
python -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
```

```python
import asyncio
from agentsandbox import Sandbox


async def main() -> None:
    async with Sandbox(
        runtime="python",
        ttl=300,
    ) as sb:
        result = await sb.exec("python -c 'print(42)'")
        print(result.stdout, end="")


asyncio.run(main())
```

### 4. Use the TypeScript SDK

```bash
cd sdks/typescript
npm install
npm run build
```

```ts
import { Sandbox } from "agentsandbox";

await using sb = await Sandbox.create({
  runtime: "python",
  ttl: 300,
});

const result = await sb.exec("python -c 'print(42)'");
console.log(result.stdout.trim());
```

## Public docs

- [Getting started](docs/getting-started.md)
- [Deployment guide](docs/deployment.md)
- [HTTP API v1](docs/api-http-v1.md)
- [Spec v1](docs/spec-v1.md)
- [Examples](examples/README.md)

## Known limits

### `network.egress` in `v1`

- DNS resolution happens once, at sandbox creation time
- DNS rebinding is not prevented
- wildcard hostnames such as `*.example.com` are rejected
- direct IPs in `egress.allow` are rejected
- if the runtime image cannot enforce the allowlist, sandbox creation fails instead of degrading to open egress
- the planned stable replacement for this mechanism is the proxy L4 design in `ROADMAP_STABLE.md` FASE C

### API and SDK behavior

- `exec` and `destroy` require the `X-Lease-Token` returned at create time
- a non-zero process exit code is not an API error; it is returned as `exit_code`
- daemon startup fails cleanly if Docker is unavailable

## Error model

The daemon returns a stable JSON envelope:

```json
{
  "error": {
    "code": "SPEC_INVALID",
    "message": "apiVersion non supportata: sandbox.ai/v2",
    "details": {}
  }
}
```

Stable error codes currently exposed:

- `SANDBOX_NOT_FOUND`
- `SANDBOX_EXPIRED`
- `SPEC_INVALID`
- `BACKEND_UNAVAILABLE`
- `EXEC_TIMEOUT`
- `LEASE_INVALID`
- `INTERNAL_ERROR`

Concrete request/response examples live in [docs/api-http-v1.md](docs/api-http-v1.md).
