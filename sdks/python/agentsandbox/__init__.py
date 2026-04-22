"""Python SDK for AgentSandbox.

Quickstart::

    import asyncio
    from agentsandbox import Sandbox

    async def main():
        async with Sandbox(runtime="python", ttl=300) as sb:
            result = await sb.exec("python -c 'print(42)'")
            print(result.stdout)

    asyncio.run(main())
"""

from .client import DEFAULT_DAEMON_URL, Sandbox
from .exceptions import (
    BackendUnavailableError,
    ExecTimeoutError,
    InternalDaemonError,
    LeaseInvalidError,
    NotSupportedError,
    SandboxError,
    SandboxExpiredError,
    SandboxNotFoundError,
    SpecInvalidError,
)
from .models import ExecResult, ExecStreamEvent, SandboxConfig, SandboxInfo

__version__ = "0.1.0"

__all__ = [
    "__version__",
    "Sandbox",
    "SandboxConfig",
    "SandboxInfo",
    "ExecResult",
    "ExecStreamEvent",
    "DEFAULT_DAEMON_URL",
    "SandboxError",
    "SandboxNotFoundError",
    "SandboxExpiredError",
    "SpecInvalidError",
    "BackendUnavailableError",
    "NotSupportedError",
    "ExecTimeoutError",
    "LeaseInvalidError",
    "InternalDaemonError",
]
