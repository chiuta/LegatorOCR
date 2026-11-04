# Drives `legatorocr --mcp` with the official MCP Python client (pip install mcp):
#   python3 scripts/mcp-client-test.py target/debug/legatorocr IMAGE.png
import asyncio, sys
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client
async def main():
    params = StdioServerParameters(command=(sys.argv[1] if len(sys.argv) > 1 else "legatorocr"), args=["--mcp"])
    async with stdio_client(params) as (r, w):
        async with ClientSession(r, w) as s:
            init = await s.initialize()
            print("server:", init.server_info.name, init.protocol_version)
            tools = await s.list_tools()
            print("tools:", [t.name for t in tools.tools])
            res = await s.call_tool("scan_to_pdf", {"inputs": [sys.argv[2]], "pdf": "/tmp/mcp_out.pdf"})
            print("scan_to_pdf:", res.is_error, str(res.content)[:200])
asyncio.run(main())
