# E2E Smoke

These files are the repo-level smoke checks for the public SDK flows.

- `python_truth.py`: create, exec, inspect, destroy via the Python SDK
- `typescript_truth.mjs`: create, exec, inspect, destroy via the TypeScript SDK

They are intentionally plain scripts instead of a framework-specific test suite:
the goal is a low-friction "truth test" a maintainer can run against a real daemon.

Prerequisites:

- daemon running on `AGENTSANDBOX_DAEMON_URL` or `http://127.0.0.1:7847`
- local SDKs installed/built

Examples:

```bash
python tests/e2e/python_truth.py
node tests/e2e/typescript_truth.mjs
```
