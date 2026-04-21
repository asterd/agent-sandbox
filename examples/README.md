# AgentSandbox Examples

Examples in this directory are meant to be copied, modified, and run against a local `agentsandbox-daemon`.

## Available examples

| Example | SDK | What it demonstrates | Network policy |
| --- | --- | --- | --- |
| [python-code-review-agent](python-code-review-agent/README.md) | Python | Claude reviews a Python file, then the fixed code is executed inside a sandbox | No guest egress |
| [ts-dependency-auditor](ts-dependency-auditor/README.md) | TypeScript | Installs dependencies and runs `npm audit` inside a sandbox, then asks Claude for a short summary | Allowlist for npm registries |

## Common prerequisites

- Docker running locally
- local daemon started with `cargo run -p agentsandbox-daemon`
- `ANTHROPIC_API_KEY` exported in the host shell

## Notes

- These examples install the SDKs from the local workspace, not from PyPI or npm.
- Output examples in the per-example README files intentionally mark dynamic values such as durations, sandbox ids, vulnerability counts, and LLM wording.
