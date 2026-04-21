# Python Code Review Agent

This example asks Claude to review a Python file, extracts fixed code as JSON, and then verifies the result inside an isolated AgentSandbox sandbox.

## What it demonstrates

- local Python SDK usage
- sandbox execution without guest network access
- a realistic agent loop: inspect code, propose a fix, run the fix in isolation

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

## Sample output

The exact bug wording, explanation text, sandbox id, and timings are dynamic. The stable structure looks like this:

```text
Reviewing: sample_code/buggy_script.py
Using model: claude-sonnet-4-20250514
--------------------------------------------------
Requesting review from Claude...

Bugs found (3):
 - <bug 1>
 - <bug 2>
 - <bug 3>

Explanation:
<short explanation from Claude>

Running fixed code inside AgentSandbox...

Sandbox output:
------------------------------
3.0
[2, 3]
{'host': 'localhost', 'port': '8080'}
------------------------------
Execution succeeded (exit 0, <duration-ms>ms)
```

## Files

- `agent.py`: entry point
- `sample_code/buggy_script.py`: intentionally broken Python file used for the demo
- `.env.example`: optional environment variable reference
