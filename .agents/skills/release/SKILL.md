---
name: release
description: Release ShellDeck by inspecting release status, selecting and applying a semantic-version bump, pushing the release tag, monitoring GitHub Actions, or checking published artifacts. Use when the user invokes $release or asks to publish, inspect, monitor, or verify a ShellDeck release.
---

# Release ShellDeck

Read the **Releasing & Auto-Update** section in `AGENTS.md` before acting. It
is authoritative for versioning, tags, CI, artifacts, and the update manifest.

Use `scripts/release.sh`; do not reproduce its bump, commit, tag, push, or CI
logic manually.

## Parse the request

Treat words supplied with `$release` as command arguments:

| Request | Command |
|---|---|
| no argument | Run `./scripts/release.sh --status`, inspect the diff since the latest tag, then ask which bump to publish. |
| `status` | `./scripts/release.sh --status` |
| `check [tag]` | `./scripts/release.sh --check [TAG]` |
| `monitor [tag]` | `./scripts/release.sh --monitor [TAG]` |
| `patch` | `./scripts/release.sh patch` |
| `minor` | `./scripts/release.sh minor` |
| `major` | `./scripts/release.sh major` |

## Guardrails

1. Require a clean working tree except for the version bump created by the
   script.
2. If the current branch is `dev`, explain that releases normally land on
   `main` after merge before continuing through the script's confirmation.
3. Treat an explicit `$release patch|minor|major` as the user's chosen bump,
   but do not bypass unrelated interactive safety prompts.
4. Read the version only from `[workspace.package]` in the root `Cargo.toml`.
5. Never float `rust-toolchain.toml` during a release unless explicitly asked.
6. After a tag push, report the GitHub Actions URL and monitor or check the
   release when requested.

Execute the workflow and report outcomes; do not restate the whole pipeline.
