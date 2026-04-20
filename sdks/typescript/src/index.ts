/**
 * AgentSandbox TypeScript SDK.
 *
 * @example
 * ```ts
 * import { Sandbox } from "agentsandbox";
 *
 * await using sb = await Sandbox.create({ runtime: "python", ttl: 300 });
 * const result = await sb.exec("python -c 'print(42)'");
 * console.log(result.stdout);
 * ```
 */

export { Sandbox } from "./client.js";
export {
  BackendUnavailableError,
  ExecTimeoutError,
  InternalDaemonError,
  LeaseInvalidError,
  SandboxError,
  SandboxExpiredError,
  SandboxNotFoundError,
  SpecInvalidError,
  exceptionFor,
} from "./errors.js";
export type { ErrorDetails, SandboxErrorOptions } from "./errors.js";
export {
  API_VERSION,
  DEFAULT_DAEMON_URL,
  KIND,
  LEASE_HEADER,
} from "./types.js";
export type {
  CreateResponse,
  ExecResult,
  SandboxConfig,
  SandboxInfo,
  SandboxOptions,
} from "./types.js";

export const VERSION = "0.1.0";
