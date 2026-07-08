# `.agents/` — modular agent rules

This directory holds topic-scoped instruction files for AI coding agents
(Claude Code, Cursor, Codex, …). It is the modular counterpart to the
repo-root [`AGENTS.md`](../AGENTS.md).

## How it works

- [`AGENTS.md`](../AGENTS.md) is the entry point read by every AGENTS.md-aware
  tool.
- `CLAUDE.md` at the repo root is a one-line pointer (`@AGENTS.md`) so Claude
  Code loads the same content.
- At the bottom of `AGENTS.md` there is a "Modular rules" section listing
  `@.agents/<file>.md` imports. Claude Code follows those imports recursively.
- **Cursor** does not expand `@` imports — it loads `AGENTS.md` via `CLAUDE.md`
  but not the `.agents/` modules automatically. The bridge lives in
  [`.cursor/rules/`](../.cursor/rules/) (one `.mdc` per module, with `globs`
  or `alwaysApply`). Keep `.agents/` and `.cursor/rules/` in sync when you
  edit a rule file.

## Adding a rule file

1. Create `xxx.md` here — one focused topic per file (a subsystem, a
   workflow, a class of bugs, a coding convention).
2. Register it by adding a line `@.agents/xxx.md` in the "Modular rules"
   section of [`AGENTS.md`](../AGENTS.md).
3. Keep it tight. If a file grows past ~200 lines, split by sub-topic.

## Conventions

- **Filename** = kebab-case topic, e.g. `gpui-patterns.md`, `ssh-safety.md`,
  `release-checklist.md`.
- **First line** = an H1 with the topic name.
- **Lead with the rule.** Explain the *why* only when the reason isn't
  obvious from the code — a past incident, a subtle invariant, a constraint
  from an external system.
- **Don't restate what's already in `AGENTS.md`.** If a rule is more general
  than one topic, put it in `AGENTS.md` directly.
