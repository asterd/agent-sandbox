# Hello Sandbox

The smallest useful AgentSandbox example: create a sandbox, run one command, print the result.

## Setup

From this directory:

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -e ../../sdks/python
```

In another terminal, start the daemon from the repository root:

```bash
cargo run -p agentsandbox-daemon
```

Run the example:

```bash
python run.py
```

If you already have the workspace SDK virtualenv, this also works:

```bash
../../sdks/python/.venv/bin/python run.py
```

## Expected output

See [expected_output.txt](expected_output.txt).

## Troubleshooting

- `ModuleNotFoundError: No module named 'agentsandbox'`
  Install the local SDK with `pip install -e ../../sdks/python` or use `../../sdks/python/.venv/bin/python`.
- `All connection attempts failed`
  The daemon is not running on `http://127.0.0.1:7847`; start it from the repository root.
- `backend unavailable`
  Docker is not reachable or no compatible backend is enabled in the daemon configuration.
