# AgentSandbox Examples

This directory contains runnable examples against a local `agentsandbox-daemon`.

## Available examples

| Example | SDK | What it demonstrates | Requires Anthropic |
| --- | --- | --- | --- |
| [01-hello-sandbox](01-hello-sandbox/README.md) | Python | Minimal sandbox creation and command execution | No |
| [02-code-review-agent](02-code-review-agent/README.md) | Python | Claude reviews a Python file, then the fixed code runs in a sandbox | Yes |
| [03-dependency-auditor](03-dependency-auditor/README.md) | TypeScript | `npm audit` inside a sandbox plus a short Claude summary | Yes |
| [04-multi-backend-demo](04-multi-backend-demo/README.md) | Python | The same workload executed on every available Python-capable backend | No |

## Common prerequisites

- Docker running locally
- local daemon started with `cargo run -p agentsandbox-daemon`
- Python examples can reuse `../sdks/python/.venv/bin/python` if present
- TypeScript example uses the local workspace SDK via `file:../../sdks/typescript`

## Verification

Run `bash examples/verify_all.sh` from the repository root.

The script:

- fails fast if the daemon is unreachable
- runs the examples that do not need Anthropic credentials
- runs the Anthropic-backed examples only when the required credentials and local dependencies are available

## Troubleshooting

- `ModuleNotFoundError: No module named 'agentsandbox'`
  Use `sdks/python/.venv/bin/python` or install the local SDK with `pip install -e ../../sdks/python`.
- `curl: (7) Failed to connect to localhost port 7847`
  Start the daemon from the repository root with `cargo run -p agentsandbox-daemon`.
- `backend unavailable` or sandbox creation errors
  Check that Docker is running and that at least one backend is enabled in the daemon config.
