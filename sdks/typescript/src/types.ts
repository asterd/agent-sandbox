/**
 * Public value objects. They mirror the daemon contract documented in
 * `docs/api-http-v1.md`; keep them in sync when the API evolves.
 */

/** Outcome of a single {@link Sandbox.exec} call. */
export interface ExecResult {
  stdout: string;
  stderr: string;
  exit_code: number;
  duration_ms: number;
}

export interface ExecStreamEvent {
  event: string;
  chunk?: string;
  exit_code?: number;
  duration_ms?: number;
  sandbox_id?: string;
  backend?: string;
}

/** Inspect response for a sandbox (mirrors daemon `InspectResponse`). */
export interface SandboxInfo {
  sandbox_id: string;
  status: string;
  backend: string;
  created_at: string;
  expires_at: string;
  error_message: string | null;
}

/** Response body of `POST /v1/sandboxes`. */
export interface CreateResponse {
  sandbox_id: string;
  lease_token: string;
  status: string;
  expires_at: string;
  backend: string;
}

/**
 * User-facing configuration accepted by the {@link Sandbox} constructor.
 *
 * `secrets` is a mapping `{ guestEnvName: hostEnvVarName }`: the SDK never
 * receives the resolved value — the daemon resolves it via `valueFrom.envRef`.
 * This preserves the invariant that secret values never cross the SDK.
 */
export interface SandboxConfig {
  runtime: string;
  image?: string;
  ttl: number;
  egress: string[];
  memoryMb: number;
  cpuMillicores: number;
  diskMb: number;
  env: Record<string, string>;
  secrets: Record<string, string>;
  secretFiles: Record<string, string>;
  workingDir?: string;
  preferWarm: boolean;
  backend?: string;
  extensions?: Record<string, unknown>;
  daemonUrl: string;
  fetch?: typeof fetch;
}

/** Shape accepted by `new Sandbox(...)` and `Sandbox.create(...)`. */
export type SandboxOptions = Partial<Omit<SandboxConfig, "runtime">> & {
  runtime: string;
};

export const API_VERSION = "sandbox.ai/v1";
export const KIND = "Sandbox";
export const DEFAULT_DAEMON_URL = "http://127.0.0.1:7847";
export const LEASE_HEADER = "X-Lease-Token";
