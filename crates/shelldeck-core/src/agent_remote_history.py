"""Read-only native transcript paging. No agent invocation, credentials, or writes."""
import base64
import collections
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import sys

PAGE = 64
CHUNK = 16 * 1024
MAX_VALUE = 128 * 1024 * 1024


class HistoryError(Exception):
    pass


class JsonStream:
    """Decode one JSON value at a time; Jcode's messages array is never materialized."""
    def __init__(self, handle):
        self.handle, self.buf, self.done = handle, '', False
        self.decoder = json.JSONDecoder()

    def fill(self):
        more = self.handle.read(65536)
        self.buf += more
        self.done = not more
        if len(self.buf) > MAX_VALUE:
            raise HistoryError('A single history record exceeds the safe reader limit.')

    def peek(self):
        self.buf = self.buf.lstrip()
        while not self.buf and not self.done:
            self.fill()
            self.buf = self.buf.lstrip()
        return self.buf[:1]

    def eat(self, expected):
        if self.peek() != expected:
            raise HistoryError('The native history file is incomplete or malformed.')
        self.buf = self.buf[1:]

    def value(self):
        self.peek()
        while True:
            try:
                value, end = self.decoder.raw_decode(self.buf)
                # A number can end exactly at a buffer boundary. Fill once before accepting.
                if end == len(self.buf) and not self.done:
                    self.fill()
                    continue
                self.buf = self.buf[end:]
                return value
            except json.JSONDecodeError:
                if self.done:
                    raise HistoryError('The native history file is incomplete or malformed.')
                self.fill()


def records(path, provider):
    with path.open('r', encoding='utf-8') as handle:
        if provider != 'jcode':
            while True:
                line = handle.readline(MAX_VALUE + 1)
                if not line:
                    break
                if len(line) > MAX_VALUE:
                    raise HistoryError('A single history record exceeds the safe reader limit.')
                if line.strip():
                    value = json.loads(line)
                    if not isinstance(value, dict):
                        raise HistoryError('Unexpected native history record.')
                    yield value
            return
        stream = JsonStream(handle)
        stream.eat('{')
        metadata = {}
        while stream.peek() != '}':
            key = stream.value()
            stream.eat(':')
            if key == 'messages':
                stream.eat('[')
                while stream.peek() != ']':
                    yield {'type': 'jcode_message', 'message': stream.value()}
                    if stream.peek() != ']':
                        stream.eat(',')
                stream.eat(']')
            else:
                value = stream.value()
                if key in ('id', 'working_dir', 'model'):
                    metadata[key] = value
            if stream.peek() != '}':
                stream.eat(',')
        stream.eat('}')
        if stream.peek():
            raise HistoryError('Unexpected trailing native history data.')
        yield {'type': 'jcode_metadata', 'payload': metadata}


def locate(provider, session_id):
    home = Path.home()
    if provider == 'jcode':
        root, depth = home / '.jcode' / 'sessions', 0
    elif provider == 'claude':
        root, depth = Path(os.environ.get('CLAUDE_CONFIG_DIR', home / '.claude')) / 'projects', 1
    else:
        root, depth = Path(os.environ.get('CODEX_HOME', home / '.codex')) / 'sessions', 4
    if root.is_symlink() or not root.is_dir():
        raise HistoryError('The native session history directory is unavailable.')
    matches = []
    def walk(folder, remaining):
        with os.scandir(folder) as entries:
            for entry in entries:
                if entry.is_symlink():
                    continue
                if entry.is_dir(follow_symlinks=False) and remaining:
                    walk(Path(entry.path), remaining - 1)
                elif entry.is_file(follow_symlinks=False):
                    name = entry.name
                    exact = (name == session_id + ('.json' if provider == 'jcode' else '.jsonl'))
                    if provider == 'codex':
                        exact = exact or (name.startswith('rollout-') and name.endswith('-' + session_id + '.jsonl'))
                    if exact:
                        matches.append(Path(entry.path))
    walk(root, depth)
    if len(matches) != 1:
        raise HistoryError('The exact native session history was not found uniquely on this server.')
    return matches[0]


def timestamp(value):
    if isinstance(value, (int, float)):
        return int(value if value > 100000000000 else value * 1000)
    if isinstance(value, str):
        try:
            return int(datetime.datetime.fromisoformat(value.replace('Z', '+00:00')).timestamp() * 1000)
        except (ValueError, OverflowError):
            pass
    return None


def stringify(value):
    return value if isinstance(value, str) else json.dumps(value, ensure_ascii=False, indent=2)


def blocks(content, role, at):
    if isinstance(content, str):
        if content:
            yield role, 'Reasoning' if role == 'reasoning' else None, content, at
        return
    if not isinstance(content, list):
        if content is not None:
            yield role, None, stringify(content), at
        return
    for block in content:
        if isinstance(block, str):
            yield role, None, block, at
            continue
        if not isinstance(block, dict):
            yield 'activity', 'Saved content', stringify(block), at
            continue
        kind = block.get('type', '')
        if kind in ('text', 'input_text', 'output_text'):
            yield role, None, stringify(block.get('text', '')), at
        elif kind in ('thinking', 'reasoning', 'summary_text', 'reasoning_trace'):
            yield 'reasoning', 'Reasoning', stringify(block.get('thinking', block.get('text', block.get('content', '')))), at
        elif kind in ('open_a_i_reasoning', 'openai_reasoning'):
            yield from blocks(block.get('summary', []), 'reasoning', at)
        elif kind in ('tool_use', 'tool_call', 'function_call'):
            yield 'tool', str(block.get('name') or 'Tool call'), stringify(block.get('input', block.get('arguments', block))), at
        elif kind in ('tool_result', 'function_call_output'):
            yield 'tool', str(block.get('tool_use_id') or block.get('call_id') or 'Tool result'), stringify(block.get('content', block.get('output', ''))), at
        elif kind in ('image', 'input_image', 'document', 'audio', 'input_audio', 'file'):
            # Never duplicate base64 blobs into chat; retain a visible attachment marker.
            name = block.get('filename') or block.get('name') or block.get('title') or kind
            yield 'activity', 'Attachment', 'Saved attachment: ' + str(name), at
        elif kind == 'redacted_thinking':
            yield 'reasoning', 'Reasoning', 'This reasoning was redacted by the provider.', at
        else:
            yield 'activity', 'Saved content (' + str(kind or 'unknown') + ')', stringify(block), at


def metadata(path, provider, wanted_id, wanted_cwd):
    found = False
    model = None
    skip_events = set()
    candidates = collections.defaultdict(lambda: {'event': [], 'canonical': []})
    scope = None
    for row_index, row in enumerate(records(path, provider)):
        typ = row.get('type')
        payload = row.get('payload', {})
        if provider == 'codex':
            if typ == 'session_meta':
                if payload.get('id', payload.get('session_id')) != wanted_id or payload.get('cwd') != wanted_cwd:
                    raise HistoryError('The native history identity or working directory does not match this session.')
                found = True
            if typ == 'turn_context' or (typ == 'event_msg' and payload.get('type') == 'task_started'):
                scope = payload.get('turn_id') or payload.get('root_turn_id')
            if typ == 'turn_context' and isinstance(payload.get('model'), str):
                model = payload['model']
            source = role = text = None
            if typ == 'response_item' and payload.get('type') == 'message' and payload.get('role') in ('user', 'assistant'):
                source, role = 'canonical', payload['role']
                content = payload.get('content', [])
                if isinstance(content, str):
                    text = content
                elif isinstance(content, list):
                    text = ''.join(block.get('text', '') for block in content if isinstance(block, dict) and block.get('type') in ('text', 'input_text', 'output_text'))
            elif typ == 'response_item' and payload.get('type') == 'reasoning':
                source, role = 'canonical', 'reasoning'
                text = ''.join(block.get('text', '') for block in payload.get('summary', []) if isinstance(block, dict))
            elif typ == 'event_msg' and payload.get('type') == 'agent_reasoning' and isinstance(payload.get('text'), str):
                source, role, text = 'event', 'reasoning', payload['text']
            elif typ == 'event_msg' and payload.get('type') in ('user_message', 'agent_message') and isinstance(payload.get('message'), str):
                source = 'event'
                role = 'user' if payload['type'] == 'user_message' else 'assistant'
                text = payload['message']
            if source is not None and text:
                # Same text in another turn is a distinct message, never a duplicate.
                signature = hashlib.sha256(text.encode('utf-8')).digest()
                candidates[(scope, role, signature)][source].append(row_index)
            if typ == 'event_msg' and payload.get('type') == 'task_complete':
                scope = None
        elif provider == 'claude':
            if row.get('sessionId') is not None and row['sessionId'] != wanted_id:
                raise HistoryError('The native history belongs to a different session.')
            if row.get('sessionId') == wanted_id and row.get('cwd') == wanted_cwd:
                found = True
            message = row.get('message', {})
            if typ == 'assistant' and isinstance(message, dict) and isinstance(message.get('model'), str):
                model = message['model']
        elif typ == 'jcode_metadata':
            if payload.get('id') != wanted_id or payload.get('working_dir') != wanted_cwd:
                raise HistoryError('The native history identity or working directory does not match this session.')
            found = True
            model = payload.get('model') if isinstance(payload.get('model'), str) else None
    if not found:
        raise HistoryError('The native history does not confirm this session and working directory.')
    for (turn, _role, _signature), pair in candidates.items():
        available = pair['event'][:]
        for canonical_index in pair['canonical']:
            if not available:
                break
            event_index = min(available, key=lambda index: abs(index - canonical_index))
            # Legacy files may lack turn IDs. Only adjacent wire records prove
            # a duplicate there; preserve ambiguous matching text rather than
            # deleting a genuine earlier greeting or repeated request.
            if turn is not None or abs(event_index - canonical_index) == 1:
                skip_events.add(event_index)
                available.remove(event_index)
    return model, skip_events


def normalized(path, provider, skip_events):
    for row_index, row in enumerate(records(path, provider)):

        typ = row.get('type')
        at = timestamp(row.get('timestamp'))
        if provider in ('claude', 'jcode'):
            if typ not in ('user', 'assistant', 'jcode_message') or row.get('isSidechain'):
                continue
            msg = row.get('message', {})
            role = msg.get('role', typ)
            if role not in ('user', 'assistant', 'tool'):
                continue
            yield from blocks(msg.get('content'), role, timestamp(msg.get('timestamp')) or at)
            for call in msg.get('tool_calls', []) or []:
                function = call.get('function', call)
                yield 'tool', str(function.get('name') or 'Tool call'), stringify(function.get('arguments', {})), at
        else:
            payload = row.get('payload', {})
            kind = payload.get('type')
            if typ == 'response_item':
                if kind == 'message' and payload.get('role') in ('user', 'assistant', 'tool'):
                    yield from blocks(payload.get('content'), payload['role'], at)
                elif kind == 'reasoning':
                    yield from blocks(payload.get('summary', []), 'reasoning', at)
                elif kind in ('function_call', 'custom_tool_call'):
                    yield 'tool', str(payload.get('name') or 'Tool call'), stringify(payload.get('arguments', payload.get('input', ''))), at
                elif kind in ('function_call_output', 'custom_tool_call_output'):
                    yield 'tool', str(payload.get('call_id') or 'Tool result'), stringify(payload.get('output', '')), at
            elif typ == 'event_msg':
                if kind == 'user_message' and row_index not in skip_events:
                    yield from blocks(payload.get('message'), 'user', at)
                elif kind == 'agent_message' and row_index not in skip_events:
                    yield from blocks(payload.get('message'), 'assistant', at)
                elif kind == 'agent_reasoning' and row_index not in skip_events:
                    yield from blocks(payload.get('text'), 'reasoning', at)
                elif kind in ('error', 'warning') and isinstance(payload.get('message'), str):
                    yield 'activity', kind.capitalize(), payload['message'], at


def chunks(text):
    # Splitting encoded bytes at a valid boundary preserves every character.
    raw = text.encode('utf-8')
    result = []
    offset = 0
    while offset < len(raw):
        end = min(offset + CHUNK, len(raw))
        while end < len(raw) and raw[end] & 0xc0 == 0x80:
            end -= 1
        result.append(raw[offset:end].decode('utf-8'))
        offset = end
    return result or ['']


def run(request):
    provider, session_id, cwd = request['provider'], request['session_id'], request['workdir']
    if provider not in ('claude', 'codex', 'jcode') or not re.fullmatch(r'[A-Za-z0-9_][A-Za-z0-9_.-]{0,255}', session_id):
        raise HistoryError('Invalid native session identity.')
    if not cwd.startswith('/') or '..' in cwd.split('/') or any(ord(c) < 32 for c in cwd):
        raise HistoryError('Invalid native session working directory.')
    position = request.get('position', 'latest')
    boundary = request.get('boundary', 0)
    snapshot = request.get('snapshot')
    pinned = None
    digest_expected = None
    if snapshot is not None:
        match = re.fullmatch(r'v1:(\d{1,16}):([0-9a-f]{64})', snapshot)
        if not match:
            raise HistoryError('Invalid history snapshot. Reload history.')
        pinned, digest_expected = int(match[1]), match[2]
    if position not in ('latest', 'before', 'after') or not isinstance(boundary, int) or boundary < 0:
        raise HistoryError('Invalid history page cursor.')
    if position != 'latest' and (pinned is None or boundary > pinned):
        raise HistoryError('Invalid history page cursor. Reload history.')
    path = locate(provider, session_id)
    stat = path.stat()
    model, skip_events = metadata(path, provider, session_id, cwd)
    hasher = hashlib.sha256(json.dumps([provider, session_id, cwd], ensure_ascii=False).encode())
    selected = collections.deque(maxlen=PAGE)
    total = 0
    for kind, title, text, at in normalized(path, provider, skip_events):
        pieces = chunks(text)
        for part, piece in enumerate(pieces, 1):
            if pinned is not None and total >= pinned:
                break
            entry = dict(index=total, kind=kind, title=title, text=piece, at_ms=at, part=part, parts=len(pieces))
            hasher.update(json.dumps(entry, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode())
            hasher.update(b'\n')
            if position == 'latest' or (position == 'before' and total < boundary):
                selected.append(entry)
            elif position == 'after' and total >= boundary and len(selected) < PAGE:
                selected.append(entry)
            total += 1
        if pinned is not None and total >= pinned:
            break
    after = path.stat()
    if (stat.st_ino, stat.st_size, stat.st_mtime_ns) != (after.st_ino, after.st_size, after.st_mtime_ns):
        raise HistoryError('History changed while it was being read. Retry loading history.')
    digest = hasher.hexdigest()
    if pinned is not None and (total != pinned or digest != digest_expected):
        raise HistoryError('Saved history changed. Reload history to start a fresh snapshot.')
    entries = list(selected)
    start = entries[0]['index'] if entries else min(boundary, total) if position != 'latest' else total
    end = entries[-1]['index'] + 1 if entries else start
    return dict(version=1, provider=provider, session_id=session_id, workdir=cwd, model=model,
                snapshot='v1:%d:%s' % (total, digest), total_entries=total, start=start, end=end,
                entries=entries, warnings=[])


if __name__ == '__main__':
    try:
        request = json.loads(base64.b64decode(sys.argv[1], validate=True))
        response = run(request)
    except (HistoryError, OSError, ValueError, KeyError, TypeError) as exc:
        # Never include source lines, transcript content, or filesystem paths in errors.
        message = str(exc) if isinstance(exc, HistoryError) else 'The native history could not be read. Check the connection and retry.'
        response = {'version': 1, 'error': message}
    print(json.dumps(response, ensure_ascii=False, separators=(',', ':')))
