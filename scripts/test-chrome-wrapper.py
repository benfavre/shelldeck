#!/usr/bin/env python3
"""SDTEST-2954 / SDUC-573 — opt-in native Chrome preview acceptance against a loopback-only login fixture.

Run with --native. Uses a disposable profile, dummy credentials and the installed
Chrome/Xvfb. It never opens an existing Chrome profile or signs into Google.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import queue
import sqlite3
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
import zlib

SOURCE = Path(__file__).resolve().parents[1] / "crates/shelldeck-browser/src/chrome_wrapper.py"


class Browser:
    def __init__(self, profile, root):
        env = os.environ.copy()
        env["SHELLDECK_CHROME_TEST_PROFILE"] = "1"
        self.log = (root / ("helper-" + str(time.time_ns()) + ".log")).open("w")
        self.process = subprocess.Popen([sys.executable, str(SOURCE), "--profile", str(profile)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, env=env)
        self.events = queue.Queue()
        self.frame = None
        self.paint_count = 0
        self.history = []
        threading.Thread(target=self.read, daemon=True).start()

    def exact(self, length):
        out = bytearray()
        while len(out) < length:
            data = self.process.stdout.read(length - len(out))
            if not data:
                raise EOFError()
            out.extend(data)
        return out

    def read(self):
        try:
            while True:
                length, = struct.unpack("<I", self.exact(4))
                assert 0 < length <= 16 * 1024 * 1024
                envelope = json.loads(self.exact(length))
                assert 0 <= envelope["payload_len"] <= 64 * 1024 * 1024
                payload = self.exact(envelope["payload_len"])
                message = envelope["message"]
                if isinstance(message, dict) and "Paint" in message:
                    self.frame = message["Paint"], payload
                    self.paint_count += 1
                else:
                    self.history.append(message)
                    self.events.put(message)
        except (EOFError, BrokenPipeError):
            self.events.put({"EOF": {}})

    def send(self, message):
        header = json.dumps({"message": message, "payload_len": 0}).encode()
        self.process.stdin.write(struct.pack("<I", len(header)) + header)
        self.process.stdin.flush()

    def wait(self, predicate, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            for message in self.history:
                assert "Error" not in message, message
            assert self.process.poll() is None, self.history
            time.sleep(.03)
        raise AssertionError("Chrome acceptance timed out: " + str(self.history[-8:]))

    def start(self, url):
        message = self.events.get(timeout=15)
        assert "Ready" in message, message
        self.send({"Create": {"browser_id": 1, "url": url, "width": 1000, "height": 760, "scale_factor": 1.0}})
        self.wait(lambda: self.frame is not None)
        time.sleep(1)

    def key(self, code, modifiers=0):
        for kind, flags in [("RawKeyDown", modifiers), ("KeyUp", 0)]:
            self.send({"Key": {"browser_id": 1, "event": {"kind": kind, "windows_key_code": code, "native_key_code": 0, "modifiers": flags, "character": None, "unmodified_character": None, "is_system_key": False}}})

    def text(self, value):
        self.send({"Text": {"browser_id": 1, "text": value}})

    def click(self, x, y):
        for pressed in [True, False]:
            self.send({"MouseButton": {"browser_id": 1, "x": x, "y": y, "button": "Left", "pressed": pressed, "click_count": 1, "modifiers": 0}})

    def png(self, path):
        metadata, pixels = self.frame
        width, height = metadata["width"], metadata["height"]
        assert len(pixels) == width * height * 4
        rgba = bytearray(pixels)
        rgba[0::4], rgba[2::4] = pixels[2::4], pixels[0::4]
        assert all(alpha == 255 for alpha in rgba[3::4])
        raw = b"".join(b"\0" + rgba[y * width * 4:(y + 1) * width * 4] for y in range(height))
        def chunk(name, data):
            return struct.pack(">I", len(data)) + name + data + struct.pack(">I", zlib.crc32(name + data) & 0xFFFFFFFF)
        path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))

    def close(self):
        if self.process.poll() is None:
            self.send("Shutdown")
            self.process.stdin.close()
            self.process.wait(timeout=6)
        self.log.close()


class Fixture(http.server.BaseHTTPRequestHandler):
    posted = []
    restored = []

    def do_GET(self):
        if self.path.startswith("/signed-in"):
            self.restored.append(self.headers.get("Cookie", ""))
            html = "<h1>Signed in to the local fixture</h1><p>The Chrome-owned session is retained.</p>"
        else:
            html = '''<h1>Chrome wrapper login fixture</h1><form method="post" action="/signed-in">
<label>Username<input id="username" name="username" autocomplete="username" autofocus></label>
<label>Password<input id="password" name="password" type="password" autocomplete="current-password"></label>
<button type="submit">Sign in</button></form>'''
        self.respond(html)

    def do_POST(self):
        self.posted.append(urllib.parse.parse_qs(self.rfile.read(int(self.headers["Content-Length"])).decode()))
        self.send_response(303)
        self.send_header("Set-Cookie", "wrapper_session=fixture-only; Max-Age=3600; SameSite=Lax")
        self.send_header("Location", "/signed-in")
        self.end_headers()

    def respond(self, html):
        page = ('''<!doctype html><meta charset="utf-8"><title>Local Chrome login fixture</title>
<style>body{font:18px sans-serif;margin:40px;background:#fff;color:#17212b}label{display:block;margin:20px 0}input{display:block;width:360px;height:36px;font:18px sans-serif}button{height:40px;padding:0 24px;font:18px sans-serif}</style>''' + html).encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(page)))
        self.end_headers()
        self.wfile.write(page)

    def log_message(self, *args):
        pass


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--native", action="store_true", help="Run the installed native Chrome, with dummy credentials only")
    parser.add_argument("--save-at", nargs=2, type=int, metavar=("X", "Y"), help="Click the visually verified native Save button")
    args = parser.parse_args()
    if not args.native:
        parser.error("This is an opt-in native acceptance test; pass --native")
    root = Path(tempfile.mkdtemp(prefix="shelldeck-chrome-acceptance-"))
    profile = root / "profile"
    print("Artifacts:", root, flush=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    origin = f"http://127.0.0.1:{server.server_port}"
    browser = Browser(profile, root)
    try:
        browser.start(origin)
        browser.png(root / "01-login.png")
        # No artificial delay between text, Tab and Return: the helper must
        # serialize clipboard completion and key input rather than losing a field.
        browser.text("wrapper@example.invalid")
        browser.key(9)
        browser.text("Fixture-é-🔐")
        browser.key(13)
        browser.wait(lambda: bool(Fixture.posted))
        assert Fixture.posted == [{"username": ["wrapper@example.invalid"], "password": ["Fixture-é-🔐"]}], Fixture.posted
        time.sleep(1)
        browser.png(root / "02-native-save-prompt.png")
        # A duplicate wrapper may fail; it cannot disturb the active profile.
        duplicate = Browser(profile, root)
        error = duplicate.events.get(timeout=15)
        assert "Error" in error and "another sidebrowser" in error["Error"]["message"], error
        duplicate.close()
        if args.save_at:
            browser.click(*args.save_at)
            time.sleep(.7)
            browser.png(root / "03-password-saved.png")
            # Chrome owns tab shortcuts and the native address field too.
            browser.key(84, 4)
            time.sleep(.3)
            browser.key(87, 4)
            time.sleep(.3)
            browser.key(76, 4)
            browser.key(67, 4)
            browser.wait(lambda: any("TextSelectionChanged" in event and event["TextSelectionChanged"]["selected_text"].startswith(origin) for event in browser.history))
        # Cursor blink may repaint; a retained idle view must not stream at 24fps.
        before = browser.paint_count
        time.sleep(1)
        assert browser.paint_count - before < 15, "Chrome repaint should follow damage"
    finally:
        browser.close()
    assert not list(profile.glob("*.auth")), "Private display authentication must be cleaned up"
    stored = []
    database = profile / "google-chrome/Default/Login Data"
    if database.exists():
        with sqlite3.connect(database) as db:
            stored = db.execute("select origin_url,username_value from logins where username_value=?", ("wrapper@example.invalid",)).fetchall()
    if args.save_at:
        assert stored and stored[0][0].startswith(origin), stored
    reopened = Browser(profile, root)
    try:
        reopened.start(origin + "/signed-in")
        reopened.wait(lambda: any("wrapper_session=fixture-only" in value for value in Fixture.restored))
        reopened.png(root / "04-restored-session.png")
        if args.save_at:
            reopened.key(76, 4)
            reopened.text(origin + "/?fixture_login=1")
            reopened.key(13)
            time.sleep(1)
            reopened.png(root / "05-autofill.png")
            reopened.key(13)
            reopened.wait(lambda: len(Fixture.posted) == 2)
            assert Fixture.posted[-1] == {"username": ["wrapper@example.invalid"], "password": ["Fixture-é-🔐"]}, Fixture.posted
        reopened.send({"Resize": {"browser_id": 1, "width": 620, "height": 600, "scale_factor": 1.0}})
        reopened.wait(lambda: reopened.frame[0]["width"] == 620)
        reopened.png(root / "05-resized.png")
    finally:
        reopened.close()
        server.shutdown()
    result = {"native_chrome": True, "unicode_form_input": True, "single_submission": True, "password_saved": bool(stored), "password_autofill_after_restart": bool(args.save_at), "native_tab_and_copy_shortcuts": bool(args.save_at), "cookie_survives_restart": True, "profile_lock": True, "resize": True, "google_sync": "requires user Google sign-in; not exercised", "artifacts": str(root)}
    (root / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
