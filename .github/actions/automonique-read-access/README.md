# Automonique dependency access

ShellDeck is public; its Automonique Git dependency is private. Trusted pushes,
manual releases and same-repository pull requests use the
`AUTOMONIQUE_READ_DEPLOY_KEY` Actions secret. The key is read-only and belongs to
only `bext-stack/automonique`. Fork pull requests never receive this secret, and
these workflows do not use `pull_request_target`.

The setup writes a temporary owner-only key, pins GitHub's Ed25519 host key,
ignores SSH config and rewrites only the exact Automonique HTTPS repository URL.
An always-run cleanup removes the key. Windows additionally removes inherited
ACLs. No personal token or credential URL is used. Cargo Git sources and key files
must never enter public Actions caches; only registry/Chromium payloads are cached.

The existing cargo-deny Docker action accepts the same scoped key and host pin
inside its disposable container. Its input is empty for fork pull requests.

For rotation, replace the Actions secret with a newly verified read-only deploy
key, then revoke the previous key. Never broaden this key to write permission.
