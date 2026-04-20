/**
 * End-to-end tests for the TypeScript SDK.
 *
 * These run against a real daemon with a healthy Docker backend. They are
 * auto-skipped when the daemon is unreachable so `npm run test` stays green
 * in environments without Docker; force them via
 * `AGENTSANDBOX_INTEGRATION=1`.
 */

import { describe, expect, it } from "vitest";
import { DEFAULT_DAEMON_URL, Sandbox } from "../src/index.js";

const DAEMON = process.env.AGENTSANDBOX_DAEMON_URL ?? DEFAULT_DAEMON_URL;
const FORCED = process.env.AGENTSANDBOX_INTEGRATION === "1";

async function probeDaemon(): Promise<boolean> {
  try {
    const res = await fetch(`${DAEMON}/v1/health`, {
      signal: AbortSignal.timeout(500),
    });
    return res.ok;
  } catch {
    return false;
  }
}

const daemonReady = await probeDaemon();

if (FORCED && !daemonReady) {
  throw new Error(`Integration forzata ma daemon non raggiungibile su ${DAEMON}`);
}

if (!daemonReady && !FORCED) {
  // eslint-disable-next-line no-console
  console.info(`[integration] daemon non raggiungibile su ${DAEMON}: test skippati`);
}

(daemonReady ? describe : describe.skip)("integration", () => {
  it("esegue un comando in una sandbox reale", async () => {
    await using sb = await Sandbox.create({
      runtime: "python",
      ttl: 60,
      daemonUrl: DAEMON,
    });
    const result = await sb.exec("echo 'hello from sandbox'");
    expect(result.exit_code).toBe(0);
    expect(result.stdout).toContain("hello from sandbox");
  });

  it("cattura exit code non-zero senza lanciare", async () => {
    await using sb = await Sandbox.create({
      runtime: "shell",
      ttl: 60,
      daemonUrl: DAEMON,
    });
    const result = await sb.exec("exit 42");
    expect(result.exit_code).toBe(42);
  });

  it("distrugge la sandbox all'uscita del blocco `await using`", async () => {
    let id: string | undefined;
    {
      await using sb = await Sandbox.create({
        runtime: "python",
        ttl: 60,
        daemonUrl: DAEMON,
      });
      id = sb.sandboxId;
      expect(id).toBeDefined();
    }
    const res = await fetch(`${DAEMON}/v1/sandboxes/${id}`);
    expect(res.status).toBe(404);
  });
});
