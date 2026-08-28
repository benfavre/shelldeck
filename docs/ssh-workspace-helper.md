# Trusted SSH workspace helper

ShellDeck opens an SSH workspace only through the fixed
`shelldeck-workspace-v1` subsystem. It never interpolates the catalog path into
a remote shell command and it does not treat SFTP canonicalization as retained
directory authority.

## Build and install

Build the helper from the same reviewed ShellDeck revision as the desktop
client:

```sh
cargo build --locked --release -p shelldeck-ssh --bin shelldeck-workspace-helper
```

Install the resulting binary at an administrator-owned absolute path. Then add
one fixed `Subsystem` directive to the remote host's `sshd_config`; repeat
`--root` for each catalog root this host is allowed to expose:

```text
Subsystem shelldeck-workspace-v1 /usr/local/libexec/shelldeck-workspace-helper --root /srv/shelldeck/workspaces --shell /bin/sh --git /usr/bin/git
```

Validate the sshd configuration before reloading it. The subsystem arguments
come from `sshd_config`, not from the connecting client. The helper refuses to
start without at least one absolute root and fixed existing shell/Git paths.

## Security and lifecycle

- Frames are versioned, typed, and limited to 8 KiB. There is no generic
  command field.
- Every configured root and requested child component is opened with
  `O_DIRECTORY | O_NOFOLLOW`; `.`/`..`, repeated separators, NULs, symlinks,
  detached HEADs, dirty repositories, and paths outside the configured roots
  fail closed.
- Prepare returns a random opaque receipt bound to the exact operation,
  workspace, directory identity, commit OID, and symbolic branch. The helper
  keeps the directory descriptor and waits on the same SSH channel.
- Resume rechecks the descriptor and Git state, changes directory with
  `fchdir`, restores normal interactive PTY modes, and replaces itself with the
  administrator-selected shell. Release with the same token exits without
  starting a shell. A stale token has no side effect.
- Before resume there is only one helper process, so disconnect/cancellation
  closes and reaps it through sshd. After resume the helper has been replaced;
  sshd directly owns the interactive process group.

The helper intentionally accepts only an existing clean repository root. A
future remote-worktree mutation must use a separately versioned protocol with
its own durable effect journal; it must not widen this contract in place.
