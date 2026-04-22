# TypeScript Dependency Auditor

This example copies a `package.json` into a sandbox, installs dependencies, runs `npm audit`, and asks an OpenAI-compatible model for a short summary.

## What it demonstrates

- local TypeScript SDK usage
- sandboxed dependency installation
- CI-friendly exit code when critical vulnerabilities are found

## Setup

From this directory:

```bash
npm install
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
npm run start -- sample/package.json
```

Type-check the example:

```bash
npm run check
```

## Sample output

The exact vulnerability counts, summary wording, sandbox id, and timings are dynamic because `npm audit` and the model output change over time. The stable structure looks like this:

```text
Auditing: sample/package.json
Using provider: openai
Using model: gpt-4.1-mini
--------------------------------------------------
Creating isolated sandbox...
Installing dependencies inside sandbox...
Running npm audit...
Requesting summary from LLM...

Final report
--------------------------------------------------
Total vulnerabilities: <number>
Critical: <number>
High: <number>
Moderate: <number>
Low: <number>

LLM summary:
<short Italian summary>
```

The process exits with code `1` when `Critical` is greater than zero.

Note:

- the current alpha keeps this example on open egress for reliability with the stock Node preset
- the planned stable replacement for filtered egress is the proxy L4 path in `ROADMAP_STABLE.md` FASE C
