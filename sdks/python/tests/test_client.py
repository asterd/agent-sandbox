"""End-to-end tests for the SDK.

These require the daemon to be running locally with a healthy Docker
backend. Run with::

    pytest -m integration

The marker ``integration`` is registered in ``pyproject.toml``; tests are
skipped by default when neither ``-m integration`` nor an explicit
``AGENTSANDBOX_INTEGRATION=1`` env var is set.
"""

from __future__ import annotations

import os

import httpx
import pytest

from agentsandbox import DEFAULT_DAEMON_URL, Sandbox

pytestmark = pytest.mark.integration


def _daemon_url() -> str:
    return os.environ.get("AGENTSANDBOX_DAEMON_URL", DEFAULT_DAEMON_URL)


async def test_version_present():
    from agentsandbox import __version__

    assert __version__ == "0.1.0"


async def test_basic_exec():
    async with Sandbox(runtime="python", ttl=60, daemon_url=_daemon_url()) as sb:
        result = await sb.exec("echo 'hello from sandbox'")
        assert result.success
        assert "hello from sandbox" in result.stdout


async def test_python_code_runs():
    async with Sandbox(runtime="python", ttl=60, daemon_url=_daemon_url()) as sb:
        result = await sb.exec("python -c 'print(1 + 1)'")
        assert result.stdout.strip() == "2"


async def test_exit_code_captured():
    async with Sandbox(runtime="shell", ttl=60, daemon_url=_daemon_url()) as sb:
        result = await sb.exec("exit 42")
        assert result.exit_code == 42
        assert not result.success


async def test_sandbox_destroyed_on_exit():
    sandbox_id: str | None = None
    async with Sandbox(runtime="python", ttl=60, daemon_url=_daemon_url()) as sb:
        sandbox_id = sb.sandbox_id
        assert sandbox_id is not None

    async with httpx.AsyncClient(base_url=_daemon_url()) as client:
        r = await client.get(f"/v1/sandboxes/{sandbox_id}")
        assert r.status_code == 404
