"""Run the same Python workload on every available Python-capable backend."""

from __future__ import annotations

import asyncio

import httpx

from agentsandbox import Sandbox

COMMAND = "python -c 'print(\"agentsandbox multi-backend ok\")'"


async def available_python_backends() -> list[str]:
    async with httpx.AsyncClient(base_url="http://127.0.0.1:7847", timeout=10.0) as client:
        response = await client.get("/v1/backends")
        response.raise_for_status()
        payload = response.json()

    items = payload.get("items", [])
    backends: list[str] = []
    for item in items:
        presets = item.get("capabilities", {}).get("supported_presets", [])
        if "python" in presets:
            backends.append(str(item["id"]))
    return backends


async def run_on(backend_id: str) -> dict[str, object]:
    try:
        async with Sandbox(runtime="python", ttl=60, backend=backend_id) as sb:
            result = await sb.exec(COMMAND)
            return {
                "backend": backend_id,
                "output": result.stdout.strip(),
                "duration_ms": result.duration_ms,
                "ok": result.success,
            }
    except Exception as exc:  # noqa: BLE001 - example CLI surface
        return {"backend": backend_id, "error": str(exc), "ok": False}


async def main() -> None:
    available = await available_python_backends()
    if not available:
        raise RuntimeError("nessun backend compatibile con il preset python")

    print(f"Backend disponibili: {', '.join(available)}")
    print("")

    results = await asyncio.gather(*(run_on(backend_id) for backend_id in available))

    outputs: set[str] = set()
    failures = 0
    for result in results:
        if result["ok"]:
            output = str(result["output"])
            outputs.add(output)
            print(
                f"OK  {result['backend']}: {output} ({result['duration_ms']}ms)"
            )
        else:
            failures += 1
            print(f"ERR {result['backend']}: {result['error']}")

    if failures:
        raise RuntimeError(f"{failures} backend falliti")

    if len(outputs) == 1:
        print("")
        print("OK  Output identico su tutti i backend.")
    else:
        raise RuntimeError("output diverso tra backend")


if __name__ == "__main__":
    asyncio.run(main())
