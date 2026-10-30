"""SDTEST-2810 / SDUC-562: execute discovery against disposable CLI histories.

Run with python3 scripts/test-remote-session-discovery.py. No network, credentials,
provider processes, or real home directory are used.
"""
import io
import json
import os
from pathlib import Path
import runpy
import tempfile
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "crates/shelldeck-core/src/agent_remote_discovery.py"


def write_json(path, value):
    path.write_text(json.dumps(value) + "\n")


with tempfile.TemporaryDirectory(prefix="shelldeck-native-fixture-") as folder:
    root = Path(folder)
    claude = root / ".claude/projects/-tmp-repo"
    claude.mkdir(parents=True)
    write_json(claude / "claude-123.jsonl", {
        "type": "user", "sessionId": "claude-123", "cwd": "/tmp/work space",
        "message": {"content": "Fix session reconnect"},
    })
    codex = root / ".codex/sessions/2026/10/30"
    codex.mkdir(parents=True)
    write_json(codex / "rollout-codex-123.jsonl", {
        "type": "session_meta",
        "payload": {"id": "codex-123", "cwd": "/tmp/codex", "source": "cli"},
    })
    write_json(root / ".codex/session_index.jsonl", {
        "id": "codex-123", "thread_name": "Fresh title", "updated_at": "2026-10-30T00:00:00Z",
    })
    write_json(codex / "rollout-subagent.jsonl", {
        "type": "session_meta",
        "payload": {"id": "codex-child", "cwd": "/tmp/codex", "source": {"subagent": {}}},
    })
    jcode = root / ".jcode/sessions"
    jcode.mkdir(parents=True)
    write_json(jcode / "session_fox_1_abcd.json", {
        "id": "session_fox_1_abcd", "working_dir": "/tmp/jcode", "title": None,
        "provider_key": "deepseek", "model": "deepseek-v4-flash",
        "messages": [{"role": "user", "content": [{"type": "text", "text": "Resume my work"}]}],
    })
    write_json(jcode / "unsafe.json", {"id": "--evil", "working_dir": "/tmp/jcode"})
    write_json(jcode / "parent-path.json", {"id": "session_bad_1", "working_dir": "/tmp/../other"})
    (jcode / "large.json").write_bytes(b" " * (8 * 1024 * 1024 + 1))
    (jcode / "symlink.json").symlink_to(root / ".codex/session_index.jsonl")
    output = io.BytesIO()
    stdout = io.TextIOWrapper(output, encoding="utf-8")
    with patch.object(Path, "home", return_value=root), patch.dict(os.environ, {}, clear=True), patch("sys.stdout", stdout):
        runpy.run_path(str(SCRIPT), run_name="__main__")
        stdout.flush()
    inventory = json.loads(output.getvalue())
    rows = {row["provider"]: row for row in inventory["sessions"]}
    assert len(inventory["sessions"]) == 3, inventory
    assert rows["claude"]["session_id"] == "claude-123"
    assert rows["claude"]["workdir"] == "/tmp/work space"
    assert rows["codex"]["title"] == "Fresh title"
    assert rows["jcode"]["title"] == "Resume my work"
    assert rows["jcode"]["model"] == "deepseek-v4-flash"
    assert not inventory["truncated"] and inventory["next_cursor"] is None and len(inventory["warnings"]) == 2, inventory
    print("Native metadata fixture passed: provider formats, index titles, subagent exclusion, "
          "invalid IDs/paths, malformed history and symlink exclusion.")
