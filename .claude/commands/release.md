---
description: Bump ShellDeck version, tag, push, and monitor CI. Args → patch | minor | major | status | check | monitor.
argument-hint: [patch | minor | major | status | check [tag] | monitor [tag]]
---

Read the **Releasing & Auto-Update** section in `AGENTS.md` — it is the
authoritative context (single source of version truth, tag format, CI
pipeline, update manifest).

Then drive `scripts/release.sh` using the arguments below. **Do not**
reimplement bump/tag/push logic inline; the script already handles safety
checks, `cargo check`, commit message, tag push, and optional CI monitor.

## Arguments

**$ARGUMENTS**

Interpret them as:

| Args | Action |
|------|--------|
| *(empty)* | Run `./scripts/release.sh status` and ask the user which bump (`patch` / `minor` / `major`) fits the diff since the last tag. |
| `status` | `./scripts/release.sh --status` — current version, latest tag, CI state. |
| `check` | `./scripts/release.sh --check [TAG]` — verify GitHub release artifacts. |
| `monitor` | `./scripts/release.sh --monitor [TAG]` — watch `release.yml` CI. |
| `patch` | `./scripts/release.sh patch` — semver patch bump (default for fixes). |
| `minor` | `./scripts/release.sh minor` — semver minor bump (new features). |
| `major` | `./scripts/release.sh major` — semver major bump (breaking changes). |

## Agent rules

1. **Pre-flight** — working tree must be clean (except the version bump the
   script will make). If on `dev`, remind the user that releases normally
   land on `main` after merge; the script warns and asks for confirmation.
2. **Interactive prompts** — `release.sh` uses `read -rp` for branch
   confirm, commit message, push confirm, and monitor confirm. Pipe answers
   only when the user has already decided (e.g. `echo y | …` after explicit
   approval). Never push a tag without the user saying so.
3. **Version source** — only `[workspace.package] version` in the root
   `Cargo.toml`. Crate versions inherit via `workspace = true`; do not bump
   individual crate `version` keys unless one is intentionally decoupled.
4. **After push** — report the GitHub Actions URL and offer
   `--monitor` / `--check` if not already run.
5. **Do not float** `rust-toolchain.toml` as part of a release unless the
   user explicitly asks — pinned nightly is intentional.

Do not restate the full release pipeline in your response — execute the
script and report outcomes.
