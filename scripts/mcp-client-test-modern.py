# Drives `legatorocr --mcp` with the official MCP Python SDK 2.x client in its default "auto" mode, which
# probes server/discover and talks revision 2026-07-28 (no initialize). Needs mcp >= 2.
#   python3 scripts/mcp-client-test-modern.py target/debug/legatorocr IMAGE.png
import asyncio, sys
from mcp import StdioServerParameters
from mcp.client.client import Client
from mcp.client.stdio import stdio_client

async def main():
    params = StdioServerParameters(command=(sys.argv[1] if len(sys.argv) > 1 else "legatorocr"), args=["--mcp"])
    async with Client(stdio_client(params)) as c:
        print("protocol:", c.protocol_version)
        assert c.protocol_version == "2026-07-28", c.protocol_version
        tools = await c.list_tools()
        names = [t.name for t in tools.tools]
        print("tools:", names)
        assert "scan_to_pdf" in names
        if len(sys.argv) > 2:
            res = await c.call_tool("scan_to_pdf", {"inputs": [sys.argv[2]], "pdf": "/tmp/mcp_out_modern.pdf"})
            print("scan_to_pdf:", res.is_error)
            assert not res.is_error
asyncio.run(main())
