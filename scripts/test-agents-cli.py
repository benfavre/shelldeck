#!/usr/bin/env python3
"""Linux production CLI contracts, using disposable XDG homes and no GUI.

Usage: python3 scripts/test-agents-cli.py target/debug/shelldeck
The handoff fixture owns its loopback listener and discovery file. It never
starts a provider or reads an existing ShellDeck profile.
"""
from pathlib import Path
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
import unittest


class AgentsCli(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="shelldeck-agents-cli-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.environment = dict(os.environ,
                                XDG_CONFIG_HOME=str(self.root / "config"),
                                XDG_DATA_HOME=str(self.root / "data"),
                                XDG_CACHE_HOME=str(self.root / "cache"))
        self.environment.pop("DISPLAY", None)
        self.environment.pop("WAYLAND_DISPLAY", None)

    def run_cli(self, *arguments):
        return subprocess.run([str(BINARY), *arguments], env=self.environment,
                              text=True, capture_output=True, timeout=10)

    # SDTEST-2640 / SDUC-558: the shipped executable handles informational and
    # invalid arguments before attempting a GUI, config/keychain or primary IPC.
    def test_sdtest_2640_headless_help_version_and_errors_have_no_profile_side_effects(self):
        for argument in ("--help", "-h"):
            result = self.run_cli(argument)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("--agents", result.stdout)
            self.assertIn("--main", result.stdout)
            self.assertIn("Usage: shelldeck", result.stdout)
        result = self.run_cli("--version")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertRegex(result.stdout, r"^ShellDeck \d+\.\d+\.\d+")
        for arguments in (("--unknown",), ("--agents", "shelldeck://issue/example")):
            result = self.run_cli(*arguments)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Usage: shelldeck", result.stderr)
        self.assertEqual(list(self.root.iterdir()), [], "CLI-only requests wrote desktop state")

    # SDTEST-2641 / SDUC-558: a second real executable sends the exact Agents
    # intent (or explicit full-app focus) before constructing another runtime.
    def test_sdtest_2641_agents_and_normal_launch_handoff_before_gui_or_config(self):
        for arguments, payload in [(("--agents",), "shelldeck://agents"),
                                   (("shelldeck://agents",), "shelldeck://agents"),
                                   ((), "shelldeck://agents"),
                                   (("-psn_0_42",), "shelldeck://agents"),
                                   (("--main",), ""),
                                   (("shelldeck://issue/MixedCase",), "shelldeck://issue/MixedCase"),
                                   (("shelldeck://assistant",), "shelldeck://assistant")]:
            with self.subTest(arguments=arguments):
                with socket.socket() as server:
                    server.bind(("127.0.0.1", 0))
                    server.listen(1)
                    server.settimeout(10)
                    config = self.root / "config/shelldeck"
                    config.mkdir(parents=True, exist_ok=True)
                    discovery = config / "instance.json"
                    token = "isolated-cli-handoff-fixture"
                    discovery.write_text(json.dumps({"port": server.getsockname()[1], "token": token}))
                    discovery.chmod(0o600)
                    received = []
                    errors = []

                    def accept():
                        try:
                            with server.accept()[0] as connection:
                                connection.settimeout(5)
                                with connection.makefile("rb") as stream:
                                    received.extend([stream.readline(256).decode().rstrip("\n"),
                                                     stream.readline(4096).decode().rstrip("\n")])
                                connection.sendall(b"OK\n")
                        except Exception as error:
                            errors.append(repr(error))

                    worker = threading.Thread(target=accept, daemon=True)
                    worker.start()
                    result = self.run_cli(*arguments)
                    worker.join(timeout=5)
                    self.assertFalse(worker.is_alive(), "handoff did not complete")
                    self.assertEqual(errors, [])
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(received, [token, payload])
                    self.assertEqual(json.loads(discovery.read_text())["token"], token)
                    self.assertEqual([p.relative_to(self.root).as_posix()
                                      for p in self.root.rglob("*") if p.is_file()],
                                     ["config/shelldeck/instance.json"])

        # A refused startup must report failure before GUI/config startup,
        # rather than claim that another application accepted the request.
        discovery.unlink()
        (config / "instance.json.startup.lock").mkdir()
        result = self.run_cli("--agents")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Unable to initialize ShellDeck single-instance routing", result.stderr)
        self.assertEqual([p for p in self.root.rglob("*") if p.is_file()], [])


if __name__ == "__main__":
    if sys.platform != "linux":
        raise SystemExit("This fixture requires Linux XDG isolation; use native core tests on other OSes")
    if len(sys.argv) != 2:
        raise SystemExit("Usage: test-agents-cli.py /path/to/shelldeck")
    BINARY = Path(sys.argv.pop()).resolve(strict=True)
    unittest.main()
