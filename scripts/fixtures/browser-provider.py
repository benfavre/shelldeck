#!/usr/bin/env python3
"""Harmless Claude/Codex stand-in. Never loads provider settings or credentials."""
import base64
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import time
try:
    import tomllib
except ImportError:
    tomllib = None

args = sys.argv[1:]
case = args[args.index("--fixture-case") + 1]
if "--mcp-config" in args:
    path = Path(args[args.index("--mcp-config") + 1])
    config = json.loads(path.read_text())["mcpServers"]["shelldeck_browser"]
else:
    config = {}
    for index, value in enumerate(args):
        if value == "-c" and args[index + 1].startswith("mcp_servers.shelldeck_browser."):
            key, raw = args[index + 1].split("=", 1)
            config[key.rsplit(".", 1)[1]] = tomllib.loads("value=" + raw)["value"] if tomllib else json.loads(raw)
    assert config["enabled"] and config["required"]
path = Path(config["args"][0])
directory = path.parent
token = json.loads((directory / "bridge.json").read_text())["capability"]
assert token not in " ".join(sys.argv)
assert stat.S_IMODE(directory.stat().st_mode) == 0o700
assert all(stat.S_IMODE(item.stat().st_mode) == 0o600 for item in directory.iterdir())
proxy = subprocess.Popen([config["command"]] + config["args"], stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE)
print(json.dumps({"fixture_setup": str(directory), "proxy_pid": proxy.pid,
                  "stdin_sha256": hashlib.sha256(sys.stdin.buffer.read()).hexdigest(),
                  "cwd": os.getcwd(), "removed_env": "SHELLDECK_FIXTURE_REMOVE" not in os.environ}), flush=True)
if case == "hold":
    while True:
        time.sleep(1)

sequence = 0
def request(method, params=None, notification=False):
    global sequence
    sequence += 1
    value = {"jsonrpc": "2.0", "method": method, "params": params or {}}
    if not notification:
        value["id"] = sequence
    proxy.stdin.write(json.dumps(value).encode() + b"\n")
    proxy.stdin.flush()
    if notification:
        return None
    response = json.loads(proxy.stdout.readline())
    assert response["id"] == sequence, response
    assert "error" not in response, response
    return response["result"]

def tool(name, arguments=None):
    return request("tools/call", {"name": name, "arguments": arguments or {}})

try:
    initialized = request("initialize", {"protocolVersion":"2024-11-05", "capabilities":{}, "clientInfo":{"name":"shelldeck-fixture","version":"1"}})
    assert "ShellDeck desktop" in initialized["instructions"]
    request("notifications/initialized", notification=True)
    tools = request("tools/list")
    listing = tool("browser_tabs")
    report = {"initialized": True, "tools": tools, "tabs": listing}
    if case.startswith("native-"):
        tabs = json.loads(listing["content"][0]["text"])["tabs"]
        assert len(tabs) == 1, tabs
        browser_id = tabs[0]["browser_id"]
        snapshot = tool("browser_snapshot", {"browser_id": browser_id})
        assert "SSH browser fixture" in json.dumps(snapshot), snapshot
        screenshot = tool("browser_screenshot", {"browser_id":browser_id})
        assert any(item["type"] == "image" and base64.b64decode(item["data"]).startswith(b"\x89PNG") for item in screenshot["content"])
        if case == "native-control":
            clicked = tool("browser_click", {"browser_id":browser_id,"x":80,"y":100})
            assert not clicked.get("isError"), clicked
            typed = tool("browser_type", {"browser_id":browser_id,"text":"via SSH"})
            assert not typed.get("isError"), typed
        else:
            refused = tool("browser_type", {"browser_id":browser_id,"text":"must refuse"})
            assert refused.get("isError"), refused
        outside = tool("browser_snapshot", {"browser_id":"9999999"})
        assert outside.get("isError"), outside
        report["native_browser"] = True
    print(json.dumps(report), flush=True)
finally:
    proxy.stdin.close()
    proxy.wait(timeout=5)
    assert proxy.returncode == 0, proxy.stderr.read().decode()
sys.exit(19 if case == "fail" else 0)
