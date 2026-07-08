# Testing — conventions & sync workflow

This file governs the ShellDeck test suite. It is the source of truth
for **why** we test, **what** we test, **how** we label tests, and
**what to do** when you add a feature.

Companion documents live under [`docs/testing/`](../docs/testing/):

- [`USE_CASES.md`](../docs/testing/USE_CASES.md) — the exhaustive
  `SDUC-NNN` catalogue of user-visible / contractually-observable
  behaviours the app supports.
- [`tests-core.md`](../docs/testing/tests-core.md),
  [`tests-ssh.md`](../docs/testing/tests-ssh.md),
  [`tests-terminal.md`](../docs/testing/tests-terminal.md),
  [`tests-ui-and-app.md`](../docs/testing/tests-ui-and-app.md) —
  per-crate `SDTEST-NNN` inventories, each mapped back to one or
  more SDUC IDs and marked Green / Yellow / Red / Retired.

## Philosophy — no bullshit tests

A test earns its keep only if **its failure would tell us something we
did not already know**. Concretely:

- **A regression the compiler / borrow checker already prevents is not a
  test** — do not assert that a struct has a field, that a getter
  returns what a constructor set, or that a `Result::Ok(_)` really is
  ok when the function only returns `Ok`.
- **Serde round-trips only when the wire format matters.** For a public
  contract (Manage API, config file on disk that older/newer versions
  must read), round-trip is essential. For an internal `#[derive]`
  struct that never crosses a boundary, it is noise.
- **Do not test the mock.** Reaching a canned fixture in a mock
  `TcpListener` only proves the mock was reached. The test must
  additionally assert something the *code under test* did — parsed a
  specific field, hit a specific route, produced a specific request
  body.
- **Prefer one integration-style test that pins a real workflow to five
  micro-tests that pin the same code path five times.**
- **When a test is wrong, delete it or fix it.** Never silence with
  `#[ignore]` unless there is a linked reason (network, live server)
  and a follow-up entry in the inventory.

If you catch yourself typing `assert_eq!(x.field, "hardcoded")` right
after `let x = X { field: "hardcoded".into(), .. }`, stop.

## Test taxonomy — what we do write

| Level | Where | Runs by default? | When to reach for it |
|---|---|---|---|
| **Unit** | `#[cfg(test)] mod tests` next to the code | ✅ | Pure functions, parsers, state reducers, small helpers. |
| **Contract mock** | same file, spawn `std::net::TcpListener` in a thread | ✅ | Every Manage API / Jean / Bext / Cloud client. The mock asserts *route*, *headers* (Bearer / Basic / `X-Bext-App-Id`), and *body shape*; the fixture pins the *response shape* the parser must handle. |
| **Fake-executor integration** | same file, custom `impl JobExecutor` | ✅ | Fleet runtime, script runner — anything where the real subprocess is expensive or dangerous (`claude`, `ssh`, database mutations). |
| **Live smoke** (`#[ignore]`) | same file, gated by `SHELLDECK_LIVE=1` or an env token | ❌ | Real-network sanity check the mock cannot substitute for (e.g. `merge_profiles` against the actual Manage). Never in CI. |
| **Cross-platform check** | `cargo check --target …` in CI | ✅ (CI) | Any code guarded by `#[cfg(target_os = "…")]`. See [`cross-platform.md`](cross-platform.md). |

We deliberately do **not** attempt to unit-test GPUI views. Testing a
`Render for MyView` block requires spinning up a GPUI `TestAppContext`
which today has a much higher maintenance cost than the bug rate it
would catch. Instead, factor logic *out* of `Render` into pure helpers
(reducers, filters, formatters) and unit-test those — see the sidebar
`fuzzy_match_indices` and command_palette `fuzzy_match` for the model.

## ID scheme

Two sticky namespaces, mirroring the `SDPATCH-NNN` convention from
[`patches.md`](patches.md).

### SDUC — Use Cases

- `SDUC-001`, `SDUC-002`, … numbered globally, not per-crate.
- One SDUC = one **externally observable behaviour** (a user story, a
  contract with an external service, a durable file format).
- **Sticky.** Once a use case has an ID, it keeps it, even after the
  feature is retired or reworked. Retired SDUCs move to a
  `## Retired use cases` section with the date and reason.
- Deprecated does not mean deleted — a retired SDUC still helps a future
  reader understand why a piece of code exists.

### SDTEST — Tests

- `SDTEST-001`, `SDTEST-002`, … also numbered globally.
- One SDTEST = one Rust test function *or* one tightly-coupled
  parameterized group.
- Every SDTEST **must** reference at least one SDUC. A test that maps
  to no use case is either testing an implementation detail (delete it)
  or documenting an undeclared use case (add the SDUC first, then link).
- Status column: **Green** (exists, passing), **Yellow** (exists but
  weak / flaky / needs adaptation), **Red** (to write, prioritized),
  **Retired** (removed on purpose — kept in the inventory with the
  removal date so the ID is not reused).
- Priority column on Red rows: **P0** must ship before next release,
  **P1** should ship this cycle, **P2** nice to have.

### Rust test naming

The Rust function name is free-form (readable snake_case) but **must**
match the SDTEST title exactly enough that a `git grep SDTEST-042`
lands on the function. Two acceptable styles:

```rust
// Style A — ID in a comment right above the test (preferred for
// existing tests we are back-labelling).
// SDTEST-042
#[test]
fn merge_reports_no_change_when_nothing_moves() { … }

// Style B — ID in the function name (preferred for new tests).
#[test]
fn sdtest_042_merge_reports_no_change_when_nothing_moves() { … }
```

Either style makes `grep -rn SDTEST-042 crates/` a single-hop lookup.

## Adding a feature — mandatory checklist

Every PR that adds a user-visible feature or an external contract must:

1. **Add or amend the SDUC entry.** If the feature is new, allocate the
   next `SDUC-NNN` in `USE_CASES.md`. If it extends an existing
   behaviour, amend the existing entry (and note the change in the
   `## Change log` section at the bottom of `USE_CASES.md`).
2. **Add the SDTEST entries** in the relevant `docs/testing/tests-*.md`
   before or with the code, at least at Red / P0-P1. Green tests may
   come in a follow-up commit if the code lands first, but the Red
   entries must exist so the gap is visible in review.
3. **Wire mocks the same way we already do.** For a new Manage-style
   HTTP client, copy the `TcpListener`-in-a-thread pattern already in
   `config/cloud_sync.rs`, `manage_support.rs`, `jean_fleet.rs`,
   `issues.rs`. Do **not** introduce `wiremock`, `mockito`, or a new
   HTTP mocking crate — the raw `TcpListener` pattern is deliberate:
   zero deps, no async runtime coupling, works offline.
4. **For subprocess-touching code, add a fake trait.** See
   `JobExecutor` / `ClaudeExecutor` in `config/jean_fleet.rs` for the
   template. Real subprocesses (`claude`, `ssh`, `rsync`, database
   clients) **never** run in unit tests.
5. **Cross-platform gates get cross-platform tests.** Any new
   `#[cfg(target_os = "…")]` branch needs at least a compile-check
   entry in the SDTEST inventory noting the CI target that covers it.
   `cargo test` on Linux does **not** catch a broken Windows branch.

## Running the suite

```bash
# Full workspace (Linux dev machine — set PKG_CONFIG_PATH for OpenSSL):
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig cargo test --workspace

# Single crate:
cargo test -p shelldeck-core

# Single test by name (either style, thanks to grep-friendliness):
cargo test -p shelldeck-core -- merge_reports_no_change
cargo test -p shelldeck-core -- sdtest_042

# Live smoke tests (opt-in, hits real servers — never in CI):
SHELLDECK_LIVE=1 cargo test -p shelldeck-core -- --ignored live_
```

The `/sync-patches` workflow (see [`patches.md`](patches.md), step 9)
already runs `cargo test --workspace` as its regression sweep — a fork
sync that breaks tests is a red patch and stops the workflow.

## Non-negotiables

- **Never** commit a test that panics-on-purpose without a matching
  `#[should_panic(expected = …)]`. Panics that look like real failures
  poison the signal of the whole run.
- **Never** commit a `#[ignore]` without a one-line comment explaining
  the gate (`SHELLDECK_LIVE`, requires real MySQL, etc.) and a matching
  Yellow entry in the SDTEST inventory.
- **Never** reuse a retired SDUC or SDTEST ID. The retired sections at
  the bottom of the inventories exist so the past stays legible.
- **Never** silence a failing test by widening an assertion. Either the
  test was right and the code broke, or the test was wrong and it
  should be deleted, not weakened.
- **Never** introduce a live-network test that runs by default. If a
  reviewer can't `cargo test` on a train without a hostile-flag, the
  contract is broken.
