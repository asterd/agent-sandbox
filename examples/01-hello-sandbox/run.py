"""Minimal AgentSandbox example using the local Python SDK."""

from __future__ import annotations

import asyncio

from agentsandbox import Sandbox


async def main() -> None:
    print("Creazione sandbox...")
    async with Sandbox(runtime="python", ttl=60) as sb:
        result = await sb.exec("python -c 'print(\"hello from sandbox\")'")
        print(f"stdout:    {result.stdout.strip()}")
        print(f"exit_code: {result.exit_code}")
        print(f"duration:  {result.duration_ms}ms")
        assert result.success, "il comando deve avere successo"
        assert result.stdout.strip() == "hello from sandbox"
    print("Sandbox distrutta. Done.")


if __name__ == "__main__":
    asyncio.run(main())
