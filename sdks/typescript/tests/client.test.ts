/**
 * Unit tests for the Sandbox client. These do NOT require a running daemon:
 * they stub `fetch` and exercise transport shape, header propagation,
 * response parsing, error mapping, and teardown behaviour.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import {
  LeaseInvalidError,
  Sandbox,
  SandboxError,
  SandboxExpiredError,
  SandboxNotFoundError,
  SpecInvalidError,
  VERSION,
} from "../src/index.js";

const DAEMON = "http://127.0.0.1:7847";
const SANDBOX_ID = "sb-1";
const LEASE = "lease-x";

type Handler = (url: string, init: RequestInit) => Response | Promise<Response>;

function mockFetch(handler: Handler): typeof fetch {
  return vi.fn(async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = typeof input === "string" ? input : input.toString();
    return handler(url, init);
  }) as unknown as typeof fetch;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

const createBody = {
  sandbox_id: SANDBOX_ID,
  lease_token: LEASE,
  status: "running",
  expires_at: "2099-01-01T00:00:00+00:00",
  backend: "docker",
};

function errorBody(code: string, message: string) {
  return { error: { code, message, details: {} } };
}

/** Build a lifecycle mock that handles create + destroy and delegates other
 *  routes to `extra`. */
function lifecycleFetch(extra: Handler = () => new Response(null, { status: 404 })) {
  return mockFetch(async (url, init) => {
    const method = (init.method ?? "GET").toUpperCase();
    if (method === "POST" && url === `${DAEMON}/v1/sandboxes`) {
      return json(201, createBody);
    }
    if (method === "DELETE" && url === `${DAEMON}/v1/sandboxes/${SANDBOX_ID}`) {
      return new Response(null, { status: 204 });
    }
    return extra(url, init);
  });
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("SDK metadata", () => {
  it("esporta la versione", () => {
    expect(VERSION).toBe("0.1.0");
  });
});

describe("Sandbox.create + destroy", () => {
  it("fa POST /v1/sandboxes e DELETE al teardown con lease header", async () => {
    const fetchImpl = lifecycleFetch();
    const sb = await Sandbox.create({ runtime: "python", ttl: 60, fetch: fetchImpl });
    expect(sb.sandboxId).toBe(SANDBOX_ID);

    await sb.destroy();
    expect(sb.sandboxId).toBeUndefined();

    const calls = (fetchImpl as unknown as ReturnType<typeof vi.fn>).mock.calls;
    const deleteCall = calls.find(([, init]) => init?.method === "DELETE");
    expect(deleteCall).toBeDefined();
    const headers = (deleteCall?.[1]?.headers ?? {}) as Record<string, string>;
    expect(headers["X-Lease-Token"]).toBe(LEASE);
  });

  it("supporta `await using` via Symbol.asyncDispose", async () => {
    const fetchImpl = lifecycleFetch();
    {
      await using sb = await Sandbox.create({ runtime: "python", fetch: fetchImpl });
      expect(sb.sandboxId).toBe(SANDBOX_ID);
    }
    const calls = (fetchImpl as unknown as ReturnType<typeof vi.fn>).mock.calls;
    expect(calls.some(([, init]) => init?.method === "DELETE")).toBe(true);
  });

  it("omette network e runtime.env quando non configurati", async () => {
    let createdBody: unknown;
    const fetchImpl = mockFetch(async (url, init) => {
      if (init.method === "POST" && url === `${DAEMON}/v1/sandboxes`) {
        createdBody = JSON.parse(init.body as string);
        return json(201, createBody);
      }
      if (init.method === "DELETE") return new Response(null, { status: 204 });
      return new Response(null, { status: 404 });
    });

    const sb = await Sandbox.create({ runtime: "python", fetch: fetchImpl });
    await sb.destroy();

    const spec = (createdBody as { spec: Record<string, unknown> }).spec;
    expect(spec.network).toBeUndefined();
    expect((spec.runtime as Record<string, unknown>).env).toBeUndefined();
    expect((spec.runtime as Record<string, unknown>).preset).toBe("python");
  });

  it("include network.egress quando egress non vuoto", async () => {
    let body: unknown;
    const fetchImpl = mockFetch(async (url, init) => {
      if (init.method === "POST" && url === `${DAEMON}/v1/sandboxes`) {
        body = JSON.parse(init.body as string);
        return json(201, createBody);
      }
      return new Response(null, { status: 204 });
    });
    const sb = await Sandbox.create({
      runtime: "python",
      egress: ["pypi.org"],
      fetch: fetchImpl,
    });
    await sb.destroy();
    const spec = (body as { spec: Record<string, unknown> }).spec;
    expect(spec.network).toEqual({
      egress: { allow: ["pypi.org"], denyByDefault: true },
    });
  });

  it("converte secrets in valueFrom.envRef (mai valori)", async () => {
    let body: unknown;
    const fetchImpl = mockFetch(async (url, init) => {
      if (init.method === "POST" && url === `${DAEMON}/v1/sandboxes`) {
        body = JSON.parse(init.body as string);
        return json(201, createBody);
      }
      return new Response(null, { status: 204 });
    });
    const sb = await Sandbox.create({
      runtime: "python",
      secrets: { API_KEY: "HOST_API_KEY_VAR" },
      fetch: fetchImpl,
    });
    await sb.destroy();
    const spec = (body as { spec: { secrets: unknown } }).spec;
    expect(spec.secrets).toEqual([
      { name: "API_KEY", valueFrom: { envRef: "HOST_API_KEY_VAR" } },
    ]);
  });

  it("usa image invece di preset quando fornita", async () => {
    let body: unknown;
    const fetchImpl = mockFetch(async (url, init) => {
      if (init.method === "POST" && url === `${DAEMON}/v1/sandboxes`) {
        body = JSON.parse(init.body as string);
        return json(201, createBody);
      }
      return new Response(null, { status: 204 });
    });
    const sb = await Sandbox.create({
      runtime: "custom",
      image: "my-registry/img:1",
      fetch: fetchImpl,
    });
    await sb.destroy();
    const runtime = (body as { spec: { runtime: Record<string, unknown> } }).spec.runtime;
    expect(runtime.image).toBe("my-registry/img:1");
    expect(runtime.preset).toBeUndefined();
  });
});

describe("Sandbox.exec", () => {
  it("invia command, lease header e parse della response", async () => {
    const fetchImpl = lifecycleFetch(async (url, init) => {
      if (
        init.method === "POST" &&
        url === `${DAEMON}/v1/sandboxes/${SANDBOX_ID}/exec`
      ) {
        return json(200, {
          stdout: "hi\n",
          stderr: "",
          exit_code: 0,
          duration_ms: 42,
        });
      }
      return new Response(null, { status: 404 });
    });

    const sb = await Sandbox.create({ runtime: "python", fetch: fetchImpl });
    const result = await sb.exec("echo hi");
    await sb.destroy();

    expect(result.stdout).toBe("hi\n");
    expect(result.exit_code).toBe(0);
    expect(result.duration_ms).toBe(42);

    const calls = (fetchImpl as unknown as ReturnType<typeof vi.fn>).mock.calls;
    const execCall = calls.find(([u, init]) =>
      String(u).endsWith("/exec") && init?.method === "POST",
    );
    expect(execCall).toBeDefined();
    const [, execInit] = execCall!;
    expect(JSON.parse(execInit.body as string)).toEqual({ command: "echo hi" });
    const headers = execInit.headers as Record<string, string>;
    expect(headers["X-Lease-Token"]).toBe(LEASE);
  });

  it("exit_code != 0 non lancia eccezione", async () => {
    const fetchImpl = lifecycleFetch(async () =>
      json(200, { stdout: "", stderr: "nope", exit_code: 42, duration_ms: 1 }),
    );
    const sb = await Sandbox.create({ runtime: "shell", fetch: fetchImpl });
    const result = await sb.exec("exit 42");
    await sb.destroy();
    expect(result.exit_code).toBe(42);
  });

  it("exec prima di create lancia SandboxError", async () => {
    const sb = new Sandbox({ runtime: "python", fetch: mockFetch(() => new Response()) });
    await expect(sb.exec("echo")).rejects.toBeInstanceOf(SandboxError);
  });
});

describe("Sandbox.inspect", () => {
  it("ritorna SandboxInfo tipizzata", async () => {
    const fetchImpl = lifecycleFetch(async (url, init) => {
      if (init.method === "GET" && url === `${DAEMON}/v1/sandboxes/${SANDBOX_ID}`) {
        return json(200, {
          sandbox_id: SANDBOX_ID,
          status: "running",
          backend: "docker",
          created_at: "2099-01-01T00:00:00+00:00",
          expires_at: "2099-01-01T00:05:00+00:00",
          error_message: null,
        });
      }
      return new Response(null, { status: 404 });
    });

    const sb = await Sandbox.create({ runtime: "python", fetch: fetchImpl });
    const info = await sb.inspect();
    await sb.destroy();

    expect(info.sandbox_id).toBe(SANDBOX_ID);
    expect(info.backend).toBe("docker");
    expect(info.error_message).toBeNull();
  });
});

describe("error mapping", () => {
  const cases: Array<[number, string, typeof SandboxError]> = [
    [404, "SANDBOX_NOT_FOUND", SandboxNotFoundError],
    [410, "SANDBOX_EXPIRED", SandboxExpiredError],
    [422, "SPEC_INVALID", SpecInvalidError],
    [403, "LEASE_INVALID", LeaseInvalidError],
  ];

  for (const [status, code, cls] of cases) {
    it(`${status}/${code} → ${cls.name}`, async () => {
      const fetchImpl = mockFetch(() =>
        json(status, errorBody(code, `boom-${code}`)),
      );
      await expect(
        Sandbox.create({ runtime: "python", fetch: fetchImpl }),
      ).rejects.toBeInstanceOf(cls);
    });
  }

  it("codice sconosciuto ricade su SandboxError ma preserva il code", async () => {
    const fetchImpl = mockFetch(() =>
      json(500, errorBody("FUTURE_CODE", "qualcosa di nuovo")),
    );
    await expect(
      Sandbox.create({ runtime: "python", fetch: fetchImpl }),
    ).rejects.toMatchObject({
      name: "SandboxError",
      code: "FUTURE_CODE",
      statusCode: 500,
    });
  });

  it("destroy swallow-a gli errori di rete", async () => {
    const fetchImpl = mockFetch(async (url, init) => {
      if (init.method === "POST") return json(201, createBody);
      if (init.method === "DELETE") throw new Error("daemon gone");
      return new Response(null, { status: 404 });
    });
    const sb = await Sandbox.create({ runtime: "python", fetch: fetchImpl });
    await expect(sb.destroy()).resolves.toBeUndefined();
  });

  it("corpo non-JSON produce SandboxError generico", async () => {
    const fetchImpl = mockFetch(
      () => new Response("internal oops", { status: 500 }),
    );
    await expect(
      Sandbox.create({ runtime: "python", fetch: fetchImpl }),
    ).rejects.toMatchObject({ name: "SandboxError", statusCode: 500 });
  });
});
