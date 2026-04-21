"""Review a Python file with Claude and verify the fix in AgentSandbox."""

from __future__ import annotations

import asyncio
import base64
import json
import os
import sys
from pathlib import Path
from typing import Any

import anthropic
from agentsandbox import Sandbox

DEFAULT_MODEL = "claude-sonnet-4-20250514"

SYSTEM_PROMPT = """Sei un code reviewer esperto Python.
Rispondi sempre in JSON valido, senza markdown, con questa forma:
{
  "bugs": ["descrizione bug 1", "descrizione bug 2"],
  "fixed_code": "codice Python corretto e completo",
  "explanation": "spiegazione breve dei fix"
}
Non aggiungere testo fuori dal JSON."""


def require_api_key() -> None:
    if not os.environ.get("ANTHROPIC_API_KEY"):
        raise RuntimeError("ANTHROPIC_API_KEY non impostata")


def extract_text(message: Any) -> str:
    parts: list[str] = []
    for block in getattr(message, "content", []):
        if getattr(block, "type", None) == "text":
            text = getattr(block, "text", "")
            if text:
                parts.append(text)
    return "".join(parts).strip()


async def request_review(source: str, model: str) -> dict[str, Any]:
    client = anthropic.Anthropic()

    def create_message() -> Any:
        return client.messages.create(
            model=model,
            max_tokens=2048,
            system=SYSTEM_PROMPT,
            messages=[
                {
                    "role": "user",
                    "content": (
                        "Analizza e correggi questo codice Python. "
                        "Mantieni la soluzione semplice e corretta.\n\n"
                        f"```python\n{source}\n```"
                    ),
                }
            ],
        )

    message = await asyncio.to_thread(create_message)
    payload = extract_text(message)
    if not payload:
        raise RuntimeError("Claude ha risposto senza testo")

    try:
        review = json.loads(payload)
    except json.JSONDecodeError as exc:
        raise RuntimeError("Claude non ha restituito JSON valido") from exc

    if not isinstance(review, dict):
        raise RuntimeError("Formato risposta inatteso")

    bugs = review.get("bugs")
    fixed_code = review.get("fixed_code")
    explanation = review.get("explanation")
    if not isinstance(bugs, list) or not isinstance(fixed_code, str) or not isinstance(
        explanation, str
    ):
        raise RuntimeError("JSON di Claude incompleto o malformato")

    return review


async def run_fixed_code(fixed_code: str) -> tuple[str, str, int, int]:
    encoded = base64.b64encode(fixed_code.encode("utf-8")).decode("ascii")
    write_command = (
        "python - <<'PY'\n"
        "import base64\n"
        "from pathlib import Path\n"
        f"Path('/workspace/script.py').write_bytes(base64.b64decode('{encoded}'))\n"
        "PY"
    )

    async with Sandbox(runtime="python", ttl=60, memory_mb=256) as sandbox:
        await sandbox.exec(write_command)
        result = await sandbox.exec(
            "python -m py_compile /workspace/script.py && python /workspace/script.py"
        )
        return result.stdout, result.stderr, result.exit_code, result.duration_ms


async def review_and_run(filepath: str) -> None:
    require_api_key()

    model = os.environ.get("ANTHROPIC_MODEL", DEFAULT_MODEL)
    source = Path(filepath).read_text(encoding="utf-8")

    print(f"Reviewing: {filepath}")
    print(f"Using model: {model}")
    print("-" * 50)
    print("Requesting review from Claude...")

    review = await request_review(source, model)

    bugs = [str(item) for item in review["bugs"]]
    print(f"\nBugs found ({len(bugs)}):")
    for bug in bugs:
        print(f" - {bug}")

    print("\nExplanation:")
    print(str(review["explanation"]).strip())

    print("\nRunning fixed code inside AgentSandbox...")
    stdout, stderr, exit_code, duration_ms = await run_fixed_code(str(review["fixed_code"]))

    print("\nSandbox output:")
    print("-" * 30)
    if stdout:
        print(stdout, end="" if stdout.endswith("\n") else "\n")
    if stderr:
        print("STDERR:")
        print(stderr, end="" if stderr.endswith("\n") else "\n")
    print("-" * 30)

    if exit_code == 0:
        print(f"Execution succeeded (exit 0, {duration_ms}ms)")
    else:
        print(f"Execution failed (exit {exit_code}, {duration_ms}ms)")


def main() -> int:
    filepath = sys.argv[1] if len(sys.argv) > 1 else "sample_code/buggy_script.py"
    try:
        asyncio.run(review_and_run(filepath))
    except KeyboardInterrupt:
        print("\nInterrupted")
        return 130
    except Exception as exc:  # noqa: BLE001 - example CLI entry point
        print(f"Error: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
