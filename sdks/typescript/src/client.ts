/**
 * Async client for the AgentSandbox daemon.
 *
 * @example
 * ```ts
 * import { Sandbox } from "agentsandbox";
 *
 * await using sb = await Sandbox.create({ runtime: "python", ttl: 300 });
 * const result = await sb.exec("python -c 'print(42)'");
 * console.log(result.stdout);
 * ```
 *
 * Transport failures (fetch rejections, aborted requests) bubble up wrapped
 * in {@link SandboxError}; daemon error envelopes map to the typed hierarchy
 * in `./errors`.
 */

import {
  API_VERSION,
  DEFAULT_DAEMON_URL,
  KIND,
  LEASE_HEADER,
  type CreateResponse,
  type ExecResult,
  type SandboxConfig,
  type SandboxInfo,
  type SandboxOptions,
} from "./types.js";
import { SandboxError, exceptionFor } from "./errors.js";

export class Sandbox {
  readonly #config: SandboxConfig;
  readonly #fetch: typeof fetch;
  #sandboxId?: string;
  #leaseToken?: string;

  constructor(options: SandboxOptions) {
    this.#config = {
      runtime: options.runtime,
      image: options.image,
      ttl: options.ttl ?? 300,
      egress: [...(options.egress ?? [])],
      memoryMb: options.memoryMb ?? 512,
      cpuMillicores: options.cpuMillicores ?? 1000,
      diskMb: options.diskMb ?? 1024,
      env: { ...(options.env ?? {}) },
      secrets: { ...(options.secrets ?? {}) },
      secretFiles: { ...(options.secretFiles ?? {}) },
      workingDir: options.workingDir,
      preferWarm: options.preferWarm ?? false,
      daemonUrl: options.daemonUrl ?? DEFAULT_DAEMON_URL,
      fetch: options.fetch,
    };
    this.#fetch = options.fetch ?? fetch;
  }

  /** Eagerly create the backing sandbox. Preferred entry point for `await using`. */
  static async create(options: SandboxOptions): Promise<Sandbox> {
    const sb = new Sandbox(options);
    await sb.#create();
    return sb;
  }

  get sandboxId(): string | undefined {
    return this.#sandboxId;
  }

  get config(): Readonly<SandboxConfig> {
    return this.#config;
  }

  /**
   * Run `command` inside the sandbox and return its captured output.
   *
   * A non-zero `exit_code` is NOT an exception — inspect it on the result.
   * Exceptions are raised only when the daemon or backend itself fails.
   */
  async exec(command: string): Promise<ExecResult> {
    const id = this.#requireActive();
    const response = await this.#fetch(
      `${this.#config.daemonUrl}/v1/sandboxes/${id}/exec`,
      {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          [LEASE_HEADER]: this.#leaseToken ?? "",
        },
        body: JSON.stringify({ command }),
      },
    );
    return (await this.#handleResponse(response)) as ExecResult;
  }

  /** Fetch the current sandbox state from the daemon. */
  async inspect(): Promise<SandboxInfo> {
    const id = this.#requireActive();
    const response = await this.#fetch(
      `${this.#config.daemonUrl}/v1/sandboxes/${id}`,
      { method: "GET" },
    );
    return (await this.#handleResponse(response)) as SandboxInfo;
  }

  /**
   * Best-effort teardown. Network errors during destroy are swallowed so
   * they don't mask an unrelated exception already in flight.
   */
  async destroy(): Promise<void> {
    if (!this.#sandboxId) return;
    const id = this.#sandboxId;
    const token = this.#leaseToken;
    this.#sandboxId = undefined;
    this.#leaseToken = undefined;
    try {
      const response = await this.#fetch(
        `${this.#config.daemonUrl}/v1/sandboxes/${id}`,
        {
          method: "DELETE",
          headers: { [LEASE_HEADER]: token ?? "" },
        },
      );
      // 204 No Content is the happy path. We don't throw on 4xx/5xx during
      // teardown because the sandbox is already gone from the caller's PoV.
      if (!response.ok && response.status !== 204) {
        // Drain body to free the connection; ignore parse errors.
        await response.text().catch(() => "");
      }
    } catch {
      // network error during teardown — nothing actionable for the caller.
    }
  }

  /** Support for `await using` (TC39 explicit resource management). */
  async [Symbol.asyncDispose](): Promise<void> {
    await this.destroy();
  }

  // ---------- internals ----------

  #requireActive(): string {
    if (!this.#sandboxId) {
      throw new SandboxError(
        "Sandbox non inizializzata. Usa 'await Sandbox.create(...)' o 'await using sb = ...'.",
      );
    }
    return this.#sandboxId;
  }

  async #create(): Promise<void> {
    const response = await this.#fetch(`${this.#config.daemonUrl}/v1/sandboxes`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(this.#buildSpec()),
    });
    const data = (await this.#handleResponse(response)) as CreateResponse;
    this.#sandboxId = data.sandbox_id;
    this.#leaseToken = data.lease_token;
  }

  /**
   * Render the config as the JSON body expected by `POST /v1/sandboxes`.
   *
   * Empty/undefined fields are omitted rather than serialised as `null` so
   * the daemon's `deny_unknown_fields` / `MissingRuntime` guards stay happy.
   */
  #buildSpec(): Record<string, unknown> {
    const runtime: Record<string, unknown> = {};
    if (this.#config.image) {
      runtime.image = this.#config.image;
    } else {
      runtime.preset = this.#config.runtime;
    }
    if (Object.keys(this.#config.env).length > 0) {
      runtime.env = this.#config.env;
    }
    if (this.#config.workingDir) {
      runtime.workingDir = this.#config.workingDir;
    }

    const specBody: Record<string, unknown> = {
      runtime,
      resources: {
        memoryMb: this.#config.memoryMb,
        cpuMillicores: this.#config.cpuMillicores,
        diskMb: this.#config.diskMb,
      },
      ttlSeconds: this.#config.ttl,
    };

    if (this.#config.egress.length > 0) {
      specBody.network = {
        egress: { allow: this.#config.egress, denyByDefault: true },
      };
    }

    if (Object.keys(this.#config.secrets).length > 0) {
      specBody.secrets = Object.entries(this.#config.secrets).map(
        ([name, hostVar]) => ({
          name,
          valueFrom: { envRef: hostVar },
        }),
      );
    }
    if (Object.keys(this.#config.secretFiles).length > 0) {
      const secretFiles = Object.entries(this.#config.secretFiles).map(
        ([name, path]) => ({
          name,
          valueFrom: { file: path },
        }),
      );
      specBody.secrets = Array.isArray(specBody.secrets)
        ? [...(specBody.secrets as object[]), ...secretFiles]
        : secretFiles;
    }
    if (this.#config.preferWarm) {
      specBody.scheduling = { preferWarm: true };
    }

    return {
      apiVersion: API_VERSION,
      kind: KIND,
      metadata: {},
      spec: specBody,
    };
  }

  async #handleResponse(response: Response): Promise<unknown> {
    if (response.status === 204) return {};
    if (response.ok) {
      try {
        return await response.json();
      } catch (cause) {
        throw new SandboxError(
          `Risposta daemon non JSON (${response.status})`,
          { statusCode: response.status, cause },
        );
      }
    }

    const { code, message, details } = await parseErrorEnvelope(response);
    throw exceptionFor(code, message, {
      details,
      statusCode: response.status,
    });
  }
}

async function parseErrorEnvelope(response: Response): Promise<{
  code: string | null;
  message: string;
  details: Record<string, unknown>;
}> {
  const fallback = response.statusText || `HTTP ${response.status}`;
  let payload: unknown;
  try {
    payload = await response.json();
  } catch {
    const text = await response.text().catch(() => "");
    return { code: null, message: text || fallback, details: {} };
  }

  if (!payload || typeof payload !== "object") {
    return { code: null, message: fallback, details: {} };
  }

  const envelope = (payload as { error?: unknown }).error;
  if (!envelope || typeof envelope !== "object") {
    return { code: null, message: fallback, details: {} };
  }

  const e = envelope as {
    code?: unknown;
    message?: unknown;
    details?: unknown;
  };
  const rawDetails = e.details;
  const details =
    rawDetails && typeof rawDetails === "object" && !Array.isArray(rawDetails)
      ? (rawDetails as Record<string, unknown>)
      : {};
  return {
    code: typeof e.code === "string" ? e.code : null,
    message: typeof e.message === "string" ? e.message : fallback,
    details,
  };
}
