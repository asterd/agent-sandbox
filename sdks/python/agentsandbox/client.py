"""Async client for the AgentSandbox daemon.

The primary surface is :class:`Sandbox`, an async context manager:

.. code-block:: python

    from agentsandbox import Sandbox

    async with Sandbox(runtime="python", ttl=300) as sb:
        result = await sb.exec("python -c 'print(42)'")
        print(result.stdout)

Errors from the daemon are mapped to the typed hierarchy in
:mod:`agentsandbox.exceptions`; transport failures (``httpx`` timeouts,
connection refused) bubble up as :class:`SandboxError`.
"""

from __future__ import annotations

import logging
from copy import deepcopy
from typing import Any, AsyncIterator

import httpx

from .exceptions import SandboxError, exception_for
from .models import ExecResult, ExecStreamEvent, SandboxConfig, SandboxInfo

DEFAULT_DAEMON_URL = "http://127.0.0.1:7847"
DEFAULT_TIMEOUT = 60.0
LEASE_HEADER = "X-Lease-Token"

_log = logging.getLogger(__name__)


class Sandbox:
    """High-level sandbox client.

    Basic use:

    .. code-block:: python

        async with Sandbox(runtime="python", ttl=300) as sb:
            await sb.exec("echo hi")

    Advanced use:

    .. code-block:: python

        async with Sandbox(
            runtime="python",
            ttl=300,
            egress=["pypi.org", "files.pythonhosted.org"],
            memory_mb=1024,
            secrets={"API_KEY": "HOST_API_KEY_VAR"},
        ) as sb:
            await sb.exec("pip install httpx")
            result = await sb.exec("python script.py")

    The instance is single-use: once the context manager exits, the backing
    sandbox is destroyed and :meth:`exec` will raise.
    """

    def __init__(
        self,
        runtime: str = "python",
        *,
        image: str | None = None,
        ttl: int = 300,
        egress: list[str] | None = None,
        memory_mb: int = 512,
        cpu_millicores: int = 1000,
        disk_mb: int = 1024,
        env: dict[str, str] | None = None,
        secrets: dict[str, str] | None = None,
        secret_files: dict[str, str] | None = None,
        working_dir: str | None = None,
        prefer_warm: bool = False,
        backend: str | None = None,
        extensions: dict[str, Any] | None = None,
        daemon_url: str = DEFAULT_DAEMON_URL,
        timeout: float = DEFAULT_TIMEOUT,
        client: httpx.AsyncClient | None = None,
    ) -> None:
        if extensions is not None and not backend:
            raise ValueError(
                "extensions richiede backend esplicito. "
                "Es: Sandbox(runtime='python', backend='docker', extensions={...})"
            )
        self._config = SandboxConfig(
            runtime=runtime,
            image=image,
            ttl=ttl,
            egress=list(egress or []),
            memory_mb=memory_mb,
            cpu_millicores=cpu_millicores,
            disk_mb=disk_mb,
            env=dict(env or {}),
            secrets=dict(secrets or {}),
            secret_files=dict(secret_files or {}),
            working_dir=working_dir,
            prefer_warm=prefer_warm,
            backend=backend,
            extensions=deepcopy(extensions) if extensions is not None else None,
        )
        self._sandbox_id: str | None = None
        self._lease_token: str | None = None
        self._owns_client = client is None
        self._client = client or httpx.AsyncClient(
            base_url=daemon_url, timeout=timeout
        )

    @property
    def sandbox_id(self) -> str | None:
        """The daemon-issued id, or ``None`` if not yet created."""
        return self._sandbox_id

    @property
    def config(self) -> SandboxConfig:
        return self._config

    async def __aenter__(self) -> "Sandbox":
        try:
            await self._create()
        except BaseException:
            # __aexit__ is not invoked when __aenter__ raises, so we must
            # close the owned client ourselves to avoid leaking it.
            if self._owns_client:
                await self._client.aclose()
            raise
        return self

    async def __aexit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        try:
            await self._destroy()
        finally:
            if self._owns_client:
                await self._client.aclose()

    async def exec(self, command: str) -> ExecResult:
        """Run ``command`` inside the sandbox and return its captured output.

        A non-zero exit code is NOT an exception — inspect
        :attr:`ExecResult.exit_code` / :attr:`ExecResult.success`. Exceptions
        are raised only when the daemon or backend itself fails.
        """

        sandbox_id = self._require_active()
        response = await self._client.post(
            f"/v1/sandboxes/{sandbox_id}/exec",
            json={"command": command},
            headers={LEASE_HEADER: self._lease_token or ""},
        )
        data = self._handle_response(response)
        return ExecResult(
            stdout=data["stdout"],
            stderr=data["stderr"],
            exit_code=int(data["exit_code"]),
            duration_ms=int(data["duration_ms"]),
        )

    async def inspect(self) -> SandboxInfo:
        """Fetch the current sandbox state from the daemon."""

        sandbox_id = self._require_active()
        response = await self._client.get(f"/v1/sandboxes/{sandbox_id}")
        data = self._handle_response(response)
        return SandboxInfo(
            sandbox_id=data["sandbox_id"],
            status=data["status"],
            backend=data["backend"],
            created_at=data["created_at"],
            expires_at=data["expires_at"],
            error_message=data.get("error_message"),
        )

    async def upload_file(self, path: str, content: bytes) -> None:
        sandbox_id = self._require_active()
        response = await self._client.post(
            f"/v1/sandboxes/{sandbox_id}/files",
            params={"path": path},
            content=content,
            headers={LEASE_HEADER: self._lease_token or ""},
        )
        self._handle_response(response)

    async def download_file(self, path: str) -> bytes:
        sandbox_id = self._require_active()
        response = await self._client.get(
            f"/v1/sandboxes/{sandbox_id}/files/{path}",
            headers={LEASE_HEADER: self._lease_token or ""},
        )
        if response.is_success:
            return response.content
        code, message, details = _parse_error(response)
        raise exception_for(
            code,
            message,
            details=details,
            status_code=response.status_code,
        )

    async def snapshot(self) -> str:
        sandbox_id = self._require_active()
        response = await self._client.post(
            f"/v1/sandboxes/{sandbox_id}/snapshot",
            headers={LEASE_HEADER: self._lease_token or ""},
        )
        data = self._handle_response(response)
        return str(data["snapshot_id"])

    async def exec_stream(self, command: str) -> AsyncIterator[ExecStreamEvent]:
        sandbox_id = self._require_active()
        async with self._client.stream(
            "POST",
            f"/v1/sandboxes/{sandbox_id}/exec",
            params={"stream": "1"},
            json={"command": command},
            headers={LEASE_HEADER: self._lease_token or ""},
        ) as response:
            if not response.is_success:
                payload = await response.aread()
                err_response = httpx.Response(
                    response.status_code,
                    headers=response.headers,
                    content=payload,
                    request=response.request,
                )
                code, message, details = _parse_error(err_response)
                raise exception_for(
                    code,
                    message,
                    details=details,
                    status_code=response.status_code,
                )
            async for line in response.aiter_lines():
                if not line:
                    continue
                data = self._parse_stream_event(line, response.status_code)
                yield ExecStreamEvent(
                    event=data["event"],
                    chunk=data.get("chunk"),
                    exit_code=data.get("exit_code"),
                    duration_ms=data.get("duration_ms"),
                    sandbox_id=data.get("sandbox_id"),
                    backend=data.get("backend"),
                )

    def _require_active(self) -> str:
        if not self._sandbox_id:
            raise SandboxError(
                "Sandbox non inizializzata. Usa 'async with Sandbox(...) as sb:'."
            )
        return self._sandbox_id

    @staticmethod
    def _parse_stream_event(line: str, status_code: int) -> dict[str, Any]:
        try:
            payload = httpx.Response(status_code, content=line).json()
        except ValueError as exc:
            raise SandboxError(f"Evento stream non JSON: {exc}") from exc
        if not isinstance(payload, dict):
            raise SandboxError("Evento stream non valido")
        return payload

    async def _create(self) -> None:
        spec = self._config.to_spec()
        response = await self._client.post("/v1/sandboxes", json=spec)
        data = self._handle_response(response)
        self._sandbox_id = data["sandbox_id"]
        self._lease_token = data["lease_token"]

    async def _destroy(self) -> None:
        """Best-effort teardown. Errors are logged but never re-raised.

        We swallow failures because ``__aexit__`` is often reached while the
        caller is already propagating an exception; masking the original
        failure with a teardown error would lose information.
        """

        if not self._sandbox_id:
            return
        try:
            await self._client.delete(
                f"/v1/sandboxes/{self._sandbox_id}",
                headers={LEASE_HEADER: self._lease_token or ""},
            )
        except httpx.HTTPError as exc:
            _log.warning(
                "destroy della sandbox %s fallito: %s", self._sandbox_id, exc
            )
        finally:
            self._sandbox_id = None
            self._lease_token = None

    @staticmethod
    def _handle_response(response: httpx.Response) -> dict[str, Any]:
        """Raise a typed exception on error, return parsed JSON on success.

        2xx → parsed JSON body (or ``{}`` for 204 No Content).
        4xx/5xx → :class:`SandboxError` (or a subclass) built from the
        daemon's error envelope. Non-JSON bodies fall back to the HTTP
        reason phrase.
        """

        if response.status_code == httpx.codes.NO_CONTENT:
            return {}

        if response.is_success:
            try:
                return response.json()
            except ValueError as exc:
                raise SandboxError(
                    f"Risposta daemon non JSON ({response.status_code}): {exc}",
                    status_code=response.status_code,
                ) from exc

        code, message, details = _parse_error(response)
        raise exception_for(
            code,
            message,
            details=details,
            status_code=response.status_code,
        )


def _parse_error(
    response: httpx.Response,
) -> tuple[str | None, str, dict[str, Any]]:
    """Extract ``(code, message, details)`` from a daemon error response."""

    try:
        payload = response.json()
    except ValueError:
        return None, response.text or response.reason_phrase, {}

    envelope = payload.get("error") if isinstance(payload, dict) else None
    if not isinstance(envelope, dict):
        return None, str(payload), {}

    code = envelope.get("code")
    message = envelope.get("message") or response.reason_phrase
    details = envelope.get("details") or {}
    if not isinstance(details, dict):
        details = {"raw": details}
    return code, str(message), details


__all__ = ["Sandbox", "DEFAULT_DAEMON_URL"]
