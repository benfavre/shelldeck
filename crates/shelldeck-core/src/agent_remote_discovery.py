# Read-only inventory of native CLI conversation metadata. No CLI is executed.
import base64
import datetime
import hashlib
import json
import os
import re
import stat
import sys
import time
from pathlib import Path

PER_PROVIDER = 200
ENTRY_LIMIT = 20000
READ_BUDGET = 32 * 1024 * 1024
JCODE_READ_BUDGET = 512 * 1024 * 1024
MAX_JCODE_FILE = 4 * 1024 * 1024 * 1024
DEADLINE = time.monotonic() + 12
ID = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.-]{0,255}$")
result = {"version": 1, "sessions": [], "warnings": [], "truncated": False, "next_cursor": None}
seen = set()


def warn(provider, message):
    text = provider + ": " + message
    if text not in result["warnings"]:
        result["warnings"].append(text)


def clean(value, limit):
    if not isinstance(value, str):
        return ""
    value = " ".join("".join(c for c in value if c.isprintable() or c.isspace()).split())
    return value.encode("utf-8")[:limit].decode("utf-8", "ignore")


def millis(value, fallback):
    if isinstance(value, str):
        try:
            parsed = datetime.datetime.fromisoformat(value.replace("Z", "+00:00"))
            return max(0, int(parsed.timestamp() * 1000))
        except (ValueError, OverflowError, OSError):
            pass
    return max(0, int(fallback * 1000))


def text_content(value):
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return " ".join(v.get("text", "") for v in value if isinstance(v, dict)
                        and v.get("type") in ("text", "input_text")
                        and isinstance(v.get("text"), str))
    return ""


def regular(path):
    try:
        return stat.S_ISREG(path.lstat().st_mode)
    except OSError:
        return False


def candidate_key(item):
    # Timestamp plus a fixed-size path identity gives stable keyset continuation.
    return (max(0, int(item[0] * 1000000)), hashlib.sha256(item[1].encode()).hexdigest())


def candidates(root, provider, suffix, max_depth):
    paths = []
    if not root.exists():
        return paths
    if root.is_symlink():
        warn(provider, "session directory is a symbolic link; not scanned")
        return paths
    count = 0
    def walk_error(_):
        warn(provider, "some session directories could not be read")
    for folder, dirs, files in os.walk(root, followlinks=False, onerror=walk_error):
        count += len(dirs)
        if count > ENTRY_LIMIT or time.monotonic() >= DEADLINE:
            result["truncated"] = True
            warn(provider, "scan limit reached; showing a partial inventory")
            break
        depth = len(Path(folder).relative_to(root).parts)
        dirs[:] = [d for d in dirs if d != "subagents" and not d.startswith(".")
                   and not (Path(folder) / d).is_symlink()] if depth < max_depth else []
        for name in files:
            count += 1
            if count > ENTRY_LIMIT or time.monotonic() >= DEADLINE:
                result["truncated"] = True
                warn(provider, "scan limit reached; showing a partial inventory")
                return sorted(paths, key=candidate_key, reverse=True)
            if not name.endswith(suffix) or name.startswith("agent-"):
                continue
            path = Path(folder) / name
            if regular(path):
                try:
                    paths.append((path.stat().st_mtime, str(path)))
                except OSError:
                    warn(provider, "some session files disappeared during refresh")
    return sorted(paths, key=candidate_key, reverse=True)


def lines(path, budget, limit=2 * 1024 * 1024):
    consumed = 0
    with open(path, "rb") as handle:
        while consumed < limit and budget[0] > 0 and time.monotonic() < DEADLINE:
            allowance = min(limit - consumed, budget[0], 2 * 1024 * 1024)
            line = handle.readline(allowance + 1)
            consumed += len(line)
            budget[0] -= len(line)
            if not line or len(line) > allowance:
                break
            try:
                value = json.loads(line)
                if isinstance(value, dict):
                    yield value
            except (ValueError, UnicodeError):
                continue


def codex_titles(root):
    path = root / "session_index.jsonl"
    titles = {}
    if not regular(path):
        return titles
    try:
        # The append-only index may be large; its recent tail is enough for titles.
        with path.open("rb") as handle:
            size = path.stat().st_size
            handle.seek(max(0, size - 2 * 1024 * 1024))
            if size > 2 * 1024 * 1024:
                handle.readline(2 * 1024 * 1024)
            for line in handle:
                if time.monotonic() >= DEADLINE:
                    break
                try:
                    value = json.loads(line)
                    if isinstance(value, dict) and isinstance(value.get("id"), str):
                        titles[value["id"]] = clean(value.get("thread_name"), 240)
                except (ValueError, UnicodeError):
                    pass
    except OSError:
        warn("Codex", "session titles could not be read")
    return titles


def admit(provider, identifier, cwd, title, updated, model):
    if not isinstance(identifier, str) or not ID.fullmatch(identifier):
        return False
    if not isinstance(cwd, str) or not cwd.startswith("/") or ".." in cwd.split("/") or len(cwd.encode("utf-8")) > 4096 or any(not c.isprintable() for c in cwd):
        return False
    key = (provider, identifier)
    if key in seen:
        return True
    seen.add(key)
    result["sessions"].append({"provider": provider, "session_id": identifier,
        "workdir": cwd, "title": clean(title, 240) or identifier,
        "updated_at_ms": updated, "model": clean(model, 256) or None})
    return True


# Select root fields while scanning bounded chunks. Nested strings (including
# screenshot payloads) are skipped without JSON decoding or whole-file buffers.
# Ordinary read() also tolerates an agent replacing/truncating its active file.
STRING_MARK = re.compile(rb'["\\]')
STRUCTURE = re.compile(rb'[{}\[\]"]')
PRIMITIVE_END = re.compile(rb'[,}\]\s]')
JCODE_FIELDS = {"id", "parent_id", "title", "working_dir", "model", "updated_at"}


class MetadataReader:
    def __init__(self, handle):
        self.handle, self.buf, self.pos = handle, b"", 0

    def fill(self):
        if time.monotonic() >= DEADLINE:
            raise TimeoutError("metadata scan deadline")
        self.buf = self.handle.read(65536)
        self.pos = 0
        return bool(self.buf)

    def peek(self):
        while True:
            while self.pos < len(self.buf) and self.buf[self.pos] in b" \t\r\n":
                self.pos += 1
            if self.pos < len(self.buf):
                return self.buf[self.pos]
            if not self.fill():
                return None

    def eat(self, value):
        if self.peek() != value:
            raise ValueError("incomplete native object")
        self.pos += 1

    def string(self, capture):
        self.eat(34)
        pieces = [b'"'] if capture else None
        size = 0
        while True:
            match = STRING_MARK.search(self.buf, self.pos)
            end = match.start() + 1 if match else len(self.buf)
            if capture:
                size += end - self.pos
                if size > 16384:
                    raise ValueError("oversized native metadata")
                pieces.append(self.buf[self.pos:end])
            self.pos = end
            if match:
                if self.buf[end - 1] == 34:
                    return json.loads(b"".join(pieces)) if capture else None
                # Keep an escaped byte with its backslash, including across chunks.
                if self.pos == len(self.buf) and not self.fill():
                    raise ValueError("incomplete native string")
                if capture:
                    pieces.append(self.buf[self.pos:self.pos + 1])
                self.pos += 1
            elif not self.fill():
                raise ValueError("incomplete native string")

    def value(self, capture=False):
        first = self.peek()
        if first == 34:
            return self.string(capture)
        if first in (123, 91):
            stack = [125 if first == 123 else 93]
            self.pos += 1
            while stack:
                match = STRUCTURE.search(self.buf, self.pos)
                if match is None:
                    if not self.fill():
                        raise ValueError("incomplete native container")
                    continue
                token = self.buf[match.start()]
                self.pos = match.start()
                if token == 34:
                    self.string(False)
                    continue
                self.pos += 1
                if token in (123, 91):
                    stack.append(125 if token == 123 else 93)
                    if len(stack) > 256:
                        raise ValueError("excessive native nesting")
                elif stack.pop() != token:
                    raise ValueError("invalid native container")
            return None
        parts = []
        size = 0
        while True:
            match = PRIMITIVE_END.search(self.buf, self.pos)
            end = match.start() if match else len(self.buf)
            size += end - self.pos
            if size > 1024:
                raise ValueError("invalid native primitive")
            parts.append(self.buf[self.pos:end])
            self.pos = end
            if match or not self.fill():
                return json.loads(b"".join(parts))

    def first_message_title(self):
        # Look ahead only enough for a small first message; title is optional.
        prefix = self.buf[self.pos + 1:]
        if len(prefix) < 65536:
            prefix += self.handle.read(65536 - len(prefix))
        self.buf = b'[' + prefix
        self.pos = 0
        try:
            message, _ = json.JSONDecoder().raw_decode(prefix.decode("utf-8", "ignore").lstrip())
            if isinstance(message, dict) and message.get("role") == "user":
                return text_content(message.get("content"))
        except ValueError:
            pass
        return None


def jcode_metadata(filename):
    metadata = {}
    with open(filename, "rb") as handle:
        initial = os.fstat(handle.fileno())
        if initial.st_size == 0 or initial.st_size > MAX_JCODE_FILE:
            raise ValueError("unsupported native file size")
        reader = MetadataReader(handle)
        reader.eat(123)
        while reader.peek() != 125:
            key = reader.string(True)
            reader.eat(58)
            if key == "messages" and reader.peek() == 91 and not metadata.get("title"):
                metadata["fallback_title"] = reader.first_message_title()
            value = reader.value(key in JCODE_FIELDS)
            if key in JCODE_FIELDS:
                metadata[key] = value
            if reader.peek() != 125:
                reader.eat(44)
        reader.eat(125)
        if reader.peek() is not None:
            raise ValueError("trailing native data")
        final = os.fstat(handle.fileno())
        if (initial.st_size, initial.st_mtime_ns) != (final.st_size, final.st_mtime_ns):
            raise ValueError("native metadata changed while reading")
    return metadata


def scan(provider, files, offset, titles=None):
    label = {"claude": "Claude Code", "codex": "Codex", "jcode": "Jcode"}[provider]
    budget = [JCODE_READ_BUDGET if provider == "jcode" else READ_BUDGET]
    found = 0
    position = offset
    for modified, filename in files[offset:]:
        if found >= PER_PROVIDER or budget[0] <= 0 or time.monotonic() >= DEADLINE:
            result["truncated"] = True
            break
        identifier = cwd = title = model = None
        sidechain = False
        updated = max(0, int(modified * 1000))
        try:
            if provider == "jcode":
                size = os.stat(filename).st_size
                if size > MAX_JCODE_FILE:
                    result["truncated"] = True
                    warn(label, "a history exceeds the 4 GiB metadata reader limit")
                    position += 1
                    continue
                budget[0] -= size
                value = jcode_metadata(filename)
                if not isinstance(value, dict) or value.get("parent_id"):
                    position += 1
                    continue
                identifier, cwd = value.get("id"), value.get("working_dir")
                title, model = value.get("title"), value.get("model")
                updated = millis(value.get("updated_at"), modified)
                title = title or value.get("fallback_title")
            else:
                for value in lines(filename, budget):
                    if provider == "claude":
                        if value.get("isSidechain"):
                            continue
                        identifier = identifier or value.get("sessionId")
                        cwd = cwd or value.get("cwd")
                        if value.get("type") == "user" and not title:
                            msg = value.get("message", {})
                            if isinstance(msg, dict):
                                title = text_content(msg.get("content"))
                        if value.get("type") == "custom-title":
                            title = value.get("customTitle") or title
                    else:
                        payload = value.get("payload", {})
                        if not isinstance(payload, dict):
                            continue
                        if value.get("type") == "session_meta":
                            # Subagent histories are not independent user conversations.
                            source = payload.get("source")
                            if isinstance(source, dict) and "subagent" in source:
                                sidechain = True
                                break
                            identifier = payload.get("id") or payload.get("session_id")
                            cwd = payload.get("cwd")
                            title = (titles or {}).get(identifier)
                        if value.get("type") == "event_msg" and payload.get("type") == "user_message" and not title:
                            title = payload.get("message")
                        if value.get("type") == "response_item" and payload.get("type") == "message" and payload.get("role") == "user" and not title:
                            title = text_content(payload.get("content"))
                    if identifier and cwd and title:
                        break
            if sidechain:
                position += 1
                continue
            if admit(provider, identifier, cwd, title, updated, model):
                found += 1
            else:
                warn(label, "some histories lack valid session or workspace metadata")
        except TimeoutError:
            result["truncated"] = True
            warn(label, "metadata deadline reached; load older sessions to continue")
            break
        except (OSError, ValueError, TypeError, UnicodeError, OverflowError):
            warn(label, "some histories could not be read; retry after the agent finishes writing")
        position += 1
    return position


home = Path.home()
claude = Path(os.environ.get("CLAUDE_CONFIG_DIR") or home / ".claude")
codex = Path(os.environ.get("CODEX_HOME") or home / ".codex")
jcode = home / ".jcode"
sources = {
    "claude": candidates(claude / "projects", "Claude Code", ".jsonl", 1),
    "codex": candidates(codex / "sessions", "Codex", ".jsonl", 3),
    "jcode": candidates(jcode / "sessions", "Jcode", ".json", 0),
}
# Keyset cursors remain usable while active sessions append/reorder or new
# sessions appear. Updated/newer files are discovered by Refresh; this traversal
# continues strictly below its prior per-provider anchor without duplicates.
fingerprint = hashlib.sha256(json.dumps([str(home), os.uname().nodename,
    str(claude), str(codex), str(jcode)], separators=(",", ":")).encode()).hexdigest()
anchors = {provider: None for provider in sources}
if len(sys.argv) > 1:
    try:
        cursor = sys.argv[1]
        if len(cursor) > 2048 or not re.fullmatch(r"[A-Za-z0-9_=-]+", cursor):
            raise ValueError("invalid cursor")
        saved = json.loads(base64.urlsafe_b64decode(cursor))
        if not isinstance(saved, dict) or not isinstance(saved.get("anchors"), dict) or saved.get("v") != 2 or saved.get("source") != fingerprint or set(saved["anchors"]) != set(sources):
            raise ValueError("foreign inventory")
        anchors = saved["anchors"]
        for anchor in anchors.values():
            if anchor is not None and (not isinstance(anchor, list) or len(anchor) != 2 or type(anchor[0]) is not int or not 0 <= anchor[0] <= 10**20 or not isinstance(anchor[1], str) or not re.fullmatch(r"[a-f0-9]{64}", anchor[1])):
                raise ValueError("invalid anchor")
    except (ValueError, TypeError, KeyError):
        sys.stdout.write(json.dumps({"version": 1, "error": "This remote session cursor is no longer valid. Refresh this server before loading older sessions."}))
        sys.exit(0)
more = False
for provider, files in sources.items():
    anchor = anchors[provider]
    remaining = files if anchor is None else [item for item in files if candidate_key(item) < tuple(anchor)]
    position = scan(provider, remaining, 0, codex_titles(codex) if provider == "codex" else None)
    if position:
        anchors[provider] = candidate_key(remaining[position - 1])
    more = more or position < len(remaining)
if more:
    result["truncated"] = True
    result["next_cursor"] = base64.urlsafe_b64encode(json.dumps({"v": 2, "source": fingerprint, "anchors": anchors}, separators=(",", ":")).encode()).decode()
result["sessions"].sort(key=lambda row: (-row["updated_at_ms"], row["provider"], row["session_id"]))
payload = json.dumps(result, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
while len(payload) > 4 * 1024 * 1024:
    result["truncated"] = True
    warn("Inventory", "response limit reached; showing recent sessions only")
    result["sessions"].pop()
    payload = json.dumps(result, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
sys.stdout.buffer.write(payload)
