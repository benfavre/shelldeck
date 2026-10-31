"""Private per-run remote launcher. Input arrives only over SSH stdin.

The temporary MCP adapter speaks to a reverse-forwarded loopback listener. It
does not install software, edit provider/project configuration, or launch Chrome.
"""
import base64
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile

MAX_LAUNCH_BYTES = 4 * 1024 * 1024
PROXY = r'''
import json, pathlib, socket, sys
LIMIT = 16 * 1024 * 1024
def line(stream):
    value = stream.readline(LIMIT + 1)
    if value and (len(value) > LIMIT or not value.endswith(b"\n")):
        raise ValueError("Browser message exceeds its size limit")
    return value
def main():
    cfg = json.loads(pathlib.Path(__file__).with_name("bridge.json").read_text())
    endpoint = cfg["endpoint"]
    host, port = endpoint.rsplit(":", 1)
    capability = cfg["capability"]
    if host != "127.0.0.1" or not 0 < int(port) < 65536 or len(capability) != 64 or any(c not in "0123456789abcdefABCDEF" for c in capability):
        raise ValueError("Invalid scoped browser connection")
    with socket.create_connection((host, int(port)), timeout=3) as connection:
        connection.settimeout(15)
        with connection.makefile("rb") as replies:
            while True:
                raw = line(sys.stdin.buffer)
                if not raw:
                    return
                request = json.loads(raw)
                if not isinstance(request, dict):
                    raise ValueError("Invalid browser request")
                envelope = json.dumps({"capability":capability,"request":request}, separators=(",", ":")).encode() + b"\n"
                if len(envelope) > LIMIT:
                    raise ValueError("Browser request exceeds its size limit")
                connection.sendall(envelope)
                response = line(replies)
                if not response:
                    raise ValueError("ShellDeck browser access has ended")
                if "id" not in request:
                    continue
                if request.get("method") == "initialize":
                    value = json.loads(response)
                    if isinstance(value.get("result"), dict):
                        original = value["result"].get("instructions", "")
                        value["result"]["instructions"] = original + " The browser runs on the ShellDeck desktop, not this SSH server. Localhost URLs refer to the desktop; use a forwarded URL for a service bound to this server's loopback interface."
                        response = json.dumps(value, separators=(",", ":")).encode() + b"\n"
                if len(response) > LIMIT:
                    raise ValueError("Browser response exceeds its size limit")
                sys.stdout.buffer.write(response)
                sys.stdout.buffer.flush()
try:
    main()
except Exception:
    print("ShellDeck browser connection ended or rejected the request", file=sys.stderr)
    sys.exit(1)
'''


def private_file(path, data):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        output.write(data)


def valid_text(value, limit=4096):
    return isinstance(value, str) and len(value.encode()) <= limit and "\0" not in value


def launch():
    if os.name != "posix":
        raise ValueError("Browser sharing requires a POSIX SSH host with Python 3")
    payload = sys.stdin.buffer.read(MAX_LAUNCH_BYTES + 1)
    if len(payload) > MAX_LAUNCH_BYTES:
        raise ValueError("Private browser launch request exceeds its size limit")
    value = json.loads(payload)
    if value.get("version") != 1 or value.get("provider") not in ("claude", "codex"):
        raise ValueError("Unsupported remote browser launch")
    program, args, workdir = value["program"], value["args"], value["workdir"]
    if not valid_text(program) or not program or not isinstance(args, list) or len(args) > 128 or not all(valid_text(arg, 2 * 1024 * 1024) for arg in args):
        raise ValueError("Invalid remote provider arguments")
    if not valid_text(workdir) or not (workdir.startswith("/") or workdir == "~"):
        raise ValueError("Invalid remote working directory")
    endpoint, capability = value["endpoint"], value["capability"]
    host, port = endpoint.rsplit(":", 1)
    if host != "127.0.0.1" or not 0 < int(port) < 65536 or not isinstance(capability, str) or len(capability) != 64 or any(c not in "0123456789abcdefABCDEF" for c in capability):
        raise ValueError("Invalid scoped browser grant")
    remove_env = value.get("remove_env", [])
    if not isinstance(remove_env, list) or len(remove_env) > 32 or not all(valid_text(key, 128) for key in remove_env):
        raise ValueError("Invalid remote environment removal")
    input_bytes = base64.b64decode(value["stdin"], validate=True) if value.get("stdin") is not None else None
    cwd = os.path.expanduser("~") if workdir == "~" else workdir
    if not os.path.isdir(cwd):
        raise ValueError("The original remote working directory is missing")
    env = os.environ.copy()
    for key in remove_env:
        env.pop(key, None)
    home = env.get("HOME", "")
    env["PATH"] = os.pathsep.join([os.path.join(home, ".local/bin"), os.path.join(home, "bin"), "/usr/local/bin", "/opt/homebrew/bin", env.get("PATH", "")])
    child = None

    class Stopped(BaseException):
        def __init__(self, number):
            self.number = number

    def stop(number, _frame):
        # Never re-enter Popen.wait from a signal handler: communicate may hold
        # its waitpid lock. Unwind it first, then terminate in ordinary cleanup.
        for sig in (signal.SIGTERM, signal.SIGHUP):
            signal.signal(sig, signal.SIG_IGN)
        raise Stopped(number)

    def terminate():
        if child is not None and child.poll() is None:
            try:
                os.killpg(child.pid, signal.SIGTERM)
                child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait(timeout=2)
                except (ProcessLookupError, subprocess.TimeoutExpired):
                    pass
            except ProcessLookupError:
                pass

    previous = {sig: signal.signal(sig, stop) for sig in (signal.SIGTERM, signal.SIGHUP)}
    try:
        with tempfile.TemporaryDirectory(prefix="shelldeck-browser-") as directory:
            proxy_path = os.path.join(directory, "proxy.py")
            private_file(proxy_path, PROXY)
            private_file(os.path.join(directory, "bridge.json"), json.dumps({"endpoint":endpoint,"capability":capability}))
            server = {"type":"stdio","command":sys.executable,"args":[proxy_path],"env":{}}
            if value["provider"] == "claude":
                config_path = os.path.join(directory, "claude.json")
                private_file(config_path, json.dumps({"mcpServers":{"shelldeck_browser":server}}))
                args = args + ["--mcp-config", config_path, "--allowedTools", "mcp__shelldeck_browser__*"]
            else:
                # These CLI values contain only private file paths, never the
                # browser capability. JSON strings are valid TOML basic strings.
                extra = [
                    "-c", "mcp_servers.shelldeck_browser.command=" + json.dumps(sys.executable, ensure_ascii=False),
                    "-c", "mcp_servers.shelldeck_browser.args=" + json.dumps([proxy_path], ensure_ascii=False),
                    "-c", "mcp_servers.shelldeck_browser.enabled=true",
                    "-c", "mcp_servers.shelldeck_browser.required=true",
                    "-c", "mcp_servers.shelldeck_browser.startup_timeout_sec=20",
                    "-c", "mcp_servers.shelldeck_browser.tool_timeout_sec=30",
                ]
                offset = 2 if len(args) > 1 and args[:2] == ["exec", "resume"] else 1
                args = args[:offset] + extra + args[offset:]
            child = subprocess.Popen([program] + args, cwd=cwd, env=env, stdin=subprocess.PIPE if input_bytes is not None else subprocess.DEVNULL, start_new_session=True)
            try:
                child.communicate(input_bytes)
            finally:
                terminate()
            return child.returncode if child.returncode >= 0 else 128 - child.returncode
    except Stopped as stopped:
        return 128 + stopped.number
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)


if __name__ == "__main__":
    try:
        sys.exit(launch())
    except (ValueError, KeyError, TypeError, OSError):
        # Do not echo payloads, capabilities, command arguments or tracebacks.
        print("ShellDeck could not prepare the remote browser-enabled agent (Python 3, folder and provider required)", file=sys.stderr)
        sys.exit(126)
