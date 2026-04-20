"""Unit tests for the Sandbox client using respx as HTTP mock.

These tests do NOT require a running daemon. They exercise the transport
layer: request shapes, header propagation, response parsing, error mapping,
and teardown on exception.
"""

from __future__ import annotations

import json

import httpx
import pytest
import respx

from agentsandbox import (
    LeaseInvalidError,
    Sandbox,
    SandboxError,
    SandboxExpiredError,
    SandboxNotFoundError,
    SpecInvalidError,
)

DAEMON = "http://127.0.0.1:7847"
SANDBOX_ID = "sb-1"
LEASE = "lease-x"


def _create_response(sandbox_id: str = SANDBOX_ID, lease: str = LEASE) -> dict:
    return {
        "sandbox_id": sandbox_id,
        "lease_token": lease,
        "status": "running",
        "expires_at": "2099-01-01T00:00:00+00:00",
        "backend": "docker",
    }


def _error(code: str, message: str) -> dict:
    return {"error": {"code": code, "message": message, "details": {}}}


def _mock_lifecycle(sandbox_id: str = SANDBOX_ID) -> tuple[respx.Route, respx.Route]:
    """Register the create+delete respx routes used by most tests."""
    create = respx.post(f"{DAEMON}/v1/sandboxes").mock(
        return_value=httpx.Response(201, json=_create_response(sandbox_id))
    )
    destroy = respx.delete(f"{DAEMON}/v1/sandboxes/{sandbox_id}").mock(
        return_value=httpx.Response(204)
    )
    return create, destroy


@pytest.mark.asyncio
@respx.mock
async def test_context_manager_creates_and_destroys():
    create, destroy = _mock_lifecycle()

    async with Sandbox(runtime="python", ttl=60) as sb:
        assert sb.sandbox_id == SANDBOX_ID

    assert create.called
    assert destroy.called
    assert destroy.calls.last.request.headers["X-Lease-Token"] == LEASE


@pytest.mark.asyncio
@respx.mock
async def test_exec_sends_command_and_parses_response():
    _mock_lifecycle()
    exec_route = respx.post(f"{DAEMON}/v1/sandboxes/{SANDBOX_ID}/exec").mock(
        return_value=httpx.Response(
            200,
            json={
                "stdout": "hi\n",
                "stderr": "",
                "exit_code": 0,
                "duration_ms": 42,
            },
        )
    )

    async with Sandbox(runtime="python") as sb:
        result = await sb.exec("echo hi")

    assert exec_route.called
    body = json.loads(exec_route.calls.last.request.content)
    assert body == {"command": "echo hi"}
    assert exec_route.calls.last.request.headers["X-Lease-Token"] == LEASE

    assert result.success is True
    assert result.stdout == "hi\n"
    assert result.duration_ms == 42


@pytest.mark.asyncio
@respx.mock
async def test_exec_nonzero_exit_is_not_an_exception():
    _mock_lifecycle()
    respx.post(f"{DAEMON}/v1/sandboxes/{SANDBOX_ID}/exec").mock(
        return_value=httpx.Response(
            200,
            json={"stdout": "", "stderr": "nope", "exit_code": 42, "duration_ms": 1},
        )
    )

    async with Sandbox(runtime="shell") as sb:
        result = await sb.exec("exit 42")

    assert result.exit_code == 42
    assert not result.success


@pytest.mark.asyncio
@respx.mock
async def test_inspect_returns_sandbox_info():
    _mock_lifecycle()
    respx.get(f"{DAEMON}/v1/sandboxes/{SANDBOX_ID}").mock(
        return_value=httpx.Response(
            200,
            json={
                "sandbox_id": SANDBOX_ID,
                "status": "running",
                "backend": "docker",
                "created_at": "2099-01-01T00:00:00+00:00",
                "expires_at": "2099-01-01T00:05:00+00:00",
                "error_message": None,
            },
        )
    )

    async with Sandbox(runtime="python") as sb:
        info = await sb.inspect()

    assert info.sandbox_id == SANDBOX_ID
    assert info.status == "running"
    assert info.backend == "docker"
    assert info.error_message is None


@pytest.mark.asyncio
@respx.mock
async def test_spec_invalid_maps_to_typed_exception():
    respx.post(f"{DAEMON}/v1/sandboxes").mock(
        return_value=httpx.Response(
            422, json=_error("SPEC_INVALID", "runtime mancante")
        )
    )

    with pytest.raises(SpecInvalidError) as info:
        async with Sandbox(runtime="python"):
            pass

    assert info.value.code == "SPEC_INVALID"
    assert "runtime mancante" in str(info.value)
    assert info.value.status_code == 422


@pytest.mark.asyncio
@respx.mock
async def test_not_found_maps_to_typed_exception():
    _mock_lifecycle()
    respx.post(f"{DAEMON}/v1/sandboxes/{SANDBOX_ID}/exec").mock(
        return_value=httpx.Response(
            404, json=_error("SANDBOX_NOT_FOUND", "sandbox sb-1 non trovata")
        )
    )

    async with Sandbox(runtime="python") as sb:
        with pytest.raises(SandboxNotFoundError):
            await sb.exec("echo x")


@pytest.mark.asyncio
@respx.mock
async def test_expired_maps_to_typed_exception():
    _mock_lifecycle()
    respx.post(f"{DAEMON}/v1/sandboxes/{SANDBOX_ID}/exec").mock(
        return_value=httpx.Response(
            410, json=_error("SANDBOX_EXPIRED", "sandbox scaduta")
        )
    )

    async with Sandbox(runtime="python") as sb:
        with pytest.raises(SandboxExpiredError):
            await sb.exec("echo x")


@pytest.mark.asyncio
@respx.mock
async def test_lease_invalid_maps_to_typed_exception():
    _mock_lifecycle()
    respx.post(f"{DAEMON}/v1/sandboxes/{SANDBOX_ID}/exec").mock(
        return_value=httpx.Response(
            403, json=_error("LEASE_INVALID", "lease non valido")
        )
    )

    async with Sandbox(runtime="python") as sb:
        with pytest.raises(LeaseInvalidError):
            await sb.exec("echo x")


@pytest.mark.asyncio
@respx.mock
async def test_unknown_error_code_falls_back_to_sandbox_error():
    respx.post(f"{DAEMON}/v1/sandboxes").mock(
        return_value=httpx.Response(
            500, json=_error("FUTURE_CODE", "qualcosa di nuovo")
        )
    )

    with pytest.raises(SandboxError) as info:
        async with Sandbox(runtime="python"):
            pass
    assert not isinstance(info.value, SpecInvalidError)
    assert info.value.code == "FUTURE_CODE"


@pytest.mark.asyncio
@respx.mock
async def test_destroy_called_when_exception_raised_in_body():
    _, destroy = _mock_lifecycle()

    with pytest.raises(RuntimeError):
        async with Sandbox(runtime="python"):
            raise RuntimeError("boom")

    assert destroy.called


@pytest.mark.asyncio
@respx.mock
async def test_destroy_errors_are_swallowed():
    respx.post(f"{DAEMON}/v1/sandboxes").mock(
        return_value=httpx.Response(201, json=_create_response())
    )
    respx.delete(f"{DAEMON}/v1/sandboxes/{SANDBOX_ID}").mock(
        side_effect=httpx.ConnectError("daemon gone")
    )

    async with Sandbox(runtime="python") as sb:
        assert sb.sandbox_id == SANDBOX_ID


@pytest.mark.asyncio
async def test_exec_before_enter_raises():
    sb = Sandbox(runtime="python")
    with pytest.raises(SandboxError):
        await sb.exec("echo hi")
