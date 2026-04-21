# TypeScript Dependency Auditor

This example copies a `package.json` into a sandbox, installs dependencies, runs `npm audit`, and asks Claude for a short summary.

## What it demonstrates

- local TypeScript SDK usage
- sandboxed dependency installation
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

## Expected output

See [expected_output.txt](expected_output.txt). Vulnerability counts, summary wording, and timings are dynamic because `npm audit` and the LLM output change over time.

## Troubleshooting

- `ANTHROPIC_API_KEY non impostata`
  Export the API key before running `npm run start`.
- `Cannot find package 'agentsandbox'`
  Run `npm install` in this example directory so the local workspace SDK is linked.
- `All connection attempts failed`
  The daemon is not running on `http://127.0.0.1:7847`; start it from the repository root before launching the auditor.
