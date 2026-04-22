import "dotenv/config";
import { Sandbox } from "agentsandbox";
import { readFileSync, existsSync } from "node:fs";
import { basename } from "node:path";
import OpenAI from "openai";

const DEFAULT_PROVIDER = "openai";
const DEFAULT_BASE_URL = "https://api.openai.com/v1";
const DEFAULT_MODEL = "gpt-4.1-mini";

interface SeverityCounts {
  critical: number;
  high: number;
  moderate: number;
  low: number;
  info: number;
  total: number;
}

function loadLlmConfig(): { provider: string; model: string; client: OpenAI } {
  const provider = (process.env.AGENTSANDBOX_LLM_PROVIDER ?? DEFAULT_PROVIDER).trim();
  const baseURL = (process.env.AGENTSANDBOX_LLM_BASE_URL ?? DEFAULT_BASE_URL).trim();
  const apiKey = (process.env.AGENTSANDBOX_LLM_API_KEY ?? "").trim();
  const model = (process.env.AGENTSANDBOX_LLM_MODEL ?? DEFAULT_MODEL).trim();

  if (!apiKey) {
    throw new Error("AGENTSANDBOX_LLM_API_KEY non impostata");
  }

  return {
    provider,
    model,
    client: new OpenAI({ apiKey, baseURL }),
  };
}

function extractText(completion: Awaited<ReturnType<OpenAI["chat"]["completions"]["create"]>>): string {
  return completion.choices[0]?.message?.content?.trim() ?? "";
}

function parseCounts(rawAuditOutput: string): SeverityCounts {
  const empty: SeverityCounts = {
    critical: 0,
    high: 0,
    moderate: 0,
    low: 0,
    info: 0,
    total: 0,
  };

  let parsed: unknown;
  try {
    parsed = JSON.parse(rawAuditOutput);
  } catch {
    return empty;
  }

  if (!parsed || typeof parsed !== "object") {
    return empty;
  }

  const metadata = (parsed as { metadata?: { vulnerabilities?: Record<string, unknown> } }).metadata;
  const vulnerabilities = metadata?.vulnerabilities;
  if (!vulnerabilities || typeof vulnerabilities !== "object") {
    return empty;
  }

  const critical = Number(vulnerabilities.critical ?? 0);
  const high = Number(vulnerabilities.high ?? 0);
  const moderate = Number(vulnerabilities.moderate ?? 0);
  const low = Number(vulnerabilities.low ?? 0);
  const info = Number(vulnerabilities.info ?? 0);
  const total =
    Number(vulnerabilities.total ?? 0) || critical + high + moderate + low + info;

  return { critical, high, moderate, low, info, total };
}

async function writeFileToSandbox(
  sandbox: Sandbox,
  guestPath: string,
  contents: string,
): Promise<void> {
  const encoded = Buffer.from(contents, "utf-8").toString("base64");
  const command = [
    "node -e",
    JSON.stringify(
      [
        "const fs = require('fs');",
        "fs.mkdirSync('/workspace', { recursive: true });",
        `fs.writeFileSync(${JSON.stringify(guestPath)}, Buffer.from(${JSON.stringify(encoded)}, 'base64'));`,
      ].join(" "),
    ),
  ].join(" ");

  const result = await sandbox.exec(command);
  if (result.exit_code !== 0) {
    throw new Error(`scrittura file sandbox fallita:\n${result.stderr || result.stdout}`);
  }
}

async function requestSummary(client: OpenAI, auditOutput: string, model: string): Promise<string> {
  const message = await client.chat.completions.create({
    model,
    max_tokens: 400,
    messages: [
      {
        role: "system",
        content:
          "Riassumi audit tecnici in italiano con tono operativo e conciso.",
      },
      {
        role: "user",
        content:
          "Riassumi in italiano questo report di npm audit in 3-5 righe. " +
          "Indica quante vulnerabilita ci sono, quali severita spiccano e la prima azione consigliata.\n\n" +
          `\`\`\`json\n${auditOutput.slice(0, 5000)}\n\`\`\``,
      },
    ],
  });

  const summary = extractText(message);
  if (!summary) {
    throw new Error("Il provider LLM ha risposto senza testo");
  }
  return summary;
}

async function auditDependencies(packageJsonPath: string): Promise<{
  counts: SeverityCounts;
  summary: string;
  rawAuditOutput: string;
}> {
  const { provider, model, client } = loadLlmConfig();
  const packageJson = readFileSync(packageJsonPath, "utf-8");

  console.log(`Auditing: ${packageJsonPath}`);
  console.log(`Using provider: ${provider}`);
  console.log(`Using model: ${model}`);
  console.log("-".repeat(50));
  console.log("Creating isolated sandbox...");

  await using sandbox = await Sandbox.create({
    runtime: "node",
    ttl: 180,
    memoryMb: 512,
  });

  await writeFileToSandbox(sandbox, "/workspace/package.json", packageJson);

  console.log("Installing dependencies inside sandbox...");
  const installResult = await sandbox.exec(
    "cd /workspace && npm install --ignore-scripts --no-fund --no-update-notifier 2>&1",
  );
  if (installResult.exit_code !== 0) {
    throw new Error(`npm install fallito:\n${installResult.stdout}${installResult.stderr}`);
  }

  console.log("Running npm audit...");
  const auditResult = await sandbox.exec("cd /workspace && npm audit --json 2>&1 || true");
  const counts = parseCounts(auditResult.stdout);

  console.log("Requesting summary from LLM...");
  const summary = await requestSummary(client, auditResult.stdout, model);

  return {
    counts,
    summary,
    rawAuditOutput: auditResult.stdout,
  };
}

async function main(): Promise<number> {
  const packageJsonPath = process.argv[2] ?? "sample/package.json";
  if (!existsSync(packageJsonPath)) {
    console.error(`File non trovato: ${packageJsonPath}`);
    return 1;
  }

  try {
    const { counts, summary } = await auditDependencies(packageJsonPath);

    console.log("\nFinal report");
    console.log("-".repeat(50));
    console.log(`Target: ${basename(packageJsonPath)}`);
    console.log(`Total vulnerabilities: ${counts.total}`);
    console.log(`Critical: ${counts.critical}`);
    console.log(`High: ${counts.high}`);
    console.log(`Moderate: ${counts.moderate}`);
    console.log(`Low: ${counts.low}`);
    console.log(`Info: ${counts.info}`);
    console.log("\nLLM summary:");
    console.log(summary);

    return counts.critical > 0 ? 1 : 0;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    console.error(`Error: ${message}`);
    return 1;
  }
}

process.exit(await main());
