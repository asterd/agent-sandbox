# Python Code Review Agent

This example asks an OpenAI-compatible model to review a Python file, extracts fixed code as JSON, and verifies the fix inside an isolated AgentSandbox sandbox.

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
cp .env.example .env
# poi popola AGENTSANDBOX_LLM_API_KEY nella .env
```

In another terminal, build at least one backend plugin and start the daemon from the repository root:

```bash
cargo build -p agentsandbox-backend-docker
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
- `requirements.txt`: local SDK plus OpenAI-compatible client deps
- `.env.example`: OpenAI standard example, adattabile a Groq e provider compatibili

## Troubleshooting

- `AGENTSANDBOX_LLM_API_KEY non impostata`
  Copia `.env.example` in `.env` e imposta la chiave del provider.
- `ModuleNotFoundError: No module named 'agentsandbox'`
  Recreate the example virtualenv and run `pip install -r requirements.txt`.
- `All connection attempts failed`
  The daemon is not running on `http://127.0.0.1:7847`; start it from the repository root before launching the agent.
