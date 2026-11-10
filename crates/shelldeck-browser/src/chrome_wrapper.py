"""Full Chrome on an app-owned X display; never opens the desktop Chrome profile.

This Linux preview carries Chrome's *whole* UI, including its own tabs, password
manager and browser popups. Only explicit user input crosses this private display.
The CEF protocol's page inspection/agent commands are deliberately unavailable.
"""
import argparse
import ctypes as C
import fcntl
import hashlib
import json
import os
from pathlib import Path
import queue
import select
import secrets
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import threading
import time

LIMIT = 4096
MAX_HEADER = 16 * 1024 * 1024
MAX_TEXT = 1024 * 1024


def packet(message, payload=b""):
    header = json.dumps({"message": message, "payload_len": len(payload)}, separators=(",", ":")).encode()
    sys.stdout.buffer.write(struct.pack("<I", len(header)) + header + payload)
    sys.stdout.buffer.flush()


def read_exact(stream, length):
    result = bytearray()
    while len(result) < length:
        part = stream.read(length - len(result))
        if not part:
            raise EOFError()
        result.extend(part)
    return result


def commands(target):
    try:
        while True:
            length, = struct.unpack("<I", read_exact(sys.stdin.buffer, 4))
            if not 0 < length <= MAX_HEADER:
                raise ValueError("Invalid browser command size")
            envelope = json.loads(read_exact(sys.stdin.buffer, length))
            if envelope.get("payload_len", 0):
                raise ValueError("Chrome commands cannot contain frame payloads")
            target.put(envelope["message"], timeout=5)
    except (EOFError, BrokenPipeError):
        target.put("Shutdown")
    except Exception as error:
        target.put({"Failed": {"message": str(error)}})


# A fresh single-threaded launcher sets parent-death handling before exec. No
# preexec_fn is run in the multithreaded protocol helper, and no global pkill.
CHILD = """import ctypes,os,signal,sys
ctypes.CDLL(None).prctl(1,signal.SIGTERM)
if os.getppid()!=int(sys.argv[1]):sys.exit(1)
os.execvp(sys.argv[2],sys.argv[2:])
"""


def spawn_child(argv, **kwargs):
    return subprocess.Popen([sys.executable, "-c", CHILD, str(os.getpid()), *argv], start_new_session=True, **kwargs)


class Common(C.Structure):
    _fields_ = [("type", C.c_int), ("serial", C.c_ulong), ("send_event", C.c_int), ("display", C.c_void_p)]


class MapRequest(C.Structure):
    _fields_ = Common._fields_ + [("parent", C.c_ulong), ("window", C.c_ulong)]


class MapNotify(C.Structure):
    _fields_ = Common._fields_ + [("event", C.c_ulong), ("window", C.c_ulong), ("override_redirect", C.c_int)]


class Configure(C.Structure):
    _fields_ = Common._fields_ + [("parent", C.c_ulong), ("window", C.c_ulong), ("x", C.c_int), ("y", C.c_int), ("width", C.c_int), ("height", C.c_int), ("border", C.c_int), ("above", C.c_ulong), ("detail", C.c_int), ("mask", C.c_ulong)]


class SelectionRequest(C.Structure):
    _fields_ = Common._fields_ + [("owner", C.c_ulong), ("requestor", C.c_ulong), ("selection", C.c_ulong), ("target", C.c_ulong), ("property", C.c_ulong), ("time", C.c_ulong)]


class Selection(C.Structure):
    _fields_ = Common._fields_ + [("requestor", C.c_ulong), ("selection", C.c_ulong), ("target", C.c_ulong), ("property", C.c_ulong), ("time", C.c_ulong)]


class ClientMessage(C.Structure):
    _fields_ = Common._fields_ + [("window", C.c_ulong), ("message_type", C.c_ulong), ("format", C.c_int), ("data", C.c_long * 5)]


class XEvent(C.Union):
    _fields_ = [("type", C.c_int), ("map_request", MapRequest), ("mapped", MapNotify), ("configure", Configure), ("selection_request", SelectionRequest), ("selection", Selection), ("client", ClientMessage), ("pad", C.c_long * 24)]


class Changes(C.Structure):
    _fields_ = [("x", C.c_int), ("y", C.c_int), ("width", C.c_int), ("height", C.c_int), ("border", C.c_int), ("sibling", C.c_ulong), ("stack", C.c_int)]


class XImage(C.Structure):
    _fields_ = [("width", C.c_int), ("height", C.c_int), ("xoffset", C.c_int), ("format", C.c_int), ("data", C.c_void_p), ("byte_order", C.c_int), ("bitmap_unit", C.c_int), ("bitmap_bit_order", C.c_int), ("bitmap_pad", C.c_int), ("depth", C.c_int), ("stride", C.c_int), ("bits", C.c_int), ("red", C.c_ulong), ("green", C.c_ulong), ("blue", C.c_ulong)]


class ChromeDisplay:
    def __init__(self, profile):
        self.profile = profile
        self.width, self.height, self.scale = 800, 600, 1.0
        self.browser_id = None
        self.chrome = None
        self.server = None
        self.main = None
        self.windows = set()
        self.last_frame = None
        self.last_title = None
        self.clipboard = b""
        self.clipboard_windows = {}
        self.pending_text = []
        self.paste_until = 0
        self.copy_pending = False
        self.input_until = 0
        self.keys = set()
        self.mods = set()
        self.pressed_buttons = set()
        self.dirty = True
        self.damage_ids = set()
        self.authority = None
        self.log = (profile / "chrome-wrapper.log").open("ab", buffering=0)
        self.chrome_path = shutil.which("google-chrome") or shutil.which("google-chrome-stable")
        if not self.chrome_path or not shutil.which("Xvfb"):
            raise RuntimeError("Google Chrome and Xvfb must be installed for the Linux Chrome preview")
        self.version = subprocess.check_output([self.chrome_path, "--version"], timeout=10, text=True).strip()
        self.lock = (profile / ".shelldeck-wrapper.lock").open("a+")
        try:
            fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError("This Chrome profile is open in another sidebrowser. Close that browser pane, then retry.")
        self.authority = profile / (".chrome-display-" + secrets.token_hex(8) + ".auth")
        cookie = secrets.token_bytes(16)
        def authority(number):
            parts = [b"", number.encode(), b"MIT-MAGIC-COOKIE-1", cookie]
            self.authority.write_bytes(struct.pack(">H", 65535) + b"".join(struct.pack(">H", len(v)) + v for v in parts))
            self.authority.chmod(0o600)
        authority("")
        read_fd, write_fd = os.pipe()
        try:
            self.server = spawn_child(["Xvfb", "-displayfd", str(write_fd), "-screen", "0", "4096x4096x24", "-nolisten", "tcp", "-noreset", "-auth", str(self.authority)], pass_fds=(write_fd,), stdout=self.log, stderr=self.log)
            os.close(write_fd)
            write_fd = None
            if not select.select([read_fd], [], [], 8)[0]:
                raise RuntimeError("The private Chrome display did not start")
            number = os.read(read_fd, 32).decode().strip()
            if not number.isdecimal():
                raise RuntimeError("The private Chrome display returned an invalid address")
            self.display_name = ":" + number
            authority(number)
            os.environ["XAUTHORITY"] = str(self.authority)
        finally:
            os.close(read_fd)
            if write_fd is not None:
                os.close(write_fd)
        self.x = C.CDLL("libX11.so.6")
        self.xt = C.CDLL("libXtst.so.6")
        self.bindings()
        self.d = self.x.XOpenDisplay(self.display_name.encode())
        if not self.d:
            raise RuntimeError("Could not connect to the private Chrome display")
        self.root = self.x.XDefaultRootWindow(self.d)
        # Own the window-manager role on *our* display. Chrome popups and dialog
        # windows then remain in the same captured surface as the browser UI.
        self.x.XSelectInput(self.d, self.root, (1 << 20) | (1 << 19) | (1 << 22))
        self.wm = self.x.XCreateSimpleWindow(self.d, self.root, 0, 0, 1, 1, 0, 0, 0)
        supported = ["_NET_SUPPORTING_WM_CHECK", "_NET_WM_NAME", "_NET_ACTIVE_WINDOW", "_NET_WM_STATE", "_NET_WM_STATE_MAXIMIZED_VERT", "_NET_WM_STATE_MAXIMIZED_HORZ", "_NET_WM_STATE_FULLSCREEN", "_NET_WORKAREA", "_NET_CLIENT_LIST", "_NET_NUMBER_OF_DESKTOPS", "_NET_CURRENT_DESKTOP", "_NET_DESKTOP_GEOMETRY"]
        self.property(self.root, "_NET_SUPPORTED", "ATOM", [self.atom(name) for name in supported])
        for window in [self.root, self.wm]:
            self.property(window, "_NET_SUPPORTING_WM_CHECK", "WINDOW", [self.wm])
        self.property(self.wm, "_NET_WM_NAME", "UTF8_STRING", b"Inklura Chrome display")
        self.property(self.root, "_NET_NUMBER_OF_DESKTOPS", "CARDINAL", [1])
        self.property(self.root, "_NET_CURRENT_DESKTOP", "CARDINAL", [0])
        self.resize(800, 600, 1.0)
        self.x.XSync(self.d, 0)
        try:
            self.damage = C.CDLL("libXdamage.so.1")
            self.damage.XDamageQueryExtension.argtypes = [C.c_void_p, C.POINTER(C.c_int), C.POINTER(C.c_int)]
            self.damage.XDamageCreate.argtypes = [C.c_void_p, C.c_ulong, C.c_int]
            self.damage.XDamageCreate.restype = C.c_ulong
            self.damage.XDamageSubtract.argtypes = [C.c_void_p, C.c_ulong, C.c_ulong, C.c_ulong]
            self.damage_event, error = C.c_int(), C.c_int()
            if not self.damage.XDamageQueryExtension(self.d, C.byref(self.damage_event), C.byref(error)):
                self.damage = None
        except OSError:
            self.damage = None

    def bindings(self):
        def bind(lib, name, args, result=None):
            fn = getattr(lib, name)
            fn.argtypes = args
            if result is not None:
                fn.restype = result
        p, u, i, ul = C.c_void_p, C.c_ulong, C.c_int, C.POINTER(C.c_ulong)
        bind(self.x, "XOpenDisplay", [C.c_char_p], p)
        bind(self.x, "XDefaultRootWindow", [p], u)
        bind(self.x, "XInternAtom", [p, C.c_char_p, i], u)
        bind(self.x, "XCreateSimpleWindow", [p, u, i, i, C.c_uint, C.c_uint, C.c_uint, u, u], u)
        bind(self.x, "XSelectInput", [p, u, C.c_long])
        bind(self.x, "XChangeProperty", [p, u, u, u, i, i, p, i])
        bind(self.x, "XGetWindowProperty", [p, u, u, C.c_long, C.c_long, i, u, ul, C.POINTER(i), ul, ul, C.POINTER(p)])
        bind(self.x, "XFree", [p])
        bind(self.x, "XGetGeometry", [p, u, ul, C.POINTER(i), C.POINTER(i), C.POINTER(C.c_uint), C.POINTER(C.c_uint), C.POINTER(C.c_uint), C.POINTER(C.c_uint)])
        bind(self.x, "XMoveResizeWindow", [p, u, i, i, C.c_uint, C.c_uint])
        bind(self.x, "XMoveWindow", [p, u, i, i])
        bind(self.x, "XMapWindow", [p, u])
        bind(self.x, "XDestroyWindow", [p, u])
        bind(self.x, "XRaiseWindow", [p, u])
        bind(self.x, "XSetInputFocus", [p, u, i, u])
        bind(self.x, "XGetInputFocus", [p, ul, C.POINTER(i)])
        bind(self.x, "XConfigureWindow", [p, u, C.c_uint, C.POINTER(Changes)])
        bind(self.x, "XPending", [p], i)
        bind(self.x, "XNextEvent", [p, C.POINTER(XEvent)])
        bind(self.x, "XFlush", [p])
        bind(self.x, "XSync", [p, i])
        bind(self.x, "XCloseDisplay", [p])
        bind(self.x, "XGetImage", [p, u, i, i, C.c_uint, C.c_uint, u, i], C.POINTER(XImage))
        bind(self.x, "XDestroyImage", [C.POINTER(XImage)])
        bind(self.x, "XStringToKeysym", [C.c_char_p], u)
        bind(self.x, "XKeysymToKeycode", [p, u], C.c_ubyte)
        bind(self.x, "XKeycodeToKeysym", [p, C.c_ubyte, i], u)
        bind(self.x, "XSetSelectionOwner", [p, u, u, u])
        bind(self.x, "XConvertSelection", [p, u, u, u, u, u])
        bind(self.x, "XSendEvent", [p, u, i, C.c_long, C.POINTER(XEvent)])
        bind(self.xt, "XTestFakeMotionEvent", [p, i, i, i, u])
        bind(self.xt, "XTestFakeButtonEvent", [p, C.c_uint, i, u])
        bind(self.xt, "XTestFakeKeyEvent", [p, C.c_uint, i, u])
        # Window destruction races with popup/capture events during normal close.
        self.error_handler = C.CFUNCTYPE(i, p, p)(lambda _display, _error: 0)
        bind(self.x, "XSetErrorHandler", [C.c_void_p], p)
        self.x.XSetErrorHandler(C.cast(self.error_handler, p))

    def atom(self, name):
        return self.x.XInternAtom(self.d, name.encode(), 0)

    def property(self, window, name, kind, value):
        if isinstance(value, bytes):
            data, bits, length = C.create_string_buffer(value), 8, len(value)
        else:
            data, bits, length = (C.c_ulong * len(value))(*value), 32, len(value)
        self.x.XChangeProperty(self.d, window, self.atom(name), self.atom(kind), bits, 0, C.cast(data, C.c_void_p), length)

    def get_property(self, window, name):
        typ, fmt, count, after, data = C.c_ulong(), C.c_int(), C.c_ulong(), C.c_ulong(), C.c_void_p()
        self.x.XGetWindowProperty(self.d, window, self.atom(name), 0, MAX_TEXT // 4, 0, 0, C.byref(typ), C.byref(fmt), C.byref(count), C.byref(after), C.byref(data))
        try:
            if not data.value or after.value:
                return b""
            if fmt.value == 8:
                return C.string_at(data, min(count.value, MAX_TEXT))
            if fmt.value == 32:
                return list(C.cast(data, C.POINTER(C.c_ulong))[:count.value])
            return b""
        finally:
            if data.value:
                self.x.XFree(data)

    def geometry(self, window):
        root, a, b, w, h, border, depth = C.c_ulong(), C.c_int(), C.c_int(), C.c_uint(), C.c_uint(), C.c_uint(), C.c_uint()
        if not self.x.XGetGeometry(self.d, window, C.byref(root), C.byref(a), C.byref(b), C.byref(w), C.byref(h), C.byref(border), C.byref(depth)):
            return (0, 0, 0, 0)
        return a.value, b.value, w.value, h.value

    def resize(self, width, height, scale):
        self.scale = min(max(float(scale), .25), 4.0, LIMIT / max(int(width), int(height), 1))
        self.width = max(1, min(LIMIT, round(int(width) * self.scale)))
        self.height = max(1, min(LIMIT, round(int(height) * self.scale)))
        if hasattr(self, "root"):
            self.property(self.root, "_NET_WORKAREA", "CARDINAL", [0, 0, self.width, self.height])
            self.property(self.root, "_NET_DESKTOP_GEOMETRY", "CARDINAL", [self.width, self.height])
            if self.main:
                self.x.XMoveResizeWindow(self.d, self.main, 0, 0, self.width, self.height)
            for window in self.windows - {self.main}:
                self.contain(window)
            self.x.XFlush(self.d)
        self.last_frame = None
        self.dirty = True

    def contain(self, window):
        a, b, w, h = self.geometry(window)
        if w and h:
            if w > self.width or h > self.height:
                self.x.XMoveResizeWindow(self.d, window, max(0, min(a, self.width - w)), max(0, min(b, self.height - h)), min(w, self.width), min(h, self.height))
            else:
                self.x.XMoveWindow(self.d, window, max(0, min(a, self.width - w)), max(0, min(b, self.height - h)))

    def launch(self, browser_id, url, width, height, scale):
        if self.chrome and self.chrome.poll() is None:
            raise RuntimeError("Use Chrome's own tab bar to open another tab in this preview")
        self.browser_id = browser_id
        self.main = None
        self.windows.clear()
        self.resize(width, height, scale)
        env = os.environ.copy()
        env.pop("WAYLAND_DISPLAY", None)
        env.update(DISPLAY=self.display_name, XDG_SESSION_TYPE="x11")
        args = [self.chrome_path, "--user-data-dir=" + str(self.profile / "google-chrome"), "--ozone-platform=x11", "--no-first-run", "--no-default-browser-check", "--window-position=0,0", f"--window-size={self.width},{self.height}", f"--force-device-scale-factor={self.scale}", "--disk-cache-size=134217728"]
        if os.getenv("SHELLDECK_CHROME_TEST_PROFILE") == "1":
            if not self.profile.resolve().is_relative_to(Path(tempfile.gettempdir()).resolve()):
                raise RuntimeError("The dummy-password fixture requires a temporary profile")
            args += ["--password-store=basic", "--disable-background-networking"]
        if (self.profile / "google-chrome/Default/Preferences").is_file() and url == "about:blank":
            args.append("--restore-last-session")
        else:
            args.append(url)
        self.chrome = spawn_child(args, env=env, stdout=self.log, stderr=self.log)
        self.initial_url = url
        self.last_title = None
        packet({"Created": {"browser_id": browser_id, "parent_id": None}})
        self.state("Google Chrome")

    def state(self, title):
        if self.browser_id is not None:
            packet({"State": {"state": {"browser_id": self.browser_id, "url": self.initial_url, "title": title, "loading": False, "can_go_back": False, "can_go_forward": False}}})

    def keycode(self, name):
        return self.x.XKeysymToKeycode(self.d, self.x.XStringToKeysym(name.encode()))

    def key(self, name, down):
        code = self.keycode(name)
        if code:
            self.xt.XTestFakeKeyEvent(self.d, code, int(down), 0)
            (self.keys.add if down else self.keys.discard)(name)

    def modifiers(self, flags):
        desired = {name for mask, name in [(2, "Shift_L"), (4, "Control_L"), (8, "Alt_L"), (128, "Super_L")] if flags & mask}
        for name in self.mods - desired:
            self.key(name, False)
        for name in desired - self.mods:
            self.key(name, True)
        self.mods = desired

    def focus(self):
        current, revert = C.c_ulong(), C.c_int()
        self.x.XGetInputFocus(self.d, C.byref(current), C.byref(revert))
        if self.main and current.value in (0, 1, self.root, self.wm):
            self.x.XSetInputFocus(self.d, self.main, 1, 0)
            self.x.XFlush(self.d)

    def shortcut(self, key, modifiers=("Control_L",)):
        self.focus()
        if key == "l" and "Control_L" in modifiers:
            self.input_until = time.monotonic() + .18
        held = set(self.mods)
        for name in held:
            self.key(name, False)
        for name in modifiers:
            self.key(name, True)
        self.key(key, True)
        self.key(key, False)
        for name in reversed(modifiers):
            self.key(name, False)
        for name in held:
            self.key(name, True)
        self.x.XFlush(self.d)

    def keyboard(self, event):
        self.focus()
        if event["kind"] == "Char":
            return  # XTest's real Return already submits the form, exactly once.
        flags, vk = event["modifiers"], event["windows_key_code"]
        if event["kind"] == "RawKeyDown" and vk == 76 and flags & 4:
            self.input_until = time.monotonic() + .18
        self.modifiers(flags)
        if (vk == 32 or 48 <= vk <= 90) and not flags & (4 | 8 | 128) and event["kind"] == "RawKeyDown":
            return  # Printable text comes through the host's text/IME handler.
        name = {8: "BackSpace", 9: "Tab", 13: "Return", 27: "Escape", 32: "space", 33: "Prior", 34: "Next", 35: "End", 36: "Home", 37: "Left", 38: "Up", 39: "Right", 40: "Down", 45: "Insert", 46: "Delete"}.get(vk)
        if not name and 48 <= vk <= 90:
            name = chr(vk).lower()
        if not name and 112 <= vk <= 135:
            name = "F" + str(vk - 111)
        if name:
            self.key(name, event["kind"] == "RawKeyDown")
        if vk in (67, 88) and flags & 4 and event["kind"] == "RawKeyDown":
            self.copy_pending = time.monotonic() + .08
        self.x.XFlush(self.d)

    def type_native_text(self, text):
        # Ordinary typing stays native and immediate. Clipboard transport is
        # reserved for large pastes and Unicode absent from the private keymap.
        if len(text) > 256:
            return False
        strokes = []
        for char in text:
            symbol = ord(char)
            if not 32 <= symbol < 127:
                return False
            code = self.x.XKeysymToKeycode(self.d, symbol)
            if not code:
                return False
            levels = [self.x.XKeycodeToKeysym(self.d, code, level) for level in (0, 1)]
            if symbol not in levels:
                return False
            strokes.append((code, levels.index(symbol) == 1))
        self.focus()
        held = set(self.mods)
        for name in held:
            self.key(name, False)
        shifted = False
        for code, shift in strokes:
            if shift != shifted:
                self.key("Shift_L", shift)
                shifted = shift
            self.xt.XTestFakeKeyEvent(self.d, code, 1, 0)
            self.xt.XTestFakeKeyEvent(self.d, code, 0, 0)
        if shifted:
            self.key("Shift_L", False)
        for name in held:
            self.key(name, True)
        self.x.XFlush(self.d)
        return True

    def selection_request(self, request):
        reply = XEvent()
        reply.selection = Selection(31, 0, 1, self.d, request.requestor, request.selection, request.target, request.property, request.time)
        prop = request.property or request.target
        if request.target == self.atom("TARGETS"):
            targets = (C.c_ulong * 4)(self.atom("TARGETS"), self.atom("UTF8_STRING"), self.atom("STRING"), self.atom("text/plain;charset=utf-8"))
            self.x.XChangeProperty(self.d, request.requestor, prop, self.atom("ATOM"), 32, 0, C.cast(targets, C.c_void_p), 4)
        elif request.target in [self.atom("UTF8_STRING"), self.atom("STRING"), self.atom("text/plain;charset=utf-8")]:
            data = C.create_string_buffer(self.clipboard_windows.get(request.owner, self.clipboard))
            value = self.clipboard_windows.get(request.owner, self.clipboard)
            self.paste_until = time.monotonic() + .15
            self.x.XChangeProperty(self.d, request.requestor, prop, request.target, 8, 0, C.cast(data, C.c_void_p), len(value))
        else:
            prop = 0
        reply.selection.property = prop
        self.x.XSendEvent(self.d, request.requestor, 0, 0, C.byref(reply))
        self.x.XFlush(self.d)

    def events(self):
        for _ in range(min(self.x.XPending(self.d), 256)):
            event = XEvent()
            self.x.XNextEvent(self.d, C.byref(event))
            if self.damage and event.type == self.damage_event.value:
                # DamageNotify starts with the common header, drawable, damage ID.
                damage_id = C.cast(C.byref(event, C.sizeof(Common) + C.sizeof(C.c_ulong)), C.POINTER(C.c_ulong))[0]
                self.damage.XDamageSubtract(self.d, damage_id, 0, 0)
                self.dirty = True
            elif event.type == 20:  # MapRequest
                window = event.map_request.window
                _, _, width, height = self.geometry(window)
                if self.main is None and width > 100 and height > 100:
                    self.main = window
                    self.x.XMoveResizeWindow(self.d, window, 0, 0, self.width, self.height)
                self.windows.add(window)
                if self.damage and window not in self.damage_ids:
                    self.damage.XDamageCreate(self.d, window, 3)
                    self.damage_ids.add(window)
                self.dirty = True
                self.x.XMapWindow(self.d, window)
                self.contain(window)
                self.x.XSync(self.d, 0)
                if width > 100 and height > 50:
                    self.x.XSetInputFocus(self.d, window, 1, 0)
                    self.property(self.root, "_NET_ACTIVE_WINDOW", "WINDOW", [window])
                self.property(self.root, "_NET_CLIENT_LIST", "WINDOW", list(self.windows))
            elif event.type == 23:  # ConfigureRequest
                e = event.configure
                if e.window == self.main:
                    self.x.XMoveResizeWindow(self.d, e.window, 0, 0, self.width, self.height)
                else:
                    changes = Changes(e.x, e.y, e.width, e.height, e.border, e.above, e.detail)
                    self.x.XConfigureWindow(self.d, e.window, e.mask, C.byref(changes))
                    self.contain(e.window)
            elif event.type == 19:  # Override-redirect Chrome bubbles also map.
                window = event.mapped.window
                if window != self.main and window != self.wm:
                    self.contain(window)
                    if self.damage and window not in self.damage_ids:
                        self.damage.XDamageCreate(self.d, window, 3)
                        self.damage_ids.add(window)
                    self.dirty = True
            elif event.type == 17:
                self.windows.discard(event.mapped.window)
                self.damage_ids.discard(event.mapped.window)
                self.dirty = True
            elif event.type == 30:
                self.selection_request(event.selection_request)
            elif event.type == 31 and event.selection.property:
                text = self.get_property(self.wm, "_INKLURA_COPY")
                if isinstance(text, bytes) and text and self.browser_id is not None:
                    packet({"TextSelectionChanged": {"browser_id": self.browser_id, "selected_text": text.decode("utf-8", "replace"), "range": {"start": 0, "end": 0}}})
            elif event.type == 33 and event.client.message_type == self.atom("_NET_ACTIVE_WINDOW"):
                self.x.XRaiseWindow(self.d, event.client.window)
                self.x.XSetInputFocus(self.d, event.client.window, 1, 0)
        self.x.XFlush(self.d)
        now = time.monotonic()
        if self.copy_pending and now >= self.copy_pending:
            self.copy_pending = False
            self.x.XConvertSelection(self.d, self.atom("CLIPBOARD"), self.atom("UTF8_STRING"), self.atom("_INKLURA_COPY"), self.wm, 0)
            self.x.XFlush(self.d)
        if self.pending_text and now >= max(self.paste_until, self.input_until):
            text = "".join(self.pending_text)
            self.pending_text.clear()
            if self.type_native_text(text):
                return
            self.clipboard = text.encode("utf-8")
            owner = self.x.XCreateSimpleWindow(self.d, self.root, 0, 0, 1, 1, 0, 0, 0)
            self.clipboard_windows[owner] = self.clipboard
            while len(self.clipboard_windows) > 8:
                retired = next(iter(self.clipboard_windows))
                del self.clipboard_windows[retired]
                self.x.XDestroyWindow(self.d, retired)
            self.x.XSetSelectionOwner(self.d, self.atom("CLIPBOARD"), owner, 0)
            self.shortcut("v")
            self.paste_until = now + 2.0

    def command(self, command):
        if command == "Shutdown":
            return False
        if not isinstance(command, dict) or len(command) != 1:
            raise ValueError("Invalid browser command")
        kind, data = next(iter(command.items()))
        if kind == "Failed":
            raise RuntimeError(data["message"])
        if kind == "Create":
            self.launch(data["browser_id"], data["url"], data["width"], data["height"], data["scale_factor"])
            return True
        if data.get("browser_id") != self.browser_id:
            return True
        if kind == "Close":
            self.stop_chrome()
        elif kind == "Resize":
            self.resize(data["width"], data["height"], data["scale_factor"])
        elif kind in ["MouseMove", "MouseButton", "MouseWheel"]:
            if kind == "MouseMove" and data.get("leave"):
                self.xt.XTestFakeMotionEvent(self.d, -1, min(LIMIT - 1, self.width + 32), min(LIMIT - 1, self.height + 32), 0)
                self.x.XFlush(self.d)
                return True
            self.modifiers(data.get("modifiers", 0))
            self.xt.XTestFakeMotionEvent(self.d, -1, max(0, min(self.width - 1, round(data["x"] * self.scale))), max(0, min(self.height - 1, round(data["y"] * self.scale))), 0)
            if kind == "MouseButton":
                button = {"Left": 1, "Middle": 2, "Right": 3}[data["button"]]
                self.xt.XTestFakeButtonEvent(self.d, button, int(data["pressed"]), 0)
                (self.pressed_buttons.add if data["pressed"] else self.pressed_buttons.discard)(button)
            elif kind == "MouseWheel":
                for value, negative, positive in [(data["delta_y"], 5, 4), (data["delta_x"], 7, 6)]:
                    for _ in range(min(12, max(1, abs(value) // 40)) if value else 0):
                        for down in [1, 0]:
                            self.xt.XTestFakeButtonEvent(self.d, negative if value < 0 else positive, down, 0)
            self.x.XFlush(self.d)
        elif kind == "Key":
            self.keyboard(data["event"])
        elif kind == "Text":
            if len(data["text"].encode("utf-8")) > MAX_TEXT:
                raise ValueError("Chrome text input exceeds 1 MiB")
            if sum(len(v.encode("utf-8")) for v in self.pending_text) + len(data["text"].encode("utf-8")) > MAX_TEXT:
                raise ValueError("Pending Chrome text exceeds 1 MiB")
            self.pending_text.append(data["text"])
        elif kind == "Ime":
            event = data["event"]
            if "Commit" in event:
                self.pending_text.append(event["Commit"]["text"][:MAX_TEXT])
        elif kind == "Focus" and data["focused"]:
            self.focus()
        elif kind == "Focus" and not data["focused"]:
            for name in list(self.keys):
                self.key(name, False)
            self.mods.clear()
            for button in list(self.pressed_buttons):
                self.xt.XTestFakeButtonEvent(self.d, button, 0, 0)
            self.pressed_buttons.clear()
            self.x.XFlush(self.d)
        elif kind == "Navigate":
            self.shortcut("l")
            self.pending_text.append(data["url"])
            self.navigate_at = time.monotonic() + .2
        elif kind == "Back":
            self.shortcut("Left", ("Alt_L",))
        elif kind == "Forward":
            self.shortcut("Right", ("Alt_L",))
        elif kind == "Reload":
            self.shortcut("r", ("Control_L", "Shift_L") if data["ignore_cache"] else ("Control_L",))
        elif kind == "Stop":
            self.shortcut("Escape", ())
        elif kind == "DevTools":
            packet({"DevToolsResult": {"browser_id": self.browser_id, "request_id": data["request_id"], "result": None, "error": "Google Chrome preview is manual browsing only; page inspection and agent control are unavailable"}})
        return True

    def frame(self):
        if self.browser_id is None or not self.main:
            return
        image = self.x.XGetImage(self.d, self.root, 0, 0, self.width, self.height, 0xFFFFFFFFFFFFFFFF, 2)
        if not image:
            return
        try:
            info = image.contents
            if info.bits != 32 or info.byte_order != 0 or info.red != 0xFF0000 or info.green != 0xFF00 or info.blue != 0xFF:
                raise RuntimeError("Unsupported Chrome display pixel format")
            raw = C.string_at(info.data, info.stride * self.height)
            if info.stride != self.width * 4:
                raw = b"".join(raw[row * info.stride:row * info.stride + self.width * 4] for row in range(self.height))
            digest = hashlib.blake2b(raw, digest_size=16).digest()
            if self.last_frame != digest:
                self.last_frame = digest
                pixels = bytearray(raw)
                pixels[3::4] = b"\xff" * (self.width * self.height)
                packet({"Paint": {"browser_id": self.browser_id, "kind": "View", "width": self.width, "height": self.height, "dirty_rects": [{"x": 0, "y": 0, "width": self.width, "height": self.height}]}}, pixels)
        finally:
            self.x.XDestroyImage(image)
        title = self.get_property(self.main, "_NET_WM_NAME")
        if isinstance(title, bytes) and title != self.last_title:
            self.last_title = title
            self.state(title.decode("utf-8", "replace") or "Google Chrome")

    def stop_chrome(self):
        if self.chrome:
            if self.chrome.poll() is None:
                self.chrome.terminate()
                try:
                    self.chrome.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    os.killpg(self.chrome.pid, signal.SIGKILL)
                    self.chrome.wait()
            self.chrome = None
        if self.browser_id is not None:
            packet({"Closed": {"browser_id": self.browser_id}})
        self.browser_id = None
        self.main = None
        self.windows.clear()

    def close(self):
        try:
            self.stop_chrome()
        finally:
            if getattr(self, "d", None):
                self.x.XCloseDisplay(self.d)
            if self.server:
                self.server.terminate()
                try:
                    self.server.wait(timeout=.5)
                except subprocess.TimeoutExpired:
                    self.server.kill()
                    self.server.wait()
            self.log.close()
            if hasattr(self, "lock"):
                self.lock.close()
            if self.authority:
                self.authority.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--profile", required=True, type=Path)
    args = parser.parse_args()
    if sys.platform != "linux" or not args.profile.is_absolute():
        raise RuntimeError("The full Chrome preview requires Linux and an absolute app-owned profile")
    args.profile.mkdir(parents=True, exist_ok=True, mode=0o700)
    args.profile.chmod(0o700)
    # Ensure partial startup failures clean up their private display and lock too.
    display = ChromeDisplay.__new__(ChromeDisplay)
    try:
        display.__init__(args.profile)
        packet({"Ready": {"protocol_version": 1, "engine_version": display.version + " (full Chrome preview)"}})
        incoming = queue.Queue(maxsize=256)
        threading.Thread(target=commands, args=(incoming,), daemon=True).start()
        last_frame = 0
        deferred = None
        while True:
            blocked = display.pending_text or time.monotonic() < max(display.paste_until, display.input_until)
            if blocked:
                # Batch adjacent typed characters while the current private
                # paste completes. Stop at focus/navigation keys so text never
                # crosses a field boundary, and never queue unbounded frames.
                if deferred is None:
                    try:
                        command = incoming.get(timeout=.005)
                        if isinstance(command, dict) and "Text" in command:
                            display.command(command)
                        elif isinstance(command, dict) and "Key" in command and command["Key"]["event"]["kind"] == "KeyUp" and 48 <= command["Key"]["event"]["windows_key_code"] <= 90 and chr(command["Key"]["event"]["windows_key_code"]).lower() not in display.keys:
                            pass
                        else:
                            deferred = command
                    except queue.Empty:
                        pass
                else:
                    time.sleep(.005)
            else:
                try:
                    command = deferred if deferred is not None else incoming.get(timeout=.01)
                    deferred = None
                    if not display.command(command):
                        break
                except queue.Empty:
                    pass
            display.events()
            now = time.monotonic()
            if getattr(display, "navigate_at", 0) and now >= max(display.navigate_at, display.paste_until, display.input_until) and not display.pending_text:
                display.navigate_at = 0
                display.shortcut("Return", ())
            if display.chrome and display.chrome.poll() is not None:
                display.stop_chrome()
            if now - last_frame >= 1 / 24 and (display.dirty or not display.damage):
                display.dirty = False
                display.frame()
                last_frame = now
    finally:
        if hasattr(display, "log"):
            display.close()


if __name__ == "__main__":
    try:
        main()
    except (BrokenPipeError, KeyboardInterrupt):
        pass
    except Exception as error:
        try:
            packet({"Error": {"browser_id": None, "message": str(error)}})
        except BrokenPipeError:
            pass
        print("Chrome preview:", str(error), file=sys.stderr)
        sys.exit(1)
