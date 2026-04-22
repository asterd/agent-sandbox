import asyncio

from agentsandbox import Sandbox


async def main() -> None:
    async with Sandbox(runtime="python", backend="docker") as sb:
        await sb.upload_file("script.py", b"print('stream-ok')\n")
        async for event in sb.exec_stream("python /script.py > /result.txt"):
            print(event.event, event.chunk or "")
        result = await sb.download_file("result.txt")
        print(result.decode().strip())


if __name__ == "__main__":
    asyncio.run(main())
