#!/usr/bin/env python3
"""SDTEST-2865 / SDUC-557: production launcher, fake providers, loopback MCP only."""
import base64
import hashlib
import json
import os
from pathlib import Path
import signal
import socketserver
import subprocess
import sys
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[1]
LAUNCHER = ROOT / "crates/shelldeck-ssh/src/browser_bridge.py"
PROVIDER = ROOT / "scripts/fixtures/browser-provider.py"
TOKEN = "ab" * 32

class Backend(socketserver.StreamRequestHandler):
    def handle(self):
        for line in self.rfile:
            envelope = json.loads(line)
            assert envelope["capability"] == TOKEN
            request = envelope["request"]
            self.server.requests.append(request)
            method = request["method"]
            result = {"instructions":"fixture"} if method == "initialize" else {"tools":[]} if method == "tools/list" else {"content":[{"type":"text","text":"{\"tabs\":[]}"}]}
            reply = {"jsonrpc":"2.0","id":request.get("id"),"result":result}
            self.wfile.write(json.dumps(reply).encode() + b"\n")
            self.wfile.flush()

class Tests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="shelldeck-ssh-browser-😀-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.server = socketserver.ThreadingTCPServer(("127.0.0.1",0),Backend)
        self.server.daemon_threads = True
        self.server.requests = []
        self.thread = threading.Thread(target=self.server.serve_forever,daemon=True)
        self.thread.start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.env = dict(os.environ, TMPDIR=str(self.root), SHELLDECK_FIXTURE_REMOVE="yes")
        self.project_config = self.root / ".mcp.json"
        self.project_config.write_bytes(b"original project MCP configuration\n")

    def payload(self, provider="codex", case="normal", resume=False):
        args = [str(PROVIDER), "exec"] + (["resume","fixture-session"] if resume else []) + ["--fixture-case",case]
        return {"version":1,"provider":provider,"program":sys.executable,"args":args,
                "workdir":str(self.root),"stdin":base64.b64encode("prompt\0\n😀".encode()).decode(),
                "remove_env":["SHELLDECK_FIXTURE_REMOVE"],
                "endpoint":"127.0.0.1:" + str(self.server.server_address[1]),"capability":TOKEN}

    def start(self, payload):
        child = subprocess.Popen([sys.executable,"-u",str(LAUNCHER)], stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=self.env)
        child.stdin.write(json.dumps(payload).encode())
        child.stdin.close()
        child.stdin = None
        return child

    def test_provider_configuration_stdin_notifications_exit_and_cleanup(self):
        for provider,resume,case in [("claude",False,"normal"),("codex",False,"normal"),("codex",True,"fail")]:
            with self.subTest(provider=provider,resume=resume):
                # Use the real provider argument layout; the fake program is a
                # Python script with a shebang, not the user's installed CLI.
                fake = self.root / provider
                fake.write_bytes(PROVIDER.read_bytes())
                fake.chmod(0o700)
                payload = self.payload(provider,case,resume)
                payload["program"] = str(fake)
                payload["args"] = payload["args"][1:]
                child = self.start(payload)
                stdout,stderr = child.communicate(timeout=15)
                self.assertEqual(child.returncode,19 if case=="fail" else 0,stderr.decode())
                setup,report = [json.loads(line) for line in stdout.splitlines()]
                self.assertEqual(setup["stdin_sha256"],hashlib.sha256("prompt\0\n😀".encode()).hexdigest())
                self.assertEqual(setup["cwd"],str(self.root))
                self.assertTrue(setup["removed_env"])
                self.assertTrue(report["initialized"])
                self.assertFalse(Path(setup["fixture_setup"]).exists())
                self.assertNotIn(TOKEN.encode(),stdout+stderr)
                self.assertEqual(self.project_config.read_bytes(),b"original project MCP configuration\n")
        methods = [request["method"] for request in self.server.requests]
        self.assertEqual(methods,["initialize","notifications/initialized","tools/list","tools/call"]*3)

    @unittest.skipUnless(os.name == "posix","production remote launcher requires a POSIX host")
    def test_term_and_hup_remove_private_directory(self):
        for sig in (signal.SIGTERM,signal.SIGHUP):
            fake=self.root/"claude"
            fake.write_bytes(PROVIDER.read_bytes());fake.chmod(0o700)
            payload=self.payload("claude","hold")
            payload["program"]=str(fake);payload["args"]=payload["args"][1:]
            child=self.start(payload)
            try:
                setup=json.loads(child.stdout.readline())
                self.assertTrue(Path(setup["fixture_setup"]).exists())
                child.send_signal(sig)
                stdout,stderr=child.communicate(timeout=8)
                self.assertEqual(child.returncode,128+sig,stderr.decode())
                self.assertFalse(Path(setup["fixture_setup"]).exists())
                self.assertNotIn(TOKEN.encode(),stdout+stderr)
            finally:
                if child.poll() is None: child.kill();child.communicate()

    def test_invalid_or_oversized_payload_cannot_start_provider(self):
        for changes in ({"endpoint":"0.0.0.0:80"},{"capability":"secret"},{"provider":"jcode"},
                        {"workdir":str(self.root/"missing")},{"args":["x"*(4*1024*1024)]}):
            payload=self.payload();payload.update(changes)
            child=self.start(payload)
            stdout,stderr=child.communicate(timeout=8)
            self.assertEqual(child.returncode,126)
            self.assertEqual(stdout,b"")
            self.assertNotIn(TOKEN.encode(),stderr)
            self.assertFalse(list(self.root.glob("shelldeck-browser-*")))
        self.assertFalse(self.server.requests)

if __name__ == "__main__":
    unittest.main()
