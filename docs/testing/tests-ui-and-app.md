# SDTEST inventory — `shelldeck-ui`, `shelldeck`, `shelldeck-update`

> Rules for this file live in [`.agents/testing.md`](../../.agents/testing.md).
> Use case IDs (`SDUC-…`) resolve in [`USE_CASES.md`](./USE_CASES.md).

**Big picture.** These three crates have **0 tests** today. That is
partly intentional (GPUI views are hard to unit-test, see
`.agents/testing.md`) and partly a real gap.

The recipe is: **push logic out of `Render` blocks into pure helpers,
then unit-test the helpers**. The two working models already in the
codebase are `command_palette::fuzzy_match` (pure fn — trivial to
test) and `sidebar::fuzzy_match_indices` (pure fn — trivial to test).
Anything that is stateful but *not* GPUI-touching (reducers, filters,
key-decoders, formatters) belongs in the same bucket.

`shelldeck-update` is different — it is mostly async I/O against
Cloudflare + a small platform-key helper. Every field of that surface
matters and is testable without GPUI.

---

## 1. `shelldeck-ui/command_palette.rs`

Existing: **0 tests.**

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1000 | *to write* — `fuzzy_match(haystack, needle)` — needle empty matches everything | SDUC-300 | **Red / P0** | Pure fn; foundational. |
| SDTEST-1001 | *to write* — fuzzy_match preserves in-order requirement | SDUC-300 | **Red / P0** | `"ab" matches "arb"` but not `"ba"`. |
| SDTEST-1002 | *to write* — fuzzy_match is case-insensitive | SDUC-300 | **Red / P0** | |
| SDTEST-1003 | *to write* — fuzzy_match handles utf-8 correctly (accented chars) | SDUC-300 | **Red / P1** | French UI strings. |
| SDTEST-1004 | *to write* — CommandPalette::set_actions replaces the action list wholesale | SDUC-303 | **Red / P1** | No accidental append. |
| SDTEST-1005 | *to write* — update_filter is deterministic for identical input | SDUC-303 | **Red / P1** | Idempotent guarantee. |
| SDTEST-1006 | *to write* — select_next / select_prev wrap at bounds | SDUC-305 | **Red / P1** | |
| SDTEST-1007 | *to write* — selected_action returns None on empty filter | SDUC-305 | **Red / P2** | |
| SDTEST-1008 | *to write* — reset_input clears the query and selection index | SDUC-305 | **Red / P2** | |

---

## 2. `shelldeck-ui/sidebar.rs`

Existing: **0 tests.**

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1020 | *to write* — `fuzzy_match_indices` returns Some(vec![]) for empty needle | SDUC-301 | **Red / P0** | Contract distinct from palette (needs highlight positions). |
| SDTEST-1021 | *to write* — fuzzy_match_indices returns byte-position matches, not char-position | SDUC-301 | **Red / P0** | Consumed by highlight rendering. |
| SDTEST-1022 | *to write* — fuzzy_match_indices returns None on no-match | SDUC-301 | **Red / P0** | |
| SDTEST-1023 | *to write* — conn_matches_site: none-filter shows all | SDUC-302 | **Red / P0** | |
| SDTEST-1024 | *to write* — conn_matches_site: filtered site shows matches + unbound | SDUC-302 | **Red / P0** | Behavioural contract per AGENTS.md. |
| SDTEST-1025 | *to write* — conn_matches_search: alias, hostname, user, tag match | SDUC-306 | **Red / P1** | |
| SDTEST-1026 | *to write* — set_width clamps within [MIN, MAX] | SDUC-307 | **Red / P1** | |
| SDTEST-1027 | *to write* — toggle_collapsed toggles state and preserves other state | SDUC-308 | **Red / P2** | |

---

## 3. `shelldeck-ui/workspace/mod.rs` (pure helpers only)

**Do not** attempt to unit-test the `Render` impl. Instead: extract
these helpers as free `pub(crate) fn`s (they mostly already are) and
test them.

Existing: **0 tests.**

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1050 | *to write* — effective_mode(): logged-out → Dev | SDUC-309, SDUC-152 | **Red / P0** | Pure fn over `Option<AccountInfo>` + `AppMode`. |
| SDTEST-1051 | *to write* — effective_mode(): superadmin returns persisted mode | SDUC-309 | **Red / P0** | |
| SDTEST-1052 | *to write* — effective_mode(): non-superadmin forced to User | SDUC-309, SDUC-152 | **Red / P0** | Security-adjacent (SDUC-152). |
| SDTEST-1053 | *to write* — can_switch_mode(): true only for signed-in superadmin | SDUC-309 | **Red / P0** | |
| SDTEST-1054 | *to write* — has_jean(): true when local `[jeanclaude]` set OR server-delivered config exists | SDUC-185 | **Red / P0** | Precedence contract per AGENTS.md. |
| SDTEST-1055 | *to write* — effective_jean_config prefers local over server | SDUC-185 | **Red / P0** | |
| SDTEST-1056 | *to write* — refresh_command_palette produces stable action list for stable input | SDUC-303 | **Red / P1** | Reducer-style test on the action-builder. |
| SDTEST-1057 | *to write* — refresh_command_palette adds "Mode : User/Support/Dev" entries only for superadmin | SDUC-152, SDUC-303 | **Red / P1** | |
| SDTEST-1058 | *to write* — action-list contains SwitchSite entries capped at 20 | SDUC-303 | **Red / P2** | |
| SDTEST-1059 | *to write* — poll schedulers no-op when the relevant surface is not visible | SDUC-168, SDUC-188, SDUC-227, SDUC-249 | **Red / P0** | Regression class: burning bandwidth / cache lines. Test as a pure predicate `should_poll(active_view, feature)`. |

---

## 4. `shelldeck-ui/editor_buffer.rs`, `file_editor/*`, `syntax/*`

Existing: **0 tests.**

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1080 | *to write* — EditorBuffer: insert / delete / move-cursor round-trip preserves content | *(new SDUC)* | **Red / P1** | Log a new SDUC when this crate ships; the surface is still moving. |
| SDTEST-1081 | *to write* — syntax highlighter: bash tokenises `$VAR`, `${VAR}`, `"$(cmd)"` correctly | *(new SDUC)* | **Red / P2** | Table-driven per language. |
| SDTEST-1082 | *to write* — highlighter never yields overlapping ranges | *(new SDUC)* | **Red / P1** | Contract — the renderer assumes non-overlap. |

*(These require SDUC entries — deferred until the file editor surface
stabilises. Marker so we don't forget.)*

---

## 5. `shelldeck-ui/{login_form,connection_form,port_forward_form,script_form}.rs`

Existing: **0 tests.**

Extract validation into pure helpers first, then test:

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1100 | *to write* — LoginForm::submit_disabled_when_empty | SDUC-315 | **Red / P1** | |
| SDTEST-1101 | *to write* — LoginForm OIDC button passes correct provider | SDUC-149, SDUC-150 | **Red / P1** | |
| SDTEST-1102 | *to write* — ConnectionForm: alias uniqueness against store | SDUC-313 | **Red / P1** | |
| SDTEST-1103 | *to write* — ConnectionForm: hostname required, port defaults to 22 | SDUC-313 | **Red / P1** | |
| SDTEST-1104 | *to write* — PortForwardForm: picker filters by connectable hosts | SDUC-314 | **Red / P1** | |
| SDTEST-1105 | *to write* — ScriptForm: variable list mirrors extract_variables() on body edit | SDUC-060 | **Red / P1** | Cross-referenced with SDTEST-034. |

---

## 6. `shelldeck/main.rs` + `actions.rs`

Existing: **0 tests.**

`main.rs` is entry glue — mostly untestable. `actions.rs` is a
`gpui::actions!` block — also untestable directly, but the
*handlers* it wires can be tested via the workspace helpers above.

The one real test worth having is the startup-sequence smoke:

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1150 | *to write* — startup cloud sync is bounded by the documented timeouts (4s / 10s) | SDUC-100 | **Red / P0** | Regression sensor: a runaway startup sync freezes the launch. |
| SDTEST-1151 | *to write* — startup account check does not touch `[cloud_sync]` when it 401s | SDUC-154 | **Red / P1** | |
| SDTEST-1152 | *to write* — shutdown() closes tunnels + sessions cleanly | SDUC-048, SDUC-052 | **Red / P1** | Regression: leaked ports. |

---

## 7. `shelldeck-update` — auto-update client

Existing: **0 tests.**

This crate is a strong candidate for a proper unit-test pass — its
surface is small, contract-heavy, and 100% testable without GPUI.

### `platform.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1200 | *to write* — current_platform() returns `linux-{arch}` on Linux | SDUC-280 | **Red / P0** | Linux CI. |
| SDTEST-1201 | *to write* — current_platform() returns `macos-{arch}` on macOS (never `darwin-*`) | SDUC-280 | **Red / P0** | macOS CI. Contract-critical. |
| SDTEST-1202 | *to write* — current_platform() returns `windows-{arch}` on Windows | SDUC-280 | **Red / P0** | Windows CI. |
| SDTEST-1203 | *to write* — arch covers `x86_64` and `aarch64` | SDUC-280 | **Red / P0** | Both macOS silicons. |

### `lib.rs` — `AutoUpdater`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1220 | *to write* — poll cadence: first check immediate, then hourly | SDUC-281 | **Red / P0** | Use a mockable clock or an `Instant`-injecting trait. |
| SDTEST-1221 | *to write* — set_enabled(false) cancels the poll task and no-ops check_for_update | SDUC-285 | **Red / P0** | User can turn it off. |
| SDTEST-1222 | *to write* — ReleaseInfo parses the Worker JSON contract example | SDUC-282 | **Red / P0** | Mock TcpListener with the canonical response. |
| SDTEST-1223 | *to write* — ReleaseInfo Errs on a missing per-platform URL | SDUC-282 | **Red / P1** | |
| SDTEST-1224 | *to write* — AutoUpdateEvent stream fires the expected transitions | SDUC-281 | **Red / P1** | State machine — Idle → Checking → Available/UpToDate → Downloading → Ready → Installed. |

### `installer.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1240 | *to write* — download_and_verify Errs on SHA-256 mismatch (does not install) | SDUC-283 | **Red / P0** | Security-critical. Feed a fixture archive + wrong hash. |
| SDTEST-1241 | *to write* — download_and_verify streams bytes (does not buffer the whole archive) | SDUC-283 | **Red / P1** | Regression sensor for memory on macOS DMG (~200 MB). |
| SDTEST-1242 | *to write* — install replaces binary atomically on Unix | SDUC-284 | **Red / P0** | Unix CI. |
| SDTEST-1243 | *to write* — install uses pending-replace pattern on Windows | SDUC-284 | **Red / P0** | Windows CI. |
| SDTEST-1244 | *to write* — install fails cleanly if archive is corrupt (no partial writes) | SDUC-284 | **Red / P1** | |

### Cross-repo smoke

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1260 | *to write* — install.sh + install.ps1 grep for the same platform keys the worker emits | SDUC-286, SDUC-287 | **Red / P0** | Bash regex check in CI. A drift here is what breaks releases silently — this is the highest-value cheap test in the whole doc. |
| SDTEST-1261 | *to write* — release.yml asset names match the worker's manifest keys | SDUC-287 | **Red / P0** | YAML+JS parser check in CI. |

---

## 8. Cross-platform coverage (referenced from everywhere)

CI matrix already runs `cargo check` on all three targets. The SDTEST
entries that carry cross-platform stakes and must run on multiple
targets (not just Linux) are cross-linked here for the release
checklist:

- SDTEST-121, SDTEST-122 (keychain macOS/Windows)
- SDTEST-960..968 (PTY spawn on all three)
- SDTEST-1201, SDTEST-1202 (platform key mapping)
- SDTEST-1242, SDTEST-1243 (installer replace on Unix / Windows)
- SDTEST-1260, SDTEST-1261 (install-script + manifest parity)

The release-day rule: **all P0 cross-platform tests must be green on
the matching CI runner before the tag goes out.** This maps directly
to AGENTS.md's `cross-platform.md` mandate that "if any of the three
builds fails, the release + manifest jobs are skipped entirely".

---

## Retired tests

*(none yet)*
