# ShellDeck portable-pty patches

This is `portable-pty` 0.8.1 with one Windows security fix:

- Preserve an explicitly requested working directory through to
  `CreateProcessW`, even when it becomes unavailable. Windows then rejects
  child creation instead of silently falling back to `USERPROFILE`.
- Declare the crate's existing legacy `cargo-clippy` cfg as an empty feature so
  current Rust check-cfg does not warn while compiling the vendored source.

The upstream 0.8.1 behavior treated an explicit missing directory as absent,
which could open a shell in the user's home after a workspace authorization
check raced with checkout removal.
