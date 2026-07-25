# SDTEST inventory — `shelldeck-ui`, `shelldeck`, `shelldeck-update`

> Rules for this file live in [`.agents/testing.md`](../../.agents/testing.md).
> Use case IDs (`SDUC-…`) resolve in [`USE_CASES.md`](./USE_CASES.md).

**Big picture.** These three crates have **12 tests** today
(`shelldeck-ui/src/{i18n,command_palette,sidebar}.rs`) and huge gaps
elsewhere. The low count is partly intentional (GPUI views are hard
to unit-test, see `.agents/testing.md`) and partly a real gap.

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
| SDTEST-1000 | `command_palette.rs::empty_needle_matches_everything` | SDUC-300 | Green | Added 2026-07-09. |
| SDTEST-1001 | `command_palette.rs::subsequence_must_appear_in_order` | SDUC-300 | Green | Added 2026-07-09. |
| SDTEST-1002 | `command_palette.rs::haystack_case_folded_but_needle_taken_as_is` | SDUC-300 | Green | Added 2026-07-09. **Contract correction** — the fn only lowercases the haystack; the caller must pre-lowercase the needle. Not "double-sided case-insensitive" as my original inventory claimed. |
| SDTEST-1003 | `command_palette.rs::utf8_accented_chars_match` | SDUC-300 | Green | Added 2026-07-09. Comparison is by unicode `char`; `é` and `e` are distinct. |
| SDTEST-1343 | `command_palette.rs::recent_actions_are_deduplicated_capped_and_followed_by_the_full_list` | SDUC-417 | Green | Recent commands are ordered newest-first, missing actions are dropped, the cap is enforced, and the remaining full list contains no duplicates. |
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
| SDTEST-1020 | `sidebar.rs::empty_needle_returns_empty_indices` | SDUC-301 | Green | Added 2026-07-09. |
| SDTEST-1021 | `sidebar.rs::returns_char_positions_not_bytes` | SDUC-301 | Green | Added 2026-07-09. **Contract correction** — returned indices are CHAR positions in the lowercased haystack, not byte offsets (consumer walks a `Vec<char>` at the same index). My original inventory was wrong. |
| SDTEST-1022 | `sidebar.rs::no_match_returns_none` | SDUC-301 | Green | Added 2026-07-09. Also covers double-sided case-insensitivity (unlike `fuzzy_match`, this fn lowercases the needle too). |
| SDTEST-1023 | `sidebar.rs::no_filter_matches_every_connection` | SDUC-302 | Green | Added 2026-07-09. |
| SDTEST-1024 | `sidebar.rs::filter_matches_bound_site_and_all_unbound_connections` | SDUC-302 | Green | Added 2026-07-09. Test hits the extracted pure fn `conn_matches_site_filter(Option<Uuid>, Option<Uuid>) -> bool` so no GPUI `Context` needed. The method still exists and delegates. |
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
| SDTEST-1050 | *(covered by SDTEST-184)* — effective_mode(): logged-out → defensive User behind welcome | SDUC-309, SDUC-152 | Green | Pure resolver is safe even if a future render path misses the welcome interception. |
| SDTEST-1051 | *(covered by SDTEST-184)* — effective_mode(): superadmin returns persisted mode | SDUC-309 | Green | Same file/test as SDTEST-184. |
| SDTEST-1052 | `cloud_account.rs::resolve_effective_mode_regular_user_and_client_admin_forced_to_user` (+ full-truth-table sibling) | SDUC-309, SDUC-152 | Green | **P0 security invariant** — regular/customer-admin accounts cannot land on Support or Dev; `inklura_support` is clamped to User/Support; super-admin gets all three. |
| SDTEST-1053 | `cloud_account.rs::can_switch_true_for_signed_in_inklura_support_or_superadmin` | SDUC-309 | Green | Pure predicate for the dedicated Support and super-admin tiers. |
| SDTEST-1054 | `jeanclaude.rs::resolve_effective_{local_wins_over_server, falls_back_to_server_when_local_unset, falls_back_to_server_when_local_none, none_when_neither_set}` | SDUC-185 | Green | 4 tests, 2026-07-09. Precedence contract from AGENTS.md § JeanClaude pinned as a pure fn on `JeanConfig`. Cross-linked to tests-core.md § SDTEST-1054 (jean). |
| SDTEST-1055 | *(covered by SDTEST-1054)* — effective_jean_config prefers local over server | SDUC-185 | Green | Same fn as SDTEST-1054 (`resolve_effective_local_wins_over_server`). |
| SDTEST-1056 | *to write* — refresh_command_palette produces stable action list for stable input | SDUC-303 | **Red / P1** | Reducer-style test on the action-builder. |
| SDTEST-1057 | `cloud_account.rs::allowed_modes_matches_the_tier_table` | SDUC-152, SDUC-303 | Green | Pins User-only, User+Support, and full User+Support+Dev mode lists. Workspace switcher and palette consume this exact list. |
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
| SDTEST-1200 | `platform.rs::linux_uses_linux_prefix` (`#[cfg(target_os = "linux")]`) | SDUC-280 | Green | Added 2026-07-09 (cluster J). Runs on Linux CI. |
| SDTEST-1201 | `platform.rs::macos_uses_macos_prefix_never_darwin` (`#[cfg(target_os = "macos")]`) | SDUC-280 | Green | Added 2026-07-09 (cluster J). **Contract-critical** — asserts `macos-*` AND explicitly forbids `darwin-*`. macOS CI runner needed to exercise the assertion. |
| SDTEST-1202 | `platform.rs::windows_uses_windows_prefix` (`#[cfg(target_os = "windows")]`) | SDUC-280 | Green | Added 2026-07-09 (cluster J). Windows CI. |
| SDTEST-1203 | `platform.rs::arch_is_a_known_value` + `platform_string_shape_is_os_dash_arch` | SDUC-280 | Green | 2 tests, added 2026-07-09 (cluster J). Runs on every target; warns (not errors) if a new arch slips in as `unknown`. |

### `lib.rs` — `AutoUpdater`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1220 | *to write* — poll cadence: first check immediate, then hourly | SDUC-281 | **Red / P0** | Use a mockable clock or an `Instant`-injecting trait. |
| SDTEST-1221 | *to write* — set_enabled(false) cancels the poll task and no-ops check_for_update | SDUC-285 | **Red / P0** | User can turn it off. |
| SDTEST-1222 | `shelldeck-update::tests::release_info_parses_signed_worker_contract` | SDUC-282 | Green | Pins the platform, digest, size, publication date and Ed25519 signature returned by the Worker. |
| SDTEST-1223 | *to write* — ReleaseInfo Errs on a missing per-platform URL | SDUC-282 | **Red / P1** | |
| SDTEST-1224 | *to write* — AutoUpdateEvent stream fires the expected transitions | SDUC-281 | **Red / P1** | State machine — Idle → Checking → Available/UpToDate → Downloading → Ready → Installed. |

### `installer.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1240 | `installer::tests::sha256_mismatch_removes_partial_download` | SDUC-283 | Green | Streams a fixture through a local HTTP socket, rejects the digest and leaves neither destination nor partial archive. |
| SDTEST-1241 | *to write* — download_and_verify streams bytes (does not buffer the whole archive) | SDUC-283 | **Red / P1** | Regression sensor for memory on macOS DMG (~200 MB). |
| SDTEST-1242 | `installer::tests::linux_binary_replacement_is_atomic_and_keeps_backup` | SDUC-284 | Green | Verifies the same-filesystem rename and retained rollback copy on Linux. |
| SDTEST-1243 | *to write* — install uses pending-replace pattern on Windows | SDUC-284 | **Red / P0** | Windows CI. |
| SDTEST-1244 | *to write* — install fails cleanly if archive is corrupt (no partial writes) | SDUC-284 | **Red / P1** | |

### Cross-repo smoke

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1260 | `lib.rs::release_parity_tests::every_shipping_key_appears_in_release_workflow` + `every_shipping_key_appears_in_update_worker` + `current_platform_matches_a_release_key_or_is_explicitly_unsupported` | SDUC-286, SDUC-287 | Green | 3 tests, added 2026-07-09 (cluster J). `include_str!` reads `.github/workflows/release.yml` + `cloudflare/update-worker/src/index.ts` at compile time; asserts each shipping key (`linux-x86_64`, `macos-aarch64`, `windows-x86_64`) is a literal string in BOTH sources + round-trips to `current_platform()`. |
| SDTEST-1261 | `lib.rs::release_parity_tests::darwin_prefix_is_forbidden_in_release_contract` | SDUC-287 | Green | Added 2026-07-09 (cluster J). Explicit forbid on `darwin-x86_64`, `darwin-aarch64`, `darwin-arm64` in workflow AND worker source. AGENTS.md contract. |

---

## 8. `shelldeck-ui/i18n.rs` — rust-i18n helpers

Existing: **2 tests.** First non-view module in `shelldeck-ui` to
carry unit tests — the pattern to copy for any future pure-logic
helper extracted out of a `Render` block.

⚠️ **Global-state footgun.** `rust_i18n::set_locale` writes a
process-wide value. Any test that calls `apply_ui_language` races
with any other. Keep locale-mutating tests **sequential inside a
single `#[test]` fn** (see `locale_fr_and_en` for the canonical
form). Do **not** add per-locale tests — they will flake under
parallel `cargo test`.

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1300 | `i18n.rs::locale_fr_and_en` | SDUC-401, SDUC-403, SDUC-434, SDUC-439 | Green | Fused fr+en scenario — deliberate (locale is process-global); also pins translated tray action, plural-counter labels, and connection-identified SSH notification copy. |
| SDTEST-1301 | `i18n.rs::resolve_locale_system_is_fr_or_en` | SDUC-401 | Green | Smoke test that `System` resolves to a known locale on the CI runner regardless of OS. |
| SDTEST-1302 | `i18n.rs::fr_en_locale_key_parity` | SDUC-403 | Green | Added 2026-07-09. Loads both TOMLs via `include_str!`, diffs the `toml::Table` key sets. Adds a `toml` dev-dependency to `shelldeck-ui` (workspace version). |
| SDTEST-1303 | ~~missing key falls back to the French value~~ | ~~SDUC-403~~ | **Retired** | Subsumed by SDTEST-1302 (strict parity means the fallback path is never exercised in practice) and SDTEST-1300 (which proves the locale actually switches by asserting `"Se connecter"` ≠ `"Sign in"` — if fallback were silently masking, en would return the fr value). Any manufactured "canary key" would itself break parity. Kept in the inventory to preserve the sticky ID. |
| SDTEST-1304 | *to write* — `rel_time(at_ms)` produces localized strings per locale | SDUC-404 | **Red / P1** | Same sequential pattern; assert "à l'instant" (fr) vs "just now" (en) at t=now. |
| SDTEST-1305 | *to write* — `t!("login.device", device = "…")` interpolates `%{device}` | SDUC-405 | **Red / P1** | |
| SDTEST-1306 | *to write* — `t!()` with no variables ignores extras without erroring | SDUC-405 | **Red / P2** | Defensive; matches rust-i18n behaviour. |
| SDTEST-1307 | *to write* — `UiLanguage` round-trips through `shelldeck.toml` as snake_case | SDUC-400 | **Red / P1** | Lives in `shelldeck-core::config::app_config` — add there, not here. Cross-linked. |
| SDTEST-1308 | *to write* — Config without `ui_language` still parses (defaults to `System`) | SDUC-400 | **Red / P1** | Same location; back-compat with pre-i18n configs. |
| SDTEST-1309 | *to write* — Unknown OS locale resolves to `"fr"`, not `"en"` | SDUC-401 | **Red / P1** | Product default per AGENTS.md; regression sensor if someone flips the fallback. Needs an injectable locale-reader trait to test deterministically. |

---

## 8a. `shelldeck-ui/terminal_view.rs` — CLI discovery helpers

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1333 | `terminal_view.rs::command_discovery_searches_every_path_entry` | SDUC-410 | Green | Uses isolated temporary PATH entries; never depends on the developer machine's installed CLIs. |
| SDTEST-1334 | `terminal_view.rs::command_discovery_honors_executable_extensions` | SDUC-410 | Green | Pins PATHEXT-style suffix lookup used by Windows npm-installed CLIs. |

## 8b. `shelldeck/src/tray/mod.rs` — pinned menu routing

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1336 | `tray::tests::pinned_menu_id_routes_to_connection` | SDUC-412 | Green | A tray id containing a valid UUID routes to that exact pinned connection. |
| SDTEST-1337 | `tray::tests::unknown_or_malformed_menu_id_is_ignored` | SDUC-412 | Green | Counter rows, unknown actions and malformed UUIDs cannot trigger a connection. |

---

## 8c. `shelldeck-ui/ai_assistant.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1341 | `ai_assistant.rs::stale_ai_response_is_rejected_after_context_invalidation` | SDUC-414 | Green | Pure request-generation gate extracted from the GPUI view; a response from a closed/previous context cannot overwrite the current draft. |
| SDTEST-1345 | *to write* — integrated AI affordances follow backend and per-surface availability | SDUC-413, SDUC-418 | **Red / P1** | GPUI wiring: Support/Script buttons stay hidden when disabled and emit the exact selected target when enabled. |
| SDTEST-1346 | *to write* — accepting an integrated draft prepares but never finalizes the action | SDUC-414, SDUC-418 | **Red / P0** | GPUI workflow: Support fills the reply composer without sending; Scripts fills the inline buffer without saving or executing. |
| SDTEST-1349 | *to write* — Script form AI generation populates only unsaved form fields | SDUC-414, SDUC-418 | **Red / P1** | GPUI wiring: explicit prompt → loading state → validated name/description/language/category/body insertion; target/host unchanged, no save and no execution. |
| SDTEST-1351 | *to write* — failed latest Script execution exposes correction without auto-run | SDUC-414, SDUC-418 | **Red / P0** | GPUI wiring: button hidden for running/success/no-history, visible for the selected script's latest non-zero exit, accepting opens unsaved inline editing only. |
| SDTEST-1354 | *to write* — request AI actions use the selected issue and never submit | SDUC-414, SDUC-420 | **Red / P0** | GPUI wiring: reply/summary/triage target the selected request; accepting a reply fills the composer without posting a comment. |
| SDTEST-1355 | *to write* — large User and Support lists preserve row routing while virtualized | SDUC-421 | **Red / P1** | GPUI integration: with 250 mixed records, scroll/filter/select/context-menu/delete still target the exact visible record while only the viewport range is rendered. |
| SDTEST-1357 | *to write* — New Request AI generation only fills the current unsent sheet | SDUC-414, SDUC-422 | **Red / P0** | GPUI wiring: AI panel starts collapsed and resets on close; explicit instructions show loading, valid structured output fills title/body/priority without submitting, and a response arriving after close cannot overwrite a later draft. |
| SDTEST-1359 | *to write* — structured request triage requires explicit staff confirmation | SDUC-414, SDUC-423 | **Red / P0** | GPUI wiring: staff sees before/after priority and assignee plus rationale/actions; apply revalidates target and agent, non-staff cannot emit mutations, and no-change/invalid proposals keep Apply disabled. |
| SDTEST-1360 | *to write* — Support conversion opens a source-aware unsent draft | SDUC-424 | **Red / P0** | GPUI wiring: Convert pre-fills title/body and `source=support`, does not call create, and close followed by New Request resets the source to `user`. |
| SDTEST-1361 | *to write* — Terminal diagnostic context opens a bounded request draft | SDUC-425 | **Red / P0** | GPUI wiring: selection wins over visible output, session identity is revalidated, source is `shelldeck`, and opening/AI adjustment never executes or creates anything. |
| SDTEST-1362 | `ai.rs::generated_name_json_is_short_single_line_text` | SDUC-426 | Green | Strict JSON naming accepts a short one-line name and rejects multiline or over-80-character output. |
| SDTEST-1363 | *to write* — naming actions apply only to their still-open entity | SDUC-426 | **Red / P0** | GPUI wiring: Script/Tunnel/Request fields and Terminal title change only after Accept; disabled Naming hides actions, stale targets and Cancel leave state untouched, and no persistence or execution is triggered. |
| SDTEST-1364 | `ai.rs::action_plan_rejects_mismatched_payload_and_redacts_content_from_audit` | SDUC-427, SDUC-428 | Green | Rejects kind/payload mismatches and proves audit metadata excludes the executable payload. |
| SDTEST-1365 | *to write* — executable AI drafts require a second target-safe confirmation | SDUC-427 | **Red / P0** | GPUI wiring: Accept still only inserts; Execute/Send opens the shared plan dialog, Cancel is inert, and final confirmation rejects a changed session/ticket/issue/instance. |
| SDTEST-1366 | *to write* — AI script tracking cannot stop a later execution | SDUC-428 | **Red / P0** | Fake-clock/process wiring: success/failure/cancel remove the matching action ID; only the still-current action times out and invokes the existing Stop path. |
| SDTEST-1368 | *to write* — AI task center routes exact targets and only exposes valid actions | SDUC-429 | **Red / P0** | GPUI wiring: actionable count matches the titlebar badge; resume/open/stop/delete route by task ID, active tasks survive sheet closure, and stale active states recover as cancelled after restart. |
| SDTEST-1370 | *to write* — AI policy controls drive the executable workflow action | SDUC-430 | **Red / P0** | GPUI wiring: Settings persists each capability independently; Prepare hides/blocks Execute, Confirm opens the second dialog, Automatic executes moderate actions directly, and High risk still opens confirmation. |
| SDTEST-1372 | *to write* — Terminal diagnostic steps remain explicit and target-safe | SDUC-431 | **Red / P0** | GPUI wiring: structured steps render without raw JSON, each step revalidates the active session and opens high-risk confirmation, full-plan execution advances only after matching OSC 133 completion, stops on failure, and Ctrl+C remains available. |
| SDTEST-1374 | `issue_attachments.rs::rejects_extension_spoofing` + `recognizes_png_magic` | SDUC-432 | Green | Pure local intake guard: accepted formats are identified by bytes, never filename alone. |
| SDTEST-1375 | *to write* — attachment picker routes URL/paste/drop/file/capture drafts to the exact composer | SDUC-432 | **Red / P0** | GPUI integration: each source adds one removable preview to the active New Request, request comment, Support request comment, ticket reply, or internal note; changing target clears drafts; submission uploads once and preserves drafts on failure. |
| SDTEST-1376 | *to write* — shared multi-line Input follows native wrapped-line editing semantics | SDUC-433 | **Red / P0** | GPUI integration: Up/Down retain visual X, Shift selection paints across hard/soft lines, Home/End stay on the visual row, mouse placement matches the glyph, wheel input scrolls a capped field, and `max_rows` keeps the caret visible. |
| SDTEST-1377 | *to write* — role boundaries cover palette, shortcuts, deep links, and Settings | SDUC-152, SDUC-310 | **Red / P0** | GPUI integration: regular users cannot spawn Dev work; Support never sees Dev; super-admin can enter all modes; Settings opens/closes from every authenticated mode and hides Terminal/Editor for non-Dev accounts. |

---

## 8d. `shelldeck` AI Dock companion

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1380 | `tray::tests::assistant_menu_id_routes_to_dock_toggle` | SDUC-434 | Green | The stable tray item ID routes only to `ToggleAiDock`. |
| SDTEST-1381 | `main::tests::ai_dock_toggle_reuses_the_existing_window` | SDUC-434 | Green | Pure window-state decision: absent creates, hidden shows, visible hides; repeated toggles never request a second creation. |
| SDTEST-1383 | `main::tests::companion_hidden_start_requires_an_available_tray` | SDUC-435 | Green | Hidden start is allowed only with a live tray; every no-tray combination leaves the main window visible and recoverable. |
| SDTEST-1384 | `main::tests::ai_dock_is_anchored_to_the_display_right_edge` | SDUC-434 | Green | The fixed 480 px Dock preserves the supplied display's vertical bounds and shares its right edge. |
| SDTEST-1385 | `main::tests::ai_dock_global_shortcut_is_parseable` | SDUC-434 | Green | The platform-specific default global shortcut is accepted by GPUI's keystroke parser. |
| SDTEST-1386 | `main::tests::command_palette_global_shortcut_is_parseable` | SDUC-436 | Green | The standalone palette's platform-specific global shortcut is accepted by GPUI's keystroke parser. |
| SDTEST-1388 | `main::tests::reachable_dynamic_icons_are_embedded` | SDUC-438 | Green | Every dynamically selected icon used by AI actions and Alert variants resolves to an SVG embedded in the application binary. |
| SDTEST-1390 | *to write* — New Request site picker searches, defaults, resets, and submits the exact site | SDUC-222, SDUC-228 | **Red / P1** | GPUI wiring: options mirror the Manage directory, the active site is selected on open, « Aucun site précis » clears targeting, close resets the draft, and submission resolves the selected id back through the current directory before posting. |
| SDTEST-1391 | `main::tests::hidden_companion_start_defers_workspace_creation` | SDUC-435 | Green | The boot policy constructs `Workspace` only for a visible main-window start; hidden companion startup keeps the lightweight root until a command needs application state. |
| SDTEST-1392 | Linux runtime smoke: hidden start → `Ctrl+Shift+Space` | SDUC-434, SDUC-435 | Green | The Dock opened at 480×1048 while the `initializing full Workspace` trace remained absent, proving the standalone controller path does not start Workspace views or pollers. |
| SDTEST-1393 | `main::tests::companion_runtime_owns_global_shortcut_routing` | SDUC-434, SDUC-436 | Green | The application-level runtime maps only the two registered global IDs to Dock/palette commands and rejects unknown IDs. |
| SDTEST-1394 | `main::tests::deferred_workspace_data_merge_preserves_ssh_alias_precedence` | SDUC-435 | Green | The deferred loader preserves startup merge semantics: SSH aliases retain precedence and unique manual connections are appended. |
| SDTEST-1395 | Linux runtime smoke: hidden start → Dock → palette | SDUC-434, SDUC-435, SDUC-436 | Green | Hidden startup and Dock-only use emitted no SSH/store/Workspace trace; the first palette invocation emitted `loading deferred Workspace connection data` exactly when the full surface became necessary. |
| SDTEST-1396 | Linux controlled process benchmark (`2e0501c` vs `4139880`) | SDUC-435 | Green | Same debug target/config: hidden-ready time moved from 663 ms to 478 ms (−28 %); RSS after 1 s remained effectively flat at 182004 vs 182092 KiB and thread count stayed 32. Indicative local measurement, not a release-build performance gate. |
| SDTEST-1397 | `global_hotkey::wayland::tests::{portal_trigger_uses_xdg_shortcut_syntax,portal_ids_round_trip_and_reject_foreign_values,unsupported_portal_key_is_rejected_before_requesting_permission}` | SDUC-434, SDUC-436 | Green | GPUI fork tests pin XDG trigger syntax (`CTRL+SHIFT+space`), activation-ID routing, and rejection before any permission request for keys the portal cannot express. Live portal dialogue/activation remains an environment test on Wayland. |
| SDTEST-1398 | `main::tests::companion_shortcuts_register_and_unregister_without_restart` | SDUC-434 | Green | A fake platform registry proves initial registration, no duplicate operation for an unchanged snapshot, immediate unregister, failed-register state retention, and a successful retry without restarting. |
| SDTEST-1399 | Linux/X11 runtime smoke: Settings palette toggle off → on | SDUC-434 | Green | The live Settings event persisted `global_palette_shortcut_enabled`, crossed the Workspace/runtime boundary, logged immediate native `unregistered`, then `registered` on re-enable without restarting ShellDeck. The preference was restored enabled afterward. |
| SDTEST-1401 | `settings::tests::shortcut_capture_requires_modifier_and_rejects_duplicate` | SDUC-434 | Green | Capture rejects a bare key, rejects the other Companion shortcut, accepts and canonicalizes a modified key, and formats the persisted combination for display. |
| SDTEST-1402 | `main::tests::custom_shortcuts_replace_native_registration_and_surface_conflicts` | SDUC-434 | Green | Changing a persisted combination unregisters the previous native binding before registering the replacement; a duplicate leaves only one binding and exposes Conflict for the other. |
| SDTEST-1403 | `main::tests::wayland_portal_result_replaces_pending_status` | SDUC-434 | Green | The runtime state reducer replaces Wayland pending with Registered or the exact asynchronous portal error. |
| SDTEST-1404 | `global_hotkey::wayland::tests::portal_registration_results_report_partial_acceptance` | SDUC-434 | Green | A partial portal response reports accepted IDs and explicit failure for omitted shortcuts, allowing Settings to leave pending state per shortcut. |
| SDTEST-1405 | `main::tests::assistant_deep_link_show_is_idempotent` | SDUC-407, SDUC-434, SDUC-435 | Green | The Assistant deep link creates a missing Dock and shows a hidden or already-visible Dock without toggling it off; routing remains independent from Workspace creation. |
| SDTEST-1406 | `command_palette::tests::keyboard_navigation_wraps_and_pages_without_leaving_results` | SDUC-436 | Green | Arrow/Tab navigation wraps, Home/End select bounds, Page Up/Page Down clamp by eight results, and an empty result set stays at index zero. |
| SDTEST-1408 | `tray::tests::ai_tasks_menu_id_routes_to_task_center` | SDUC-429, SDUC-434 | Green | The stable clickable AI-task row routes to the task center command rather than toggling the Dock or revealing the main window. |
| SDTEST-1409 | `main::tests::task_center_request_always_shows_the_existing_dock` | SDUC-429, SDUC-434 | Green | A missing Dock is created and a hidden or visible Dock is shown idempotently, so selecting the tray task indicator never hides it. |
| SDTEST-1410 | `tray::tests::macos_template_asset_is_retina_monochrome_with_transparent_background` | SDUC-434 | Green | The dedicated 36×36 Retina asset decodes, contains only black visible pixels, keeps transparent corners/background, and has non-trivial bounded mark coverage. macOS alone enables AppKit template rendering. |
| SDTEST-1411 | `tray::tests::tray_state_pump_forwards_every_snapshot_until_shutdown` | SDUC-429, SDUC-434 | Green | The shared async pump forwards every live snapshot until all publishers close. Linux consumes it on the GTK owner thread; macOS/Windows retain `muda` handles on GPUI's foreground executor. Native visual smoke remains a release check. |
| SDTEST-1412 | `workspace::ssh::tests::only_unexpected_ssh_transport_loss_notifies_with_exact_identity` | SDUC-439 | Green | The session-end reducer keeps explicit tab closes and clean remote exits silent, while unexpected transport loss emits one notification carrying the exact connection display name. |
| SDTEST-1414 | *to write* — User/Support home dashboards route to their operational tabs | SDUC-440 | **Red / P1** | GPUI integration: both modes start on Accueil, counters reflect their caches, quick actions select the exact list/composer, a recent request opens its detail, sync acts on the current Manage account, and onboarding omits Dev cards/media/shortcuts for non-Dev roles. |
| SDTEST-1415 | `workspace::tests::only_conflict_and_error_count_as_failures` | SDUC-444 | Green | `Disabled`, `Applying` and `PendingPortal` are in-flight or intentional states and must never announce a failure. |
| SDTEST-1416 | `workspace::tests::only_the_transition_into_failure_toasts` | SDUC-444 | Green | Settings republishes statuses on every save, so only the transition into a failure toasts; repeats and recoveries stay silent. |
| SDTEST-1417 | `workspace::tests::each_shortcut_reports_independently` | SDUC-444 | Green | The Dock and palette shortcuts report separately; one failing neither masks nor duplicates the other. |
| SDTEST-1418 | `workspace::tests::portal_absence_is_explained_but_other_errors_pass_through` | SDUC-444 | Green | A Wayland session with no Global Shortcuts portal reaches the user as the translated explanation, never as the ashpd/D-Bus sentence; unrecognized platform errors still arrive verbatim. |
| SDTEST-1419 | `settings::tests::portal_missing_matches_ashpd_shapes_only` | SDUC-444 | Green | The classifier catches both ashpd shapes (resolved interface name, raw `ServiceUnknown`) without swallowing keycode, `BadAccess`, or portal-refused errors. |
| SDTEST-1420 | *to write* — tray-mode registration results survive until the Workspace exists | SDUC-444 | **Red / P1** | GPUI integration: with `start_hidden`, a portal answer arriving before the first window must be the status the Workspace is seeded with — the bug behind the silent `PendingPortal` badge. Covered by construction today (the root reads the live registration state instead of a boot snapshot). |

### Application chrome (menu bar, sidebar rail, scaling)

| ID | Test | Use case | Status | Notes |
|---|---|---|---|---|
| SDTEST-1200 | `menu_bar::tests::logged_out_bar_exposes_no_session_commands` | SDUC-442 | Green | Logged out the bar offers sign-in and quit only; every session-dependent command and the whole Go menu are absent, matching the no-guest-mode rule in `.agents/roles.md`. |
| SDTEST-1201 | `menu_bar::tests::user_mode_hides_every_dev_only_command` | SDUC-442 | Green | User mode keeps requests but drops quick-connect, terminals, scripts, editor, sidebar toggle, splits and terminal zoom, and omits the Terminal menu entirely. |
| SDTEST-1202 | `menu_bar::tests::staff_consoles_follow_availability_flags` | SDUC-442 | Green | JeanClaude and Fleet appear only when configured, so a super-admin without a Jean config gets no dead entry. bext Cloud is capability-gated only. |
| SDTEST-1203 | `menu_bar::tests::view_toggles_reflect_current_state` | SDUC-442 | Green | The sidebar and menu-bar checkmarks report live state rather than a constant, so the tick never lies about what is on screen. |
| SDTEST-1204 | `menu_bar::tests::entry_ids_are_unique_across_the_whole_bar` | SDUC-442 | Green | Entry ids become GPUI `ElementId`s; duplicates would make two rows share hover/click state. Checked across all four mode/sign-in combinations with every optional menu enabled. |
| SDTEST-1205 | `menu_bar::tests::accel_renders_platform_modifiers` | SDUC-442 | Green | Shortcut hints resolve `secondary` to Cmd on macOS and Ctrl elsewhere, from the same vocabulary `actions.rs` binds with. |
| SDTEST-1210 | `sidebar::tests::total_width_is_rail_plus_panel` | SDUC-443 | Green | The rail and the panel each contribute exactly their own width. A collapsed panel must still reserve the rail, or the terminal grid is sized underneath it. |
| SDTEST-1211 | `sidebar::tests::collapsed_panel_width_is_ignored_at_any_size` | SDUC-443 | Green | A collapsed panel leaks no width back in at either end of the 180–400px resize clamp. |
| SDTEST-1213 | `sidebar::tests::activity_without_a_panel_contributes_no_panel_width` | SDUC-443 | Green | An activity with no contextual rows hides the panel while not collapsed; counting the panel there would offset every terminal past a column that is not on screen. The two reasons to hide must not compound. |
| SDTEST-1216 | `sidebar::tests::total_width_always_reserves_the_rail` | SDUC-443 | Green | The rail is unconditional across every panel-collapse / panel-less combination. Guards the v0.6.3 retirement of the hide-nav toggle, whose "hidden" state swapped in a second navigation UI that had already drifted out of sync with the rail. |
| SDTEST-1214 | `sidebar::tests::rail_lists_activities_not_destinations` | SDUC-443 | Green | JeanClaude, Fleet, bext Cloud and Settings never take a rail slot, and every rail entry either has a panel or is the spelled-out Server Sync exception — the guard against re-adding a rail icon with nothing behind it. |
| SDTEST-1215 | *to write* — panel content follows the selected activity | SDUC-443 | **Red / P1** | GPUI integration: selecting each rail activity swaps the panel to that activity's rows, a row click performs its open/focus action, empty activities show their localized hint, and a panel-less activity collapses the panel. Regression guard for the 2026-07-25 mislabelled-panel defect. |
| SDTEST-1212 | *to write* — Workspace surfaces re-layout at non-default App Font Size | SDUC-441 | **Red / P2** | GPUI integration: at 10px and 22px the User home, welcome screen and titlebar dropdowns scale proportionally while the client inset, shadow geometry and window-resize border stay in device pixels. Needs a `TestAppContext` harness we do not have yet. |

---

## 9. Cross-platform coverage (referenced from everywhere)

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
