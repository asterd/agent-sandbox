# Deployment Guide

This document explains how AgentSandbox is deployed and used in practice today.

The most important distinction is:

- `agentsandbox-daemon` is the server/runtime component
- the Python and TypeScript SDKs are client libraries
- the SDKs do not embed Rust and do not call native bindings
- the bridge between SDKs and Rust is HTTP

## What Gets Built

The Rust workspace contains multiple crates, but only one deployable server binary:

- `agentsandbox-daemon`

Everything else is a library crate, backend plugin crate, conformance helper, or SDK support crate.

Typical release build:

```bash
cargo build --release -p agentsandbox-daemon
```

Primary deployable artifact:

```text
target/release/agentsandbox-daemon
```

The `.rlib` and `.d` files produced by Cargo are build artifacts, not runtime deployment units.

## Runtime Architecture

Current architecture:

```text
Python app / TypeScript app
            |
            | HTTP
            v
agentsandbox-daemon
            |
            v
backend plugin
            |
            +-- Docker
            +-- Podman
            +-- gVisor via Docker runtime
```

The daemon owns:

- spec validation and compile pipeline
- SQLite persistence
- lease token enforcement
- backend selection
- create / exec / inspect / destroy lifecycle

The SDKs own:

- building a request spec
- sending HTTP requests to the daemon
- mapping daemon error envelopes to language-native exceptions
- exposing a friendly client API to the application

## How the Bridge Works

There is no FFI bridge.

There is no `pyo3`, no C ABI, no Node native addon, and no embedding of the Rust daemon into Python or TypeScript.

The bridge is plain HTTP:

1. the application uses the SDK
2. the SDK sends `POST /v1/sandboxes`
3. the daemon compiles the spec and creates the sandbox
4. the daemon returns `sandbox_id` and `lease_token`
5. the SDK uses those values for `exec`, `inspect`, and `destroy`

This means the SDKs and the daemon can run:

- on the same machine
- on different machines
- in separate containers

as long as the SDK can reach the daemon URL.

## Server Deployment

### Minimum Requirements

- Linux or macOS host for the daemon itself
- Rust toolchain only for build time
- Docker running locally for the default backend
- writable filesystem path for SQLite

Additional backend requirements:

- Podman backend: reachable Podman socket
- gVisor backend: Linux host, Docker runtime configured with `runsc`

### Configuration

By default the daemon reads:

```text
agentsandbox.toml
```

Key runtime settings:

- listen host/port
- SQLite database URL
- enabled backends
- backend-specific options such as Docker socket or gVisor runtime name

You can also override settings with environment variables such as:

- `AS_CONFIG`
- `AS_DAEMON_HOST`
- `AS_DAEMON_PORT`
- `AS_DATABASE_URL`
- `AS_BACKENDS_ENABLED`
- `AS_BACKENDS_DOCKER_SOCKET`
- `AS_BACKENDS_GVISOR_SOCKET`
- `AS_BACKENDS_GVISOR_RUNTIME`
- `AS_BACKENDS_PODMAN_SOCKET`

### Running the Daemon

Typical local run:

```bash
./target/release/agentsandbox-daemon
```

Run with explicit config and DB path:

```bash
AS_CONFIG=/etc/agentsandbox/agentsandbox.toml \
AS_DATABASE_URL=sqlite:///var/lib/agentsandbox/agentsandbox.db \
./agentsandbox-daemon
```

The daemon applies SQL migrations on startup.

## Python SDK Deployment

### What It Is

The Python SDK is a normal Python package. It does not compile Rust code and does not link against the daemon binary.

Its job is to call the daemon over HTTP using `httpx`.

Relevant implementation:

- `sdks/python/agentsandbox/client.py`

### How It Is Installed

Current repository-local install:

```bash
cd sdks/python
pip install -e .
```

Development install:

```bash
cd sdks/python
pip install -e ".[dev]"
```

Package metadata lives in:

- `sdks/python/pyproject.toml`

### How It Is Used

Example:

```python
import asyncio
from agentsandbox import Sandbox

async def main() -> None:
    async with Sandbox(
        runtime="python",
        ttl=300,
        daemon_url="http://127.0.0.1:7847",
    ) as sb:
        result = await sb.exec("python -c 'print(42)'")
        print(result.stdout, end="")

asyncio.run(main())
```

Under the hood:

- `Sandbox._create()` sends `POST /v1/sandboxes`
- `Sandbox.exec()` sends `POST /v1/sandboxes/:id/exec`
- `Sandbox.inspect()` sends `GET /v1/sandboxes/:id`
- `Sandbox._destroy()` sends `DELETE /v1/sandboxes/:id`

Default daemon URL:

```text
http://127.0.0.1:7847
```

If the daemon runs elsewhere, pass `daemon_url=...`.

## TypeScript SDK Deployment

### What It Is

The TypeScript SDK is a standard Node package. It does not compile to a native binding and does not embed Rust.

Its job is to call the daemon over HTTP using `fetch`.

Relevant implementation:

- `sdks/typescript/src/client.ts`

### How It Is Installed

Current repository-local install:

```bash
cd sdks/typescript
npm install
npm run build
```

Package metadata lives in:

- `sdks/typescript/package.json`

### How It Is Used

Example:

```ts
import { Sandbox } from "agentsandbox";

await using sb = await Sandbox.create({
  runtime: "python",
  ttl: 300,
  daemonUrl: "http://127.0.0.1:7847",
});

const result = await sb.exec("python -c 'print(42)'");
console.log(result.stdout.trim());
```

Under the hood:

- `Sandbox.create()` sends `POST /v1/sandboxes`
- `exec()` sends `POST /v1/sandboxes/:id/exec`
- `inspect()` sends `GET /v1/sandboxes/:id`
- `destroy()` sends `DELETE /v1/sandboxes/:id`

If the daemon runs remotely, set `daemonUrl`.

## End-to-End Request Flow

Example flow for a Python app:

1. app calls `Sandbox(...)`
2. SDK builds a `sandbox.ai/v1` request body
3. SDK sends `POST /v1/sandboxes`
4. daemon validates and compiles the spec into IR
5. daemon selects a backend
6. backend creates the actual sandbox
7. daemon returns:
   - `sandbox_id`
   - `lease_token`
   - `backend`
8. SDK stores `sandbox_id` and `lease_token`
9. app calls `exec(...)`
10. SDK sends `POST /v1/sandboxes/:id/exec` with `X-Lease-Token`
11. daemon forwards the command to the backend
12. daemon returns stdout, stderr, exit code, and duration
13. on shutdown, SDK sends `DELETE /v1/sandboxes/:id`

The same flow applies to TypeScript.

## Hidden Backend Extensions

The public spec remains `sandbox.ai/v1`.

Backend-specific hidden extensions can be sent through the daemon-only header:

```text
X-AgentSandbox-Extensions
```

The header value must be a JSON object.

Example:

```bash
curl -sS \
  -H 'Content-Type: application/json' \
  -H 'X-AgentSandbox-Extensions: {"gvisor":{"network":"host"}}' \
  -d '{
    "apiVersion": "sandbox.ai/v1",
    "kind": "Sandbox",
    "metadata": {},
    "spec": {
      "runtime": { "preset": "python" },
      "scheduling": { "backend": "gvisor" }
    }
  }' \
  http://127.0.0.1:7847/v1/sandboxes
```

Today this path exists at the daemon/backend layer, not as a public SDK field.

## Deployment Topologies

### Single Host

Best for local development or a small internal service.

```text
same machine:
- app
- agentsandbox-daemon
- Docker / Podman
- SQLite file
```

Benefits:

- simplest setup
- no network hop between SDK and daemon
- easiest debugging

### Split App and Daemon

Useful when application code and sandbox infrastructure are separated.

```text
host A:
- Python app or Node app

host B:
- agentsandbox-daemon
- Docker / Podman / gVisor
- SQLite
```

Requirements:

- the SDK must point to the daemon URL
- network policy must allow the app to reach the daemon
- you should add auth/TLS in front of the daemon because the current `v0.1.0` API is intentionally minimal

### Containerized Daemon

You can containerize the daemon itself, but the backend socket must still be reachable:

- Docker socket mount for Docker backend
- Podman socket mount for Podman backend
- gVisor runtime configured in the host Docker daemon

In practice this means the daemon container is still an infrastructure-side component, not a self-contained sandbox appliance.

## Recommended Linux Service Layout

Suggested directories:

- binary: `/usr/local/bin/agentsandbox-daemon`
- config: `/etc/agentsandbox/agentsandbox.toml`
- data: `/var/lib/agentsandbox/agentsandbox.db`
- logs: handled by `systemd` journal or your log pipeline

Example `systemd` unit:

```ini
[Unit]
Description=AgentSandbox Daemon
After=network.target docker.service
Requires=docker.service

[Service]
Type=simple
User=agentsandbox
Group=agentsandbox
Environment=AS_CONFIG=/etc/agentsandbox/agentsandbox.toml
Environment=AS_DATABASE_URL=sqlite:///var/lib/agentsandbox/agentsandbox.db
ExecStart=/usr/local/bin/agentsandbox-daemon
Restart=on-failure
RestartSec=2

[Install]
WantedBy=multi-user.target
```

If you use Docker backend, the service user must be able to reach the Docker socket.

## What Is Not Bundled

AgentSandbox is not currently shipped as:

- a single self-sufficient appliance image
- a Python wheel containing the Rust daemon
- an npm package containing the Rust daemon
- a native in-process library for Python or Node

Today the supported operational model is:

- deploy the daemon separately
- install the SDK in the application
- connect the SDK to the daemon over HTTP

## Operational Checklist

- build `agentsandbox-daemon`
- choose a backend and verify its runtime is available
- create the daemon config
- choose a stable SQLite path
- start the daemon
- verify `GET /v1/health`
- install the Python or TypeScript SDK in the application
- point the SDK at the daemon URL
- test create / exec / destroy end-to-end

## Related Documents

- [Getting started](getting-started.md)
- [HTTP API v1](api-http-v1.md)
- [Spec v1](spec-v1.md)
- [gVisor backend](backends/gvisor.md)
