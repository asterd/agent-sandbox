# Python Code Review Agent

This example asks an OpenAI-compatible model to review a Python file, extracts fixed code as JSON, and then verifies the result inside an isolated AgentSandbox sandbox.

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

## Sample output

The exact bug wording, explanation text, sandbox id, and timings are dynamic. The stable structure looks like this:

```text
Reviewing: sample_code/buggy_script.py
Using provider: openai
Using model: gpt-4.1-mini
--------------------------------------------------
Requesting review from LLM...

Bugs found (3):
 - <bug 1>
 - <bug 2>
 - <bug 3>

Explanation:
<short explanation from the model>

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
- `.env.example`: OpenAI standard example, adattabile a Groq e provider compatibili
