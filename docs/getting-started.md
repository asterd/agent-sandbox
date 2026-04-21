# Getting Started

This guide is for a developer starting from a fresh clone of the repository.

## Requirements

- Docker running locally
- Rust toolchain
- Python 3.10+
- Node.js 18+

## 1. Start the daemon

From the repository root:

```bash
cargo run -p agentsandbox-daemon
```

Default configuration:

- address: `127.0.0.1:7847`
- SQLite DB: `sqlite://agentsandbox.db`

Override them if needed:

```bash
AGENTSANDBOX_ADDR=127.0.0.1:9000 AGENTSANDBOX_DB=sqlite://dev.db cargo run -p agentsandbox-daemon
```

## 2. Verify the daemon

```bash
curl http://127.0.0.1:7847/v1/health
```

Expected response:

```json
{"status":"ok","backend":"docker"}
```

If Docker is not available, the daemon exits during startup with a backend error instead of panicking.

## 3. Run the Python SDK locally

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


async def main() -> None:
    async with Sandbox(runtime="python", ttl=300) as sb:
        result = await sb.exec("python -c 'print(42)'")
        print(result.stdout, end="")


asyncio.run(main())
```

## 4. Run the TypeScript SDK locally

```bash
cd sdks/typescript
npm install
npm run build
```

Example:

```ts
import { Sandbox } from "agentsandbox";

await using sb = await Sandbox.create({ runtime: "python", ttl: 300 });
const result = await sb.exec("python -c 'print(42)'");
console.log(result.stdout.trim());
```

## 5. Understand the request model

The daemon accepts a `sandbox.ai/v1alpha1` spec in JSON or YAML.

Minimal JSON example:

```json
{
  "apiVersion": "sandbox.ai/v1alpha1",
  "kind": "Sandbox",
  "metadata": {},
  "spec": {
    "runtime": { "preset": "python" },
    "ttlSeconds": 300
  }
}
```

Useful references:

- [docs/api-http-v1.md](api-http-v1.md)
- [docs/spec-v1alpha1.md](spec-v1alpha1.md)

## Operational notes

- `exec` and `destroy` require the `X-Lease-Token` returned by `POST /v1/sandboxes`
- a process returning exit code `1` is still a successful API call; the failure is in the guest process, not in the daemon
- `network.egress` is strict: unsupported allowlists fail sandbox creation instead of silently opening the network
- today `network.egress` still depends on guest-side `iptables`; the planned stable replacement is the proxy L4 path in `ROADMAP_STABLE.md` FASE C
