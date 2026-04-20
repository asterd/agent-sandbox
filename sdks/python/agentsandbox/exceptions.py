"""Exception hierarchy for the AgentSandbox Python SDK.

Every error returned by the daemon HTTP API has this JSON envelope::

    { "error": { "code": "SANDBOX_NOT_FOUND", "message": "...", "details": {} } }

The SDK maps each `code` to a typed exception so callers can
``except SandboxNotFoundError`` without string matching.
"""

from __future__ import annotations

from typing import Any


class SandboxError(Exception):
    """Base class for every SDK-level error.

    Always carries the daemon's stable error ``code`` and any ``details``
    dict so callers that want to branch on specifics can still do it.
    """

    code: str = "SANDBOX_ERROR"

    def __init__(
        self,
        message: str,
        *,
        code: str | None = None,
        details: dict[str, Any] | None = None,
        status_code: int | None = None,
    ) -> None:
        super().__init__(message)
        self.message = message
        if code is not None:
            self.code = code
        self.details = details or {}
        self.status_code = status_code


class SandboxNotFoundError(SandboxError):
    """Daemon returned 404 / ``SANDBOX_NOT_FOUND``."""

    code = "SANDBOX_NOT_FOUND"


class SandboxExpiredError(SandboxError):
    """Daemon returned 410 / ``SANDBOX_EXPIRED``."""

    code = "SANDBOX_EXPIRED"


class SpecInvalidError(SandboxError):
    """Daemon returned 422 / ``SPEC_INVALID``."""

    code = "SPEC_INVALID"


class BackendUnavailableError(SandboxError):
    """Daemon returned 503 / ``BACKEND_UNAVAILABLE``."""

    code = "BACKEND_UNAVAILABLE"


class ExecTimeoutError(SandboxError):
    """Daemon returned 504 / ``EXEC_TIMEOUT``."""

    code = "EXEC_TIMEOUT"


class LeaseInvalidError(SandboxError):
    """Daemon returned 403 / ``LEASE_INVALID``."""

    code = "LEASE_INVALID"


class InternalDaemonError(SandboxError):
    """Daemon returned 5xx with a generic ``INTERNAL_ERROR``."""

    code = "INTERNAL_ERROR"


_CODE_TO_EXC: dict[str, type[SandboxError]] = {
    SandboxNotFoundError.code: SandboxNotFoundError,
    SandboxExpiredError.code: SandboxExpiredError,
    SpecInvalidError.code: SpecInvalidError,
    BackendUnavailableError.code: BackendUnavailableError,
    ExecTimeoutError.code: ExecTimeoutError,
    LeaseInvalidError.code: LeaseInvalidError,
    InternalDaemonError.code: InternalDaemonError,
}


def exception_for(
    code: str | None,
    message: str,
    *,
    details: dict[str, Any] | None = None,
    status_code: int | None = None,
) -> SandboxError:
    """Construct the typed exception matching a daemon error ``code``.

    Unknown codes fall back to the generic :class:`SandboxError` so forward
    compatibility is preserved when the daemon adds new codes.
    """

    cls = _CODE_TO_EXC.get(code or "", SandboxError)
    return cls(message, code=code, details=details, status_code=status_code)


__all__ = [
    "SandboxError",
    "SandboxNotFoundError",
    "SandboxExpiredError",
    "SpecInvalidError",
    "BackendUnavailableError",
    "ExecTimeoutError",
    "LeaseInvalidError",
    "InternalDaemonError",
    "exception_for",
]
