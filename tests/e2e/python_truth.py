from __future__ import annotations

import asyncio
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "sdks/python"))

from agentsandbox import Sandbox  # noqa: E402


async def main() -> int:
    daemon_url = os.environ.get("AGENTSANDBOX_DAEMON_URL", "http://127.0.0.1:7847")
    async with Sandbox(runtime="python", ttl=60, daemon_url=daemon_url) as sandbox:
        result = await sandbox.exec("python -c 'print(40 + 2)'")
        info = await sandbox.inspect()

        assert result.exit_code == 0, result.stderr
        assert result.stdout.strip() == "42", result.stdout
        assert info.status == "running", info
        assert info.backend, info

    return 0


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
