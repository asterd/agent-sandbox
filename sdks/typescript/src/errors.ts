/**
 * Exception hierarchy for the AgentSandbox TypeScript SDK.
 *
 * Every error returned by the daemon HTTP API has this JSON envelope:
 *
 *     { "error": { "code": "SANDBOX_NOT_FOUND", "message": "...", "details": {} } }
 *
 * The SDK maps each `code` to a typed subclass so callers can narrow with
 * `instanceof SandboxNotFoundError` without string matching.
 */

export type ErrorDetails = Record<string, unknown>;

export interface SandboxErrorOptions {
  code?: string;
  details?: ErrorDetails;
  statusCode?: number;
  cause?: unknown;
}

export class SandboxError extends Error {
  static readonly code: string = "SANDBOX_ERROR";

  readonly code: string;
  readonly details: ErrorDetails;
  readonly statusCode?: number;

  constructor(message: string, options: SandboxErrorOptions = {}) {
    super(message, options.cause !== undefined ? { cause: options.cause } : undefined);
    this.name = new.target.name;
    this.code = options.code ?? (new.target as typeof SandboxError).code;
    this.details = options.details ?? {};
    this.statusCode = options.statusCode;
  }
}

export class SandboxNotFoundError extends SandboxError {
  static override readonly code = "SANDBOX_NOT_FOUND";
}

export class SandboxExpiredError extends SandboxError {
  static override readonly code = "SANDBOX_EXPIRED";
}

export class SpecInvalidError extends SandboxError {
  static override readonly code = "SPEC_INVALID";
}

export class BackendUnavailableError extends SandboxError {
  static override readonly code = "BACKEND_UNAVAILABLE";
}

export class ExecTimeoutError extends SandboxError {
  static override readonly code = "EXEC_TIMEOUT";
}

export class LeaseInvalidError extends SandboxError {
  static override readonly code = "LEASE_INVALID";
}

export class InternalDaemonError extends SandboxError {
  static override readonly code = "INTERNAL_ERROR";
}

const CODE_TO_CLASS: Record<string, typeof SandboxError> = {
  [SandboxNotFoundError.code]: SandboxNotFoundError,
  [SandboxExpiredError.code]: SandboxExpiredError,
  [SpecInvalidError.code]: SpecInvalidError,
  [BackendUnavailableError.code]: BackendUnavailableError,
  [ExecTimeoutError.code]: ExecTimeoutError,
  [LeaseInvalidError.code]: LeaseInvalidError,
  [InternalDaemonError.code]: InternalDaemonError,
};

/**
 * Build the typed error matching a daemon error `code`. Unknown codes fall
 * back to the base `SandboxError` so forward compatibility is preserved when
 * the daemon introduces new codes.
 */
export function exceptionFor(
  code: string | null | undefined,
  message: string,
  options: Omit<SandboxErrorOptions, "code"> = {},
): SandboxError {
  const cls = (code && CODE_TO_CLASS[code]) || SandboxError;
  return new cls(message, { ...options, code: code ?? undefined });
}
