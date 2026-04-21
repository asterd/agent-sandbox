# Python Code Review Agent

This example asks Claude to review a Python file, extracts fixed code as JSON, and verifies the fix inside an isolated AgentSandbox sandbox.

## What it demonstrates

- local Python SDK usage
- sandbox execution without guest network access
- an agent loop: inspect code, propose a fix, run the fix in isolation

## Setup

From this directory:

```bash
python -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
export ANTHROPIC_API_KEY=your_key_here
```

In another terminal, start the daemon from the repository root:

```bash
cargo run -p agentsandbox-daemon
```

Run the example:

```bash
python agent.py sample_code/buggy_script.py
```

## Expected output

See [expected_output.txt](expected_output.txt). Model wording and durations are dynamic, but the output structure should remain the same.

## Files

- `agent.py`: entry point
- `sample_code/buggy_script.py`: intentionally broken Python file used for the demo
- `requirements.txt`: local SDK plus Anthropic dependency

## Troubleshooting

- `ANTHROPIC_API_KEY non impostata`
  Export `ANTHROPIC_API_KEY` before running the example.
- `ModuleNotFoundError: No module named 'agentsandbox'`
  Recreate the example virtualenv and run `pip install -r requirements.txt`.
- `All connection attempts failed`
  The daemon is not running on `http://127.0.0.1:7847`; start it from the repository root before launching the agent.
