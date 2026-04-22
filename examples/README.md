# AgentSandbox Examples

This directory contains runnable examples against a local `agentsandbox-daemon`.

## Available examples

| Example | SDK | What it demonstrates | Requires LLM credentials |
| --- | --- | --- | --- |
| [01-hello-sandbox](01-hello-sandbox/README.md) | Python | Minimal sandbox creation and command execution | No |
| [02-code-review-agent](02-code-review-agent/README.md) | Python | An OpenAI-compatible model reviews a Python file, then the fixed code runs in a sandbox | Yes |
| [03-dependency-auditor](03-dependency-auditor/README.md) | TypeScript | `npm audit` inside a sandbox plus a short OpenAI-compatible summary | Yes |
| [04-multi-backend-demo](04-multi-backend-demo/README.md) | Python | The same workload executed on every available Python-capable backend | No |
| [05-file-stream-demo](05-file-stream-demo/README.md) | Python | Upload, NDJSON exec stream, download, and normal teardown | No |

## Common prerequisites

- Docker running locally
- local daemon started with `cargo run -p agentsandbox-daemon`
- at least one backend plugin binary built and discoverable, for example `cargo build -p agentsandbox-backend-docker`
- Python examples can reuse `../sdks/python/.venv/bin/python` if present
- TypeScript example uses the local workspace SDK via `file:../../sdks/typescript`
- LLM-backed examples read `AGENTSANDBOX_LLM_*` settings from `.env` files when present

## Verification

Run `bash examples/verify_all.sh` from the repository root.

The script:

- fails fast if the daemon is unreachable
- runs the examples that do not need LLM credentials
- runs the LLM-backed examples only when `AGENTSANDBOX_LLM_API_KEY` and local dependencies are available
- skips Python examples when the local interpreter cannot import the required modules

## Troubleshooting

- `ModuleNotFoundError: No module named 'agentsandbox'`
  Use `sdks/python/.venv/bin/python` or install the local SDK with `pip install -e ../../sdks/python`.
- `curl: (7) Failed to connect to localhost port 7847`
  Start the daemon from the repository root and ensure at least one backend plugin is discoverable.
- `backend unavailable` or sandbox creation errors
  Check that Docker is running and that at least one backend plugin is installed, healthy, and enabled in the daemon config.
