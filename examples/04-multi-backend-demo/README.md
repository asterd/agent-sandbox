# Multi-backend Demo

This example queries the daemon for available backends, filters those that support the `python` preset, and runs the same command on each of them.

It intentionally follows the real daemon contract in this repository:

- `GET /v1/backends` returns `{ "items": [...] }`
- backend health is inferred by successful execution, not by a `healthy` field in the response

## Setup

From this directory:

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -e ../../sdks/python
```

In another terminal, build the backend plugins you want to exercise and start the daemon from the repository root:

```bash
cargo build -p agentsandbox-backend-docker
cargo run -p agentsandbox-daemon
```

Run the example:

```bash
python demo.py
```

If you already have the workspace SDK virtualenv, this also works:

```bash
../../sdks/python/.venv/bin/python demo.py
```

## Expected output

See [expected_output.txt](expected_output.txt). Backend ids and durations depend on the local daemon configuration.

## Troubleshooting

- `All connection attempts failed`
  The daemon is not running on `http://127.0.0.1:7847`; start it from the repository root.
- `nessun backend compatibile con il preset python`
  No enabled backend advertises support for the `python` preset; review the daemon backend configuration.
- `backend unavailable`
  A listed backend could not actually create or execute a sandbox; verify Docker and backend-specific runtime prerequisites.
