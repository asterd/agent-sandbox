"""Shared test plumbing.

Integration tests require a running daemon. Instead of failing the default
``pytest`` run we auto-skip them when the daemon is unreachable; explicit
``pytest -m integration`` still surfaces the full failure so CI can treat
"daemon should be up" as an invariant.
"""

from __future__ import annotations

import os
import socket
from urllib.parse import urlparse

import pytest

from agentsandbox import DEFAULT_DAEMON_URL


def _daemon_reachable(url: str, timeout: float = 0.25) -> bool:
    parsed = urlparse(url)
    host = parsed.hostname or "127.0.0.1"
    port = parsed.port or 7847
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return True
    except OSError:
        return False


def pytest_collection_modifyitems(config: pytest.Config, items: list[pytest.Item]) -> None:
    # If the user explicitly asked for integration tests, don't second-guess.
    if config.getoption("-m") == "integration":
        return

    url = os.environ.get("AGENTSANDBOX_DAEMON_URL", DEFAULT_DAEMON_URL)
    if _daemon_reachable(url):
        return

    skip = pytest.mark.skip(reason=f"daemon non raggiungibile su {url}")
    for item in items:
        if "integration" in item.keywords:
            item.add_marker(skip)
