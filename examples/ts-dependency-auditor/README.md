# TypeScript Dependency Auditor

This example copies a `package.json` into a sandbox, installs dependencies with a restricted egress allowlist, runs `npm audit`, and asks Claude for a short summary.

## What it demonstrates

- local TypeScript SDK usage
- sandboxed dependency installation
- `network.egress` allowlist for npm registries
- CI-friendly exit code when critical vulnerabilities are found

## Setup

From this directory:

```bash
npm install
export ANTHROPIC_API_KEY=your_key_here
```

In another terminal, start the daemon from the repository root:

```bash
cargo run -p agentsandbox-daemon
```

Run the example:

```bash
npm run start -- sample/package.json
```

Type-check the example:

```bash
npm run check
```

## Sample output

The exact vulnerability counts, summary wording, sandbox id, and timings are dynamic because `npm audit` and Claude output change over time. The stable structure looks like this:

```text
Auditing: sample/package.json
Using model: claude-sonnet-4-20250514
--------------------------------------------------
Creating isolated sandbox...
Installing dependencies inside sandbox...
Running npm audit...
Requesting summary from Claude...

Final report
--------------------------------------------------
Total vulnerabilities: <number>
Critical: <number>
High: <number>
Moderate: <number>
Low: <number>

Claude summary:
<short Italian summary>
```

The process exits with code `1` when `Critical` is greater than zero.
