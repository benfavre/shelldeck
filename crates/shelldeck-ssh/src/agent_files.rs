//! Bounded remote text-file access for the agent workspace editor.
//!
//! The remote host needs Python 3 and POSIX descriptor-relative filesystem
//! operations. Paths and editor text travel exclusively as JSON on stdin to a
//! fixed helper; no user-controlled value becomes shell source or argv. Saves
//! are explicit, existing-file-only operations, protected by a content revision
//! and a cooperative inode lock. POSIX has no general atomic compare-and-swap
//! rename: a writer ignoring that lock can still race the last validation and
//! replacement, so callers must present conflicts rather than silently retry.

use crate::session::{ExecResult, SshSession};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DIRECTORY_ENTRIES: usize = 5000;
const MAX_PATH_BYTES: usize = 4096;
// JSON may escape each input byte as six ASCII bytes.
const MAX_PROTOCOL_BYTES: usize = MAX_DOCUMENT_BYTES * 6 + 128 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Directory {
    /// Canonical remote root. Retain this value for subsequent requests so a
    /// remote HOME change cannot silently change the workspace authority.
    pub root: String,
    pub path: String,
    pub entries: Vec<Entry>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Document {
    pub root: String,
    pub path: String,
    pub text: String,
    /// SHA-256 of the exact UTF-8 bytes, including original line endings.
    pub revision: String,
    pub size: u64,
}

#[derive(Debug, Error)]
pub enum AgentFileError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("Remote files require Python 3 with POSIX filesystem support on this SSH host")]
    UnsupportedHost,
    #[error("{message}")]
    Remote { code: String, message: String },
    #[error("Remote file response is invalid: {0}")]
    Protocol(String),
    #[error(transparent)]
    Ssh(#[from] crate::SshError),
}

impl AgentFileError {
    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Remote { code, .. } if code == "conflict" || code == "scope_changed")
    }
}

#[derive(Serialize)]
struct Request<'a> {
    op: &'a str,
    root: &'a str,
    path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_revision: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
}

#[derive(Deserialize)]
struct Response<T> {
    ok: bool,
    data: Option<T>,
    error: Option<RemoteError>,
}

#[derive(Deserialize)]
struct RemoteError {
    code: String,
    message: String,
}

#[async_trait]
trait FileExecutor: Sync {
    async fn execute(&self, command: &str, input: &[u8]) -> Result<ExecResult, AgentFileError>;
}

#[async_trait]
impl FileExecutor for SshSession {
    async fn execute(&self, command: &str, input: &[u8]) -> Result<ExecResult, AgentFileError> {
        Ok(self
            .exec_with_input_bounded(
                command,
                input,
                MAX_PROTOCOL_BYTES,
                MAX_PROTOCOL_BYTES,
                MAX_DIAGNOSTIC_BYTES,
            )
            .await?)
    }
}

/// List one directory, including hidden entries, within the selected SSH root.
/// An empty path or `.` means the root; relative paths are relative to that root.
/// In-root symlink directories may be browsed; symlink files cannot be edited.
/// The caller must apply a timeout covering the complete operation.
pub async fn list(
    session: &SshSession,
    root: &str,
    path: &str,
) -> Result<Directory, AgentFileError> {
    list_with(session, root, path).await
}

async fn list_with(
    executor: &impl FileExecutor,
    root: &str,
    path: &str,
) -> Result<Directory, AgentFileError> {
    let directory: Directory = request(executor, "list", root, path, None, None).await?;
    validate_remote_path(&directory.root)?;
    validate_remote_path(&directory.path)?;
    if !remote_contains(&directory.root, &directory.path) {
        return Err(AgentFileError::Protocol(
            "directory is outside the workspace".into(),
        ));
    }
    if directory.entries.len() > MAX_DIRECTORY_ENTRIES {
        return Err(AgentFileError::Protocol(
            "too many directory entries".into(),
        ));
    }
    for entry in &directory.entries {
        validate_remote_path(&entry.path)?;
        if entry.name.is_empty()
            || entry.name == "."
            || entry.name == ".."
            || entry.name.contains('/')
            || entry.name.contains('\0')
            || entry.name.len() > MAX_PATH_BYTES
        {
            return Err(AgentFileError::Protocol(
                "invalid directory entry name".into(),
            ));
        }
        let expected = format!("{}/{}", directory.path.trim_end_matches('/'), entry.name);
        if entry.path != expected {
            return Err(AgentFileError::Protocol(
                "entry does not belong to its directory".into(),
            ));
        }
    }
    Ok(directory)
}

/// Read an existing regular UTF-8 file without changing any of its bytes.
pub async fn read(
    session: &SshSession,
    root: &str,
    path: &str,
) -> Result<Document, AgentFileError> {
    document_with(session, "read", root, path, None, None).await
}

/// Replace an existing regular file only if its revision still matches. The
/// caller must invoke this only for an explicit user save, never on text change.
pub async fn save(
    session: &SshSession,
    root: &str,
    path: &str,
    expected_revision: &str,
    text: &str,
) -> Result<Document, AgentFileError> {
    document_with(
        session,
        "save",
        root,
        path,
        Some(expected_revision),
        Some(text),
    )
    .await
}

async fn document_with(
    executor: &impl FileExecutor,
    op: &str,
    root: &str,
    path: &str,
    expected_revision: Option<&str>,
    text: Option<&str>,
) -> Result<Document, AgentFileError> {
    let document: Document = request(executor, op, root, path, expected_revision, text).await?;
    validate_remote_path(&document.root)?;
    validate_remote_path(&document.path)?;
    if document.text.len() > MAX_DOCUMENT_BYTES
        || document.text.contains('\0')
        || document.size != document.text.len() as u64
        || !valid_revision(&document.revision)
        || !remote_contains(&document.root, &document.path)
    {
        return Err(AgentFileError::Protocol(
            "invalid text document or revision".into(),
        ));
    }
    Ok(document)
}

fn validate_remote_path(path: &str) -> Result<(), AgentFileError> {
    validate_path(path, false)?;
    if !path.starts_with('/')
        || path.split('/').any(|part| part == "." || part == "..")
        || (path.len() > 1 && path.ends_with('/'))
    {
        return Err(AgentFileError::Protocol(
            "expected a canonical absolute remote path".into(),
        ));
    }
    Ok(())
}

fn remote_contains(root: &str, path: &str) -> bool {
    root == "/"
        || root == path
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn validate_path(path: &str, allow_empty: bool) -> Result<(), AgentFileError> {
    if (!allow_empty && path.is_empty()) || path.contains('\0') || path.len() > MAX_PATH_BYTES {
        return Err(AgentFileError::InvalidRequest(
            "File paths must be nonempty, contain no NUL, and fit within 4096 UTF-8 bytes".into(),
        ));
    }
    Ok(())
}

fn valid_revision(revision: &str) -> bool {
    revision.len() == 64 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
}

async fn request<T: DeserializeOwned>(
    executor: &impl FileExecutor,
    op: &str,
    root: &str,
    path: &str,
    expected_revision: Option<&str>,
    text: Option<&str>,
) -> Result<T, AgentFileError> {
    validate_path(root, false)?;
    validate_path(path, op == "list")?;
    if let Some(revision) = expected_revision {
        if !valid_revision(revision) {
            return Err(AgentFileError::InvalidRequest(
                "Invalid file revision".into(),
            ));
        }
    }
    if let Some(text) = text {
        if text.len() > MAX_DOCUMENT_BYTES || text.contains('\0') {
            return Err(AgentFileError::InvalidRequest(
                "Only UTF-8 text files up to 8 MiB without NUL bytes can be saved".into(),
            ));
        }
    }
    let input = serde_json::to_vec(&Request {
        op,
        root,
        path,
        expected_revision,
        text,
    })
    .map_err(|error| AgentFileError::InvalidRequest(error.to_string()))?;
    if input.len() > MAX_PROTOCOL_BYTES {
        return Err(AgentFileError::InvalidRequest(
            "File request is too large".into(),
        ));
    }
    let output = executor.execute(&helper_command(), &input).await?;
    if output.stdout.len() > MAX_PROTOCOL_BYTES || output.stderr.len() > MAX_DIAGNOSTIC_BYTES {
        return Err(AgentFileError::Protocol(
            "response exceeds its byte limit".into(),
        ));
    }
    if output.exit_code == Some(127) {
        return Err(AgentFileError::UnsupportedHost);
    }
    let response: Response<T> = serde_json::from_slice(&output.stdout).map_err(|_| {
        AgentFileError::Protocol(
            "The SSH host did not return the file helper protocol; check Python 3 availability"
                .into(),
        )
    })?;
    if !response.ok {
        let error = response
            .error
            .ok_or_else(|| AgentFileError::Protocol("missing remote error details".into()))?;
        if error.code == "unsupported_host" {
            return Err(AgentFileError::UnsupportedHost);
        }
        return Err(AgentFileError::Remote {
            code: error.code,
            message: error.message,
        });
    }
    if output.exit_code != Some(0) {
        return Err(AgentFileError::Protocol(
            "file helper did not exit successfully".into(),
        ));
    }
    response
        .data
        .ok_or_else(|| AgentFileError::Protocol("missing response data".into()))
}

fn helper_command() -> String {
    // This quoting applies only to our fixed helper source. Paths/text never
    // enter this string, including malicious filenames containing shell syntax.
    let script = REMOTE_HELPER.replace('\'', "'\\''");
    format!("command -v python3 >/dev/null 2>&1 || exit 127; exec python3 -c '{script}'")
}

const REMOTE_HELPER: &str = r#"
import os, sys, json, stat, hashlib
MAX_FILE = 8 * 1024 * 1024
MAX_PROTOCOL = MAX_FILE * 6 + 128 * 1024
MAX_PATH = 4096
MAX_ENTRIES = 5000

class Refusal(Exception):
    def __init__(self, code, message):
        self.code, self.message = code, message

def refuse(code, message):
    raise Refusal(code, message)

def text_path(value, empty=False):
    if not isinstance(value, str) or (not empty and not value) or '\0' in value:
        refuse('invalid_path', 'Invalid remote file path')
    try:
        encoded = value.encode('utf-8', 'strict')
    except UnicodeError:
        refuse('invalid_path', 'Remote file path is not UTF-8')
    if len(encoded) > MAX_PATH:
        refuse('invalid_path', 'Remote file path exceeds 4096 bytes')
    return value

def within(root, path):
    return os.path.commonpath([root, path]) == root

def same_inode(a, b):
    return (a.st_dev, a.st_ino) == (b.st_dev, b.st_ino)

def fingerprint(value):
    return (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns, value.st_nlink, value.st_mode)

def open_absolute_directory(path):
    # Walk the canonical absolute spelling with NOFOLLOW at every component,
    # rather than trusting realpath across a later symlink substitution.
    if not os.path.isabs(path):
        refuse('invalid_path', 'The remote workspace root must be absolute or ~')
    fd = os.open(os.path.sep, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for part in path.split(os.path.sep):
            if not part:
                continue
            if part in ('.', '..'):
                refuse('invalid_path', 'Invalid canonical path')
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
        return fd
    except BaseException:
        os.close(fd)
        raise

class Scope:
    def __init__(self, root):
        self.root = text_path(os.path.realpath(os.path.expanduser(root)))
        self.fd = open_absolute_directory(self.root)
        self.identity = os.fstat(self.fd)

    def open_directory(self, canonical):
        if not within(self.root, canonical):
            refuse('out_of_root', 'This path is outside the selected remote workspace')
        relative = os.path.relpath(canonical, self.root)
        fd = os.dup(self.fd)
        try:
            if relative != '.':
                for part in relative.split(os.path.sep):
                    if part in ('', '.', '..'):
                        refuse('invalid_path', 'Invalid remote directory')
                    child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
                    os.close(fd)
                    fd = child
            return fd
        except BaseException:
            os.close(fd)
            raise

    def raw_path(self, path):
        expanded = os.path.expanduser(path) if path else self.root
        return expanded if os.path.isabs(expanded) else os.path.join(self.root, expanded)

    def validate(self, canonical, descriptor):
        # Refuse if either the selected root or the opened parent moved while
        # the request was in flight. The descriptors still fence all access.
        try:
            root_fd = open_absolute_directory(self.root)
            try:
                if not same_inode(self.identity, os.fstat(root_fd)):
                    refuse('scope_changed', 'The remote workspace root changed; reopen it')
            finally:
                os.close(root_fd)
            if os.path.realpath(canonical) != canonical:
                refuse('scope_changed', 'The remote directory changed; reopen the file')
            fresh = self.open_directory(canonical)
            try:
                if not same_inode(os.fstat(descriptor), os.fstat(fresh)):
                    refuse('scope_changed', 'The remote directory changed; reopen the file')
            finally:
                os.close(fresh)
        except OSError:
            refuse('scope_changed', 'The remote directory changed; reopen the file')

    def close(self):
        os.close(self.fd)

def listing(scope, request):
    canonical = text_path(os.path.realpath(scope.raw_path(request['path'])))
    fd = scope.open_directory(canonical)
    entries, truncated = [], False
    try:
        with os.scandir(fd) as iterator:
            for entry in iterator:
                if len(entries) >= MAX_ENTRIES:
                    truncated = True
                    break
                name = text_path(entry.name)
                path = text_path(os.path.join(canonical, name))
                info = entry.stat(follow_symlinks=False)
                linked = stat.S_ISLNK(info.st_mode)
                directory = stat.S_ISDIR(info.st_mode)
                if linked:
                    target = os.path.realpath(path)
                    if within(scope.root, target):
                        try:
                            target_fd = scope.open_directory(target)
                            os.close(target_fd)
                            directory = True
                        except OSError:
                            pass
                entries.append(dict(name=name, path=path, is_dir=directory, is_symlink=linked, size=info.st_size))
        scope.validate(canonical, fd)
    finally:
        os.close(fd)
    entries.sort(key=lambda item: (not item['is_dir'], item['name'].casefold(), item['name']))
    return dict(root=scope.root, path=canonical, entries=entries, truncated=truncated)

def read_bytes(fd):
    before = os.fstat(fd)
    if not stat.S_ISREG(before.st_mode):
        refuse('not_regular', 'Only regular text files can be opened')
    if before.st_size > MAX_FILE:
        refuse('too_large', 'This file exceeds the 8 MiB editor limit')
    os.lseek(fd, 0, os.SEEK_SET)
    chunks, total = [], 0
    while total <= MAX_FILE:
        chunk = os.read(fd, min(65536, MAX_FILE + 1 - total))
        if not chunk:
            break
        chunks.append(chunk)
        total += len(chunk)
    if total > MAX_FILE:
        refuse('too_large', 'This file exceeds the 8 MiB editor limit')
    after = os.fstat(fd)
    if fingerprint(before) != fingerprint(after):
        refuse('conflict', 'The remote file changed while being read; reopen it')
    content = b''.join(chunks)
    if b'\0' in content:
        refuse('binary', 'Binary files cannot be opened in the text editor')
    try:
        text = content.decode('utf-8', 'strict')
    except UnicodeError:
        refuse('encoding', 'Only UTF-8 files can be opened in the text editor')
    return content, text, after

def document(scope, request):
    raw = scope.raw_path(request['path'])
    parent = text_path(os.path.realpath(os.path.dirname(raw)))
    name = text_path(os.path.basename(raw))
    if name in ('.', '..'):
        refuse('not_regular', 'Choose a regular text file')
    path = text_path(os.path.join(parent, name))
    parent_fd = scope.open_directory(parent)
    fd, temporary = None, None
    try:
        observed = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
        if stat.S_ISLNK(observed.st_mode):
            refuse('symlink', 'Symlink files cannot be opened or saved by this editor')
        if not stat.S_ISREG(observed.st_mode):
            refuse('not_regular', 'Only regular text files can be opened')
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent_fd)
        if not same_inode(observed, os.fstat(fd)):
            refuse('conflict', 'The remote file changed; reopen it')
        saving = request['op'] == 'save'
        if saving:
            import fcntl
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                refuse('conflict', 'Another editor is saving this file; try again after reopening it')
        content, text, original = read_bytes(fd)
        revision = hashlib.sha256(content).hexdigest()
        scope.validate(parent, parent_fd)
        current = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
        if fingerprint(current) != fingerprint(original):
            refuse('conflict', 'The remote file changed; reopen it')
        if saving:
            expected = request.get('expected_revision')
            if expected != revision:
                refuse('conflict', 'The remote file changed since it was opened; reopen it before saving')
            if original.st_nlink != 1:
                refuse('hardlink', 'Hard-linked files cannot be replaced by this editor')
            text = request.get('text')
            if not isinstance(text, str) or '\0' in text:
                refuse('binary', 'Only UTF-8 text without NUL bytes can be saved')
            content = text.encode('utf-8', 'strict')
            if len(content) > MAX_FILE:
                refuse('too_large', 'This file exceeds the 8 MiB editor limit')
            temporary = '.shelldeck-write-' + os.urandom(16).hex()
            output_fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=parent_fd)
            try:
                remaining = memoryview(content)
                while remaining:
                    written = os.write(output_fd, remaining)
                    remaining = remaining[written:]
                target = os.fstat(output_fd)
                if (target.st_uid, target.st_gid) != (original.st_uid, original.st_gid):
                    os.fchown(output_fd, original.st_uid, original.st_gid)
                os.fchmod(output_fd, stat.S_IMODE(original.st_mode))
                os.fsync(output_fd)
                scope.validate(parent, parent_fd)
                current = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
                if fingerprint(current) != fingerprint(original) or current.st_nlink != 1:
                    refuse('conflict', 'The remote file changed during save; reopen it')
                os.replace(temporary, name, src_dir_fd=parent_fd, dst_dir_fd=parent_fd)
                temporary = None
                # The atomic replacement has committed. Some remote filesystems
                # reject directory fsync; that cannot undo a completed rename.
                try:
                    os.fsync(parent_fd)
                except OSError:
                    pass
            finally:
                os.close(output_fd)
            revision = hashlib.sha256(content).hexdigest()
        return dict(root=scope.root, path=path, text=text, revision=revision, size=len(content))
    finally:
        if temporary is not None:
            try:
                os.unlink(temporary, dir_fd=parent_fd)
            except OSError:
                pass
        if fd is not None:
            os.close(fd)
        os.close(parent_fd)

def run():
    required = ('O_DIRECTORY', 'O_NOFOLLOW', 'O_NONBLOCK')
    if os.name != 'posix' or not all(hasattr(os, flag) for flag in required):
        refuse('unsupported_host', 'Remote files require Python 3 with POSIX filesystem support')
    if os.open not in os.supports_dir_fd or os.stat not in os.supports_dir_fd or os.scandir not in os.supports_fd:
        refuse('unsupported_host', 'Remote Python lacks descriptor-relative filesystem support')
    raw = sys.stdin.buffer.read(MAX_PROTOCOL + 1)
    if len(raw) > MAX_PROTOCOL:
        refuse('too_large', 'Remote file request exceeds its byte limit')
    request = json.loads(raw.decode('utf-8', 'strict'))
    if not isinstance(request, dict) or request.get('op') not in ('list', 'read', 'save'):
        refuse('invalid_request', 'Unknown remote file operation')
    text_path(request.get('root'))
    text_path(request.get('path'), request['op'] == 'list')
    scope = Scope(request['root'])
    try:
        return listing(scope, request) if request['op'] == 'list' else document(scope, request)
    finally:
        scope.close()

try:
    response = dict(ok=True, data=run())
except Refusal as error:
    response = dict(ok=False, error=dict(code=error.code, message=error.message))
except FileNotFoundError:
    response = dict(ok=False, error=dict(code='not_found', message='The remote file or directory no longer exists'))
except PermissionError:
    response = dict(ok=False, error=dict(code='permission', message='Permission denied for this remote file or directory'))
except (ValueError, UnicodeError, KeyError, TypeError):
    response = dict(ok=False, error=dict(code='invalid_request', message='Invalid remote file request or non-UTF-8 path'))
except OSError:
    response = dict(ok=False, error=dict(code='filesystem', message='The remote filesystem operation failed; the path may have changed'))
except ImportError:
    response = dict(ok=False, error=dict(code='unsupported_host', message='Remote files require Python 3 with POSIX filesystem support'))
encoded = json.dumps(response, ensure_ascii=False, separators=(',', ':')).encode('utf-8', 'strict')
if len(encoded) > MAX_PROTOCOL:
    encoded = b'{"ok":false,"error":{"code":"too_large","message":"Remote file response exceeds its byte limit"}}'
sys.stdout.buffer.write(encoded)
sys.stdout.buffer.flush()
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use serde_json::{json, Value};
    use std::collections::VecDeque;

    #[derive(Default)]
    struct FakeExecutor {
        calls: Mutex<Vec<(String, Vec<u8>)>>,
        outputs: Mutex<VecDeque<ExecResult>>,
    }

    impl FakeExecutor {
        fn protocol(response: Value) -> Self {
            Self::output(ExecResult {
                stdout: serde_json::to_vec(&response).unwrap(),
                stderr: Vec::new(),
                exit_code: Some(0),
            })
        }

        fn output(output: ExecResult) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                outputs: Mutex::new(VecDeque::from([output])),
            }
        }
    }

    #[async_trait]
    impl FileExecutor for FakeExecutor {
        async fn execute(&self, command: &str, input: &[u8]) -> Result<ExecResult, AgentFileError> {
            self.calls.lock().push((command.into(), input.to_vec()));
            Ok(self
                .outputs
                .lock()
                .pop_front()
                .expect("unexpected filesystem execution or retry"))
        }
    }

    fn document(text: &str) -> Value {
        json!({"ok":true,"data":{
            "root":"/workspace","path":"/workspace/file.txt","text":text,
            "revision":"a".repeat(64),"size":text.len()
        }})
    }

    // SDTEST-2510 — dynamic editor bytes must cross stdin, never shell source.
    #[tokio::test]
    async fn sdtest_2510_editor_payload_stays_in_json_stdin() {
        let text = "$(touch should-never-execute)\n'quoted' `command` \\text\r\nλ";
        let path = "/workspace/$(touch filename-must-not-execute).txt";
        let executor = FakeExecutor::protocol(document(text));
        document_with(
            &executor,
            "save",
            "/workspace",
            path,
            Some(&"a".repeat(64)),
            Some(text),
        )
        .await
        .unwrap();
        let calls = executor.calls.lock();
        assert_eq!(calls.len(), 1);
        let (command, input) = &calls[0];
        assert_eq!(command, &helper_command());
        assert!(!command.contains("should-never-execute"));
        assert!(!command.contains("filename-must-not-execute"));
        let request: Value = serde_json::from_slice(input).unwrap();
        assert_eq!(
            request,
            json!({
                "op":"save","root":"/workspace","path":path,
                "expected_revision":"a".repeat(64),"text":text
            })
        );
    }

    // SDTEST-2511 — invalid/oversize requests cannot reach an SSH executor.
    #[tokio::test]
    async fn sdtest_2511_invalid_requests_stop_before_remote_execution() {
        let executor = FakeExecutor::default();
        let oversized = "x".repeat(MAX_DOCUMENT_BYTES + 1);
        let long_path = "/".repeat(MAX_PATH_BYTES + 1);
        for (root, path, revision, text) in [
            ("", "/workspace/file", "a".repeat(64), "text"),
            ("/workspace", "file\0name", "a".repeat(64), "text"),
            ("/workspace", long_path.as_str(), "a".repeat(64), "text"),
            ("/workspace", "/workspace/file", "wrong".into(), "text"),
            ("/workspace", "/workspace/file", "a".repeat(64), "a\0b"),
            (
                "/workspace",
                "/workspace/file",
                "a".repeat(64),
                oversized.as_str(),
            ),
        ] {
            assert!(matches!(
                document_with(&executor, "save", root, path, Some(&revision), Some(text)).await,
                Err(AgentFileError::InvalidRequest(_))
            ));
        }
        assert!(executor.calls.lock().is_empty());
    }

    // SDTEST-2512 — conflict/unsupported errors retain their meaning; no retry.
    #[tokio::test]
    async fn sdtest_2512_save_conflicts_and_missing_python_are_actionable() {
        for code in ["conflict", "scope_changed"] {
            let executor = FakeExecutor::protocol(json!({"ok":false,"error":{
                "code":code,"message":"Reopen the changed remote file"
            }}));
            let error = document_with(
                &executor,
                "save",
                "/workspace",
                "file.txt",
                Some(&"a".repeat(64)),
                Some("new"),
            )
            .await
            .unwrap_err();
            assert!(error.is_conflict());
            assert_eq!(error.to_string(), "Reopen the changed remote file");
            assert_eq!(executor.calls.lock().len(), 1);
        }
        for output in [
            ExecResult {
                stdout: Vec::new(),
                stderr: b"not found".to_vec(),
                exit_code: Some(127),
            },
            ExecResult {
                stdout: serde_json::to_vec(&json!({"ok":false,"error":{
                    "code":"unsupported_host","message":"POSIX support is missing"
                }}))
                .unwrap(),
                stderr: Vec::new(),
                exit_code: Some(0),
            },
        ] {
            let executor = FakeExecutor::output(output);
            assert!(matches!(
                document_with(&executor, "read", "~", "file.txt", None, None).await,
                Err(AgentFileError::UnsupportedHost)
            ));
        }
    }

    // SDTEST-2513 — preserve exact text while rejecting invalid document data.
    #[tokio::test]
    async fn sdtest_2513_text_bytes_and_document_limits_are_preserved() {
        let text = "first\r\n\tsecond\n\u{1b}[31mλ\n";
        let executor = FakeExecutor::protocol(document(text));
        let result = document_with(&executor, "read", "~", "file.txt", None, None)
            .await
            .unwrap();
        assert_eq!(result.text.as_bytes(), text.as_bytes());
        for malformed in [
            document("a\0b"),
            document(&"x".repeat(MAX_DOCUMENT_BYTES + 1)),
            {
                let mut response = document("text");
                response["data"]["size"] = json!(3);
                response
            },
            {
                let mut response = document("text");
                response["data"]["path"] = json!("/workspace-other/file.txt");
                response
            },
            {
                let mut response = document("text");
                response["data"]["revision"] = json!("not-a-hash");
                response
            },
        ] {
            let executor = FakeExecutor::protocol(malformed);
            assert!(matches!(
                document_with(&executor, "read", "~", "file.txt", None, None).await,
                Err(AgentFileError::Protocol(_))
            ));
        }
    }

    // SDTEST-2514 — remote explorer metadata is fenced to its canonical directory.
    #[tokio::test]
    async fn sdtest_2514_listing_rejects_malformed_or_unbounded_entries() {
        let entry = json!({"name":"linked-dir","path":"/workspace/linked-dir",
            "is_dir":true,"is_symlink":true,"size":10});
        let listing = json!({"ok":true,"data":{"root":"/workspace","path":"/workspace",
            "entries":[entry.clone()],"truncated":true}});
        let executor = FakeExecutor::protocol(listing.clone());
        let result = list_with(&executor, "~", "").await.unwrap();
        assert!(result.truncated && result.entries[0].is_dir && result.entries[0].is_symlink);
        for malformed in [
            {
                let mut response = listing.clone();
                response["data"]["entries"][0]["name"] = json!("..");
                response
            },
            {
                let mut response = listing.clone();
                response["data"]["entries"][0]["path"] = json!("/elsewhere/linked-dir");
                response
            },
            {
                let mut response = listing.clone();
                response["data"]["path"] = json!("/workspace/../private");
                response
            },
            {
                let mut response = listing;
                response["data"]["entries"] = json!(vec![entry; MAX_DIRECTORY_ENTRIES + 1]);
                response
            },
        ] {
            let executor = FakeExecutor::protocol(malformed);
            assert!(matches!(
                list_with(&executor, "~", "").await,
                Err(AgentFileError::Protocol(_))
            ));
        }
    }

    // SDTEST-2515 — partial shell output and unbounded diagnostics are not documents.
    #[tokio::test]
    async fn sdtest_2515_invalid_remote_protocol_fails_closed() {
        for output in [
            ExecResult {
                stdout: b"shell welcome message".to_vec(),
                stderr: Vec::new(),
                exit_code: Some(0),
            },
            ExecResult {
                stdout: br#"{"ok":true}"#.to_vec(),
                stderr: Vec::new(),
                exit_code: Some(0),
            },
            ExecResult {
                stdout: serde_json::to_vec(&document("text")).unwrap(),
                stderr: Vec::new(),
                exit_code: None,
            },
            ExecResult {
                stdout: serde_json::to_vec(&document("text")).unwrap(),
                stderr: vec![b'x'; MAX_DIAGNOSTIC_BYTES + 1],
                exit_code: Some(0),
            },
        ] {
            let executor = FakeExecutor::output(output);
            assert!(matches!(
                document_with(&executor, "read", "~", "file.txt", None, None).await,
                Err(AgentFileError::Protocol(_))
            ));
        }
    }
}
