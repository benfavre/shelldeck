# Agent file explorer and middle-pane editor — 2026-10-15

Files now open in a retained native `FileEditorView` in the middle pane. The
Conversation/Editor switch preserves tabs, cursor/scroll, undo and unsaved text
within each session and execution context. The Files inspector remains beside
it on wide panes; its narrow drawer closes when a file opens. The global editor
and agent editor use the same persisted editor preferences.

The explorer shares local/SSH row styling, keyboard activation, full-path
hints, current-file highlighting, refresh and hidden-file controls. SSH directory
breadcrumbs and Up stay inside the canonical session root. Large listings have
bounded traversal, rendered pages and explicit truncation notices. Opening a
local file reveals its parent directories. The directory heading copies its
full path.

Remote text documents use a distinct external identity and explicit Save
requests; remote paths can never become native local write paths. The remote
helper receives paths/content as bounded JSON stdin and requires Python 3 with
POSIX descriptor-relative filesystem support. Reads preserve UTF-8 bytes and
line endings, reject binary/symlink files and enforce an 8 MiB text limit.
Saves check SHA-256 revisions, reject changed/symlink/hardlinked files and replace
atomically while preserving permissions. Save acknowledgments only clean the
submitted text snapshot; newer edits and failed saves remain dirty. The agent
access badge explicitly identifies execution permissions, independently of
human editor Save actions.

Validation:

- Linux app and provider-free native example built successfully with the pinned
  nightly and `--locked`; `cargo check -p shelldeck-ui` passed.
- Isolated-XDG UI suite: **224 passed, 3 failed**. The failures are the same
  pre-existing native worktree lifecycle tests SDTEST-1763, 1764 and 1770, with
  the previously recorded owned-worktree-location authorization error.
- All ten new UI regressions passed (2495–2498, 2500–2503, 2520, 2522); migrated
  production local snapshot scope regression 1909 passed. Shell-probe tests
  1908 and 2300 were retired with their replaced implementations.
- SSH new contract/transport suite: **9 passed**, SDTEST-2510–2518. The existing
  modified in-memory transport harness also passed all eight selected tests.
- Disposable local helper fixtures verified exact CRLF/control UTF-8 bytes,
  permission preservation, content conflicts, root/symlink/hardlink fences,
  binary/encoding/size refusal, shell-like filenames and temporary-file cleanup.
- Native 1280px fixture: typed into a file, returned to Conversation and Editor,
  saved through the production local snapshot helper, opened a second file,
  returned to the first tab and checked its exact saved bytes. The isolated
  fixture supplies its own local-only Save adapter because no Workspace or
  account/runtime exists there. No remote host or provider was contacted.
- Native 700px fixture: editor, toolbar, tab, gutter and status bar remained
  visible after the Files drawer closed. Screenshots were inspected:
  [wide](agent-editor-preview-wide.png), [narrow](agent-editor-preview-narrow.png).
- Formatting and whitespace checks passed. Workspace orchestrator remains below
  its 2,000-line limit. Cargo.lock only adds the existing serde dependencies to
  `shelldeck-ssh`; pinned nightly and pathfinder_simd 0.5.5 are unchanged.

Limits: real SSH-host editor interaction and macOS/Windows native rendering
were not exercised here. Remote image/PDF/binary editing is not supported;
existing local previews remain available. Unsaved editor buffers are retained
in memory across panes/sessions, not persisted across app exit. Conflict recovery
preserves the draft and tells the user to copy changes, close/discard the stale
tab and reopen the file. Revision checks are optimistic: ordinary local path
checks and remote cooperative locking cannot provide a filesystem-wide atomic
compare-and-replace guarantee against an unrelated writer racing the final
rename. Conflicts never trigger automatic overwrites or retries.
