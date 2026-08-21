# SDTEST inventory — `shelldeck-ui`, `shelldeck`, `shelldeck-update`

> Rules for this file live in [`.agents/testing.md`](../../.agents/testing.md).
> Use case IDs (`SDUC-…`) resolve in [`USE_CASES.md`](./USE_CASES.md).

**Big picture.** These crates now have broad pure-helper and reducer coverage,
including command routing, settings, tray behavior, desktop companion physics,
drag/snap/follow lifecycle, and update helpers. Live GPUI/native integration is
still the main gap because view and platform-window behavior requires real
desktop sessions; see `.agents/testing.md`.

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
| SDTEST-1056 | *to write* — refresh_command_palette produces stable action list for stable input | SDUC-303 | **Red / P1** | Reducer-style test on the action-builder. |
| SDTEST-1057 | `cloud_account.rs::allowed_modes_matches_the_tier_table` | SDUC-152, SDUC-303 | Green | Pins User-only, User+Support, and full User+Support+Dev mode lists. Workspace switcher and palette consume this exact list. |
| SDTEST-1602 | `workspace::palette::tests::logged_out_palette_contains_only_public_or_recovery_actions` | SDUC-152, SDUC-303 | Green | The standalone palette remains a recovery/login surface before authentication and excludes settings, sync, sites, requests, AI, terminals and Dev navigation even when those features are configured locally. |
| SDTEST-1058 | *to write* — action-list contains SwitchSite entries capped at 20 | SDUC-303 | **Red / P2** | |
| SDTEST-1059 | `workspace/polling.rs::no_surface_polls_behind_settings_or_while_signed_out` + `::each_surface_polls_only_where_it_is_displayed` | SDUC-168, SDUC-188, SDUC-227, SDUC-249 | Green | 2 tests. The four inline predicates (Support, Issues, Monique, bext) are extracted into one pure `should_poll(PollContext, PolledSurface)`, so the whole decision table is asserted without a GPUI context. The shared rules — Settings covers the surface, a signed-out session shows the welcome screen — are checked once at the top and swept across every mode × view × surface combination. The context carries more than the `(active_view, feature)` this line proposed: mode, sign-in and per-surface availability all gate polling, and omitting them would have made the predicate untestably incomplete. |
| SDTEST-1667 | *to write* — GPUI Monique console interaction smoke | SDUC-470, SDUC-471, SDUC-473, SDUC-474 | **Red / P1** | Open the staff console, render status/history/process metrics and multiple native accounts, submit one turn, and approve/reject a staged action without exposing alternate bot controls. Core HTTP/account contracts are Green in SDTEST-1662..1671. |

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
| SDTEST-1225 | `lib.rs::tests::update_checks_require_both_user_opt_in_and_a_verification_key` | SDUC-285 | Green | A local build without an embedded public key stays silently disabled; a signed build polls only when the persisted setting is enabled. |

### `installer.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1240 | `installer::tests::sha256_mismatch_removes_partial_download` | SDUC-283 | Green | Streams a fixture through a local HTTP socket, rejects the digest and leaves neither destination nor partial archive. |
| SDTEST-1241 | *to write* — download_and_verify streams bytes (does not buffer the whole archive) | SDUC-283 | **Red / P1** | Regression sensor for memory on macOS DMG (~200 MB). |
| SDTEST-1242 | `installer::tests::linux_binary_replacement_is_atomic_and_keeps_backup` | SDUC-284 | Green | Verifies the same-filesystem rename and retained rollback copy on Linux. |
| SDTEST-1243 | `installer::tests::windows_pending_replace_keeps_a_rollback_and_restores_it_on_failure` (`#[cfg(target_os = "windows")]`) | SDUC-284 | Green | Found a real defect: the Windows swap renamed the running exe aside and copied the new one in, but **nothing undid the rename when the copy failed** — a full disk, a locked file or an antivirus left the user with no executable at all. The Unix path had always rolled back. Extracted as `pending_replace_file` and now executed on the Windows runner. |
| SDTEST-1244 | `installer::tests::an_archive_without_the_binary_fails_before_touching_the_installation` | SDUC-284 | Green | The binary lookup runs before any swap, so an archive missing `shelldeck`/`shelldeck.exe` fails with nothing outside the staging directory touched. |
| SDTEST-1592 | *to write* — Windows `Expand-Archive` command builder doubles single quotes in paths | SDUC-284 | **Red / P2** | Extract the PowerShell command string into a pure builder fn and pin the `''`-doubling for both the archive and staging paths (an apostrophe in an install path must neither break nor inject the command — fixed 2026-08-06). `install_windows` is `cfg(windows)`, so the builder must live outside the cfg gate to be testable and type-checked on Linux. |

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
| SDTEST-1333 | *retired — see § Retired tests* | SDUC-410 | Retired | 2026-08-06: coverage moved to SDTEST-1591 (`shelldeck-core` util). |
| SDTEST-1334 | *retired — see § Retired tests* | SDUC-410 | Retired | 2026-08-06: coverage moved to SDTEST-1591 (`shelldeck-core` util). |

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
| SDTEST-1578 | `ai_assistant.rs::reopening_the_same_context_preserves_the_in_flight_request` | SDUC-414, SDUC-434 | Green | Added 2026-08-06. Reopening the Dock (same surface + title, payload may differ) re-prepares the Global context *without* invalidating the request gate, so a pending reply still lands with its loading state; a genuine surface/title switch still invalidates and drops the stale reply. Guards the Dock-reopen bug where the reply vanished without spinner or error. |
| SDTEST-1425 | *to write* — durable AI conversation renders Markdown for both roles | SDUC-414 | **Red / P1** | GPUI rendering: user and assistant messages interpret headings, emphasis, lists, links, fenced code, and tables; short user bubbles stay compact/right-aligned, long or structured bubbles use the 88% cap and grow to every wrapped line instead of clipping or collapsing to min-content; constrained conversation/header labels end with an ellipsis instead of a hard clip; the context chip never duplicates the composer action icons; the main-window overlay clips both outer right corners once, with no dim inner wedge; assistant Copy retains the exact Markdown source. Manually smoked on X11 on 2026-08-11 with short, threshold, long, hard-break, URL and structured-Markdown turns plus 8× top/bottom-right corner captures. |
| SDTEST-1600 | `ai_assistant.rs::sheet_message_width_accounts_for_the_visible_history_column` | SDUC-414 | Green | The Sheet uses its 600 px reading cap with history closed, subtracts the scaled 240 px history column when open, and also respects a viewport narrower than the nominal 780 px Sheet. Definite 88% user bubbles therefore stay inside the real conversation viewport instead of extending under history and losing their leading text. |
| SDTEST-1601 | `ai.rs::conversation_titles_mark_new_and_legacy_truncation_with_an_ellipsis` | SDUC-414 | Green | New 56-character conversation titles append a Unicode ellipsis when the first message continues; legacy stored titles are recognized against the retained first user message and repaired for display. The history renderer also gives its title/meta column a definite post-button width so GPUI shapes an ellipsis instead of hard-clipping the row. Manually validated on X11 on 2026-08-17 with three long title/context pairs in an isolated profile, including the selected-row background. |
| SDTEST-1621 | `adabraka-ui::display::rich_text::tests::compact_heading_scale_is_relative_and_document_sizes_are_unchanged` | SDUC-414 | Green | A 12.5 px compact body produces the 18/16.5/15/14/13.25/12.5 px H1–H6 ramp, while document H1/H4/H6 stay 32/20/16 px. Manually validated with H1–H4 in an isolated 480 px GPUI render on X11 on 2026-08-17. |
| SDTEST-1426 | `ai_workflow.rs::only_read_only_free_form_ai_capabilities_render_as_markdown` | SDUC-418 | Green | Exhaustive capability classification renders Support/request summaries and Script explain/review as Markdown, while structured triage/diagnostic/naming and raw replies/scripts/commands/dispatch payloads cannot enter the Markdown path. |
| SDTEST-1604 | `markdown.rs::markdown_links_only_accept_absolute_http_urls_without_credentials` | SDUC-460 | Green | The application link boundary accepts normal public and loopback HTTP(S), while rejecting JavaScript/data/file/custom schemes, relative URLs, missing hosts and deceptive credential-bearing authorities before clipboard/open state exists. |
| SDTEST-1605 | `markdown.rs::ecosystem_domains_require_an_exact_host_or_subdomain_boundary` | SDUC-460 | Green | External-warning suppression is limited to exact Inklura/Bext/ShellDeck hosts and true subdomains; suffix and query-string spoofing remain external. |
| SDTEST-1606 | `adabraka-ui::display::markdown::tests::unsafe_markdown_destinations_stay_inert` | SDUC-460 | Green | The renderer itself preserves an unsafe link label without emitting an interactive `RichInline::Link`, providing defense in depth below the application popover. |
| SDTEST-1607 | `adabraka-ui::display::markdown::tests::markdown_images_are_links_not_automatic_network_fetches` | SDUC-460 | Green | A Markdown image becomes a deliberate validated link and never reaches the renderer's network-backed image block. |
| SDTEST-1608 | `adabraka-ui::display::markdown::tests::raw_html_is_not_exposed_as_rendered_content` | SDUC-460 | Green | Raw block/inline HTML tags and semantics are discarded while harmless inner text and surrounding Markdown prose remain plain and renderable. |
| SDTEST-1609 | *to write* — every free-form prose surface renders the shared secure Markdown interaction | SDUC-460 | **Red / P1** | GPUI integration: Requests/Tickets, Assistant chat/read-only tasks, Clippy result, Fleet prompt/result, Monique conversation/actions and site notes render headings/lists/code/tables; unsafe destinations and image syntax never fetch/open; every valid link shows the same copy/open panel and external-host warning; Copy/Edit/diff retain exact source. |
| SDTEST-1620 | `adabraka-ui::display::markdown::tests::adjacent_email_label_becomes_the_autolink_label_only` + link-shape regression table | SDUC-460 | Green | A Postmark/Outlook `[generated alt]<https://destination>` pair renders one link whose visible text is the generated label. Standard labelled Markdown, standalone/spaced autolinks and unsafe schemes retain their existing text, interaction and security behavior. Manually validated on the real `RE: CORRECTION JEU RENTREE` e-mail on X11 on 2026-08-17: no raw URL/brackets remained and clicking the label opened the external-domain confirmation. |
| SDTEST-1610 | `external_content.rs::slack_links_use_their_label_or_readable_url` | SDUC-461 | Green | Labeled Slack links and mail addresses expose their human label while a bare HTTP(S) link remains readable as its URL. |
| SDTEST-1611 | `external_content.rs::slack_references_are_readable_but_unknown_angle_text_is_preserved` | SDUC-461 | Green | Known channel, broadcast and user-group references gain readable prefixes, while unknown angle-bracket content survives unchanged. |
| SDTEST-1612 | `external_content.rs::external_titles_are_trimmed_and_kept_on_one_line` | SDUC-461 | Green | External titles collapse line breaks and repeated whitespace into the single-line label required by compact request surfaces. |
| SDTEST-1613 | *to write* — User request detail scrolls independently of its reply composer | SDUC-228 | **Red / P1** | GPUI integration: with a thread taller than the 480 px detail sheet, wheel to both limits and confirm that only `user-sheet-body` moves while the non-shrinking composer footer remains visible and interactive. Manually validated on X11 on 2026-08-17 after rebuilding `dev`; the automated GPUI harness is still missing. |
| SDTEST-1428 | *to write* — both Assistant surfaces open the routed unsent request form | SDUC-452 | **Red / P0** | GPUI wiring: the main sheet closes into a prefilled New Request sheet; the standalone Dock reveals the main window and does the same; priority is preserved, normal chat stays in-place, logged-out routing only warns, and neither path calls the create API before the existing Create action. |
| SDTEST-1429 | `ai_assistant.rs::quick_actions_distinguish_immediate_submit_from_composer_prefill` | SDUC-453 | Green | Script Generate/Convert, Terminal Command, and Create Request on Issue/Terminal use dedicated editable-template keys; context-complete analyses such as Summary submit immediately, while chat Triage uses readable prose rather than the workflow-only JSON prompt. Since the 2026-08 tile redesign the two modes are no longer visually distinguished (no tooltips, no button-variant coding) — the pinned contract is purely behavioral: Submit sends immediately, Prefill only fills the composer. |
| SDTEST-1431 | `workspace::ai::tests::assistant_request_target_resolution_rejects_stale_and_ambiguous_matches` | SDUC-454 | Green | Exact case-insensitive ID/title and unique partial matches resolve; stale model-proposed IDs, missing targets, and ambiguous partial titles cannot navigate. |
| SDTEST-1432 | *to write* — both Assistant surfaces reuse typed workflows without finalizing actions | SDUC-454 | **Red / P0** | GPUI wiring: Script opens an unsaved generated form; Terminal and Support open the exact active target with routed instructions; Monique stages the existing confirmation; request navigation opens the resolved detail; missing/stale targets warn; no save, execute, send, or dispatch occurs before the existing explicit action. |
| SDTEST-1433 | `workspace::requests::tests::user_issue_refresh_forces_owner_scope_while_support_keeps_triage_filters` | SDUC-228 | Green | User polling always emits `mine=1` with no stale Support filters; Support retains the active triage query. The User overview also applies `is_my_issue` defensively before counting or rendering recent titles. |
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
| SDTEST-1363 | `workspace/ai_routing.rs::a_resumed_name_never_lands_on_a_different_form` + `::terminal_names_by_session_id_and_the_request_draft_by_being_open` | SDUC-426 | Green | 2 tests. Found a real defect, not a drift risk: Script and Tunnel naming carried the **literal** `"script-form"` / `"tunnel-form"` as their target, which named no entity at all. Since a naming task can be parked and resumed from the task center once another form is open, an accepted name landed on whichever form happened to be open. Both now carry the form's GPUI entity id, and `resolve_naming_application` compares it. The Accept-only and Cancel halves stay uncovered — those are view wiring. |
| SDTEST-1364 | `ai.rs::action_plan_rejects_mismatched_payload_and_redacts_content_from_audit` | SDUC-427, SDUC-428 | Green | Rejects kind/payload mismatches and proves audit metadata excludes the executable payload. |
| SDTEST-1365 | `workspace/ai_routing.rs::only_an_automatic_policy_on_a_non_high_risk_action_skips_the_dialog` + `::an_automatic_policy_never_promotes_an_action_the_caller_cannot_perform` + `::each_payload_is_gated_by_the_surface_that_owns_it` + `::a_signed_out_session_performs_no_ai_action` | SDUC-427 | Green | 4 tests. The staging decision is extracted into a pure `route_ai_action`, so the safety rules of [`.agents/ai.md`](../../.agents/ai.md) are checkable without a GPUI context: `High` risk always confirms whatever the policy says, `Preparation` never executes, and **permission is decided before policy** so an `Automatic` setting cannot promote an action the caller may not perform. The automatic path still routes through `confirm_ai_action`, which revalidates the target — a skipped dialog, not a bypassed gate. What stays uncovered here is the *dialog rendering* half (Accept inserts, Cancel is inert): that is genuine view wiring and belongs to whatever harness REL-006's remaining lines get. |
| SDTEST-1366 | `workspace/ai_routing.rs::a_timeout_never_stops_a_run_it_does_not_own` | SDUC-428 | Green | No fake clock needed as this line assumed: what matters is not *when* the timer fires but *what it decides* when it does. That decision is extracted into `ai_timeout_outcome` and both timers — script and terminal — call it. An AI timeout is detached and cannot be cancelled, so it always outlives its run; firing on whatever occupies the target at that moment would stop a newer, unrelated execution. Covers the three ways ownership is lost: a newer action took the target, the run finished and was untracked, and the target is no longer running. |
| SDTEST-1368 | *to write* — AI task center routes exact targets and only exposes valid actions | SDUC-429 | **Red / P0** | GPUI wiring: actionable count matches the titlebar badge; resume/open/stop/delete route by task ID, active tasks survive sheet closure, and stale active states recover as cancelled after restart. |
| SDTEST-1370 | *to write* — AI policy controls drive the executable workflow action | SDUC-430 | **Red / P0** | GPUI wiring: Settings persists each capability independently; Prepare hides/blocks Execute, Confirm opens the second dialog, Automatic executes moderate actions directly, and High risk still opens confirmation. |
| SDTEST-1372 | *to write* — Terminal diagnostic steps remain explicit and target-safe | SDUC-431 | **Red / P0** | GPUI wiring: structured steps render without raw JSON, each step revalidates the active session and opens high-risk confirmation, full-plan execution advances only after matching OSC 133 completion, stops on failure, and Ctrl+C remains available. |
| SDTEST-1374 | `issue_attachments.rs::rejects_extension_spoofing` + `recognizes_png_magic` | SDUC-432 | Green | Pure local intake guard: accepted formats are identified by bytes, never filename alone. |
| SDTEST-1375 | *to write* — attachment picker routes URL/paste/drop/file/capture drafts to the exact composer | SDUC-432 | **Red / P0** | GPUI integration: each source adds one removable preview to the active New Request, request comment, Support request comment, ticket reply, or internal note; changing target clears drafts; submission uploads once and preserves drafts on failure. |
| SDTEST-1593 | *to write* — Linux area capture distinguishes tool-missing from user cancellation | SDUC-432 | **Red / P2** | `issue_attachments.rs::capture_region` filters `gnome-screenshot`/`spectacle`/`import` through `util::executable_on_path` and must return the dedicated `attachments.capture.tool_missing` error when none is installed, and the cancelled message otherwise. Extract the installed-tools decision into a pure helper to test without spawning anything. |
| SDTEST-1423 | `attachment_annotator.rs::empty_export_preserves_original_draft` + `annotation_export_is_a_valid_changed_png` | SDUC-432 | Green | Pins cancel/no-op preservation and proves a real annotation exports a distinct valid PNG without touching the original capture. |
| SDTEST-1376 | *to write* — shared multi-line Input follows native wrapped-line editing semantics | SDUC-433 | **Red / P0** | GPUI integration: Up/Down retain visual X, Shift selection paints across hard/soft lines, Home/End stay on the visual row, mouse placement matches the glyph, wheel input scrolls a capped field, and `max_rows` keeps the caret visible. |
| SDTEST-1377 | `workspace/palette.rs::a_regular_account_is_offered_no_support_or_dev_command` + `::inklura_support_reaches_triage_but_never_dev` + `::a_super_admin_gets_dev_commands_only_while_in_dev_mode` + `::ai_commands_appear_only_when_their_backend_is_configured` | SDUC-152, SDUC-310 | Green | 4 tests. **Not GPUI integration** as this line assumed: `base_palette_actions` is already a pure associated fn, so the whole role × surface matrix is asserted directly. The load-bearing case is a super-admin standing in User mode — Dev commands follow the *surface*, not the privilege, and gating them on capability alone would leak the Dev block into the customer-facing palette. Execution-side gating is deliberately left where it is: every Dev route goes through `enter_dev_mode`, which both checks the capability **and** performs the switch, so a central pre-gate would duplicate it rather than consolidate it. The Settings tab half (`set_dev_tabs_enabled`) stays uncovered — its only logic is three lines bouncing the active tab, and its input `can_access_mode(Dev)` is already pinned by SDTEST-184. |

---

## 8d. `shelldeck` AI Dock and desktop companion

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1380 | `tray::tests::assistant_menu_id_routes_to_dock_toggle` | SDUC-434 | Green | The stable tray item ID routes only to `ToggleAiDock`. |
| SDTEST-1381 | `main::tests::ai_dock_toggle_reuses_the_existing_window` | SDUC-434 | Green | Pure window-state decision: absent creates, hidden shows, visible hides; repeated toggles never request a second creation. |
| SDTEST-1383 | `main::tests::companion_hidden_start_requires_an_available_tray` | SDUC-435 | Green | Hidden start is allowed only with a live tray; every no-tray combination leaves the main window visible and recoverable. |
| SDTEST-1384 | `main::tests::ai_dock_is_anchored_to_the_display_right_edge` | SDUC-434 | Green | The fixed 480 px Dock preserves the supplied display's vertical bounds and shares its right edge. |
| SDTEST-1385 | `main::tests::ai_dock_global_shortcut_is_parseable` | SDUC-434 | Green | The platform-specific default global shortcut is accepted by GPUI's keystroke parser. |
| SDTEST-1386 | `main::tests::command_palette_global_shortcut_is_parseable` | SDUC-436 | Green | The standalone palette's platform-specific global shortcut is accepted by GPUI's keystroke parser. |
| SDTEST-1388 | `main::tests::reachable_dynamic_icons_are_embedded` | SDUC-438 | Green | Every dynamically selected icon used by AI actions and Alert variants resolves to an SVG embedded in the application binary. |
| SDTEST-1424 | `main::tests::contextual_monolith_animations_are_embedded_webp_assets` | SDUC-438 | Green | Every contextual Monolith motion used by AI generation and terminal/site empty states resolves to an embedded RIFF/WebP payload. |
| SDTEST-1390 | *to write* — New Request site picker searches, defaults, resets, and submits the exact site | SDUC-222, SDUC-228 | **Red / P1** | GPUI wiring: options mirror the Manage directory, the active site is selected on open, « Aucun site précis » clears targeting, close resets the draft, and submission resolves the selected id back through the current directory before posting. |
| SDTEST-1391 | `main::tests::hidden_companion_start_defers_workspace_creation` | SDUC-435 | Green | The boot policy constructs `Workspace` only for a visible main-window start; hidden companion startup keeps the lightweight root until a command needs application state. |
| SDTEST-1393 | `main::tests::companion_runtime_owns_global_shortcut_routing` | SDUC-434, SDUC-436 | Green | The application-level runtime maps only the two registered global IDs to Dock/palette commands and rejects unknown IDs. |
| SDTEST-1394 | `main::tests::deferred_workspace_data_merge_preserves_ssh_alias_precedence` | SDUC-435 | Green | The deferred loader preserves startup merge semantics: SSH aliases retain precedence and unique manual connections are appended. |
| SDTEST-1396 | Linux controlled process benchmark (`2e0501c` vs `4139880`) | SDUC-435 | Green | Same debug target/config: hidden-ready time moved from 663 ms to 478 ms (−28 %); RSS after 1 s remained effectively flat at 182004 vs 182092 KiB and thread count stayed 32. Indicative local measurement, not a release-build performance gate. |
| SDTEST-1397 | `global_hotkey::wayland::tests::{portal_trigger_uses_xdg_shortcut_syntax,portal_ids_round_trip_and_reject_foreign_values,unsupported_portal_key_is_rejected_before_requesting_permission}` | SDUC-434, SDUC-436 | Green | GPUI fork tests pin XDG trigger syntax (`CTRL+SHIFT+space`), activation-ID routing, and rejection before any permission request for keys the portal cannot express. Live portal dialogue/activation remains an environment test on Wayland. |
| SDTEST-1398 | `main::tests::companion_shortcuts_register_and_unregister_without_restart` | SDUC-434 | Green | A fake platform registry proves initial registration, no duplicate operation for an unchanged snapshot, immediate unregister, failed-register state retention, and a successful retry without restarting. |
| SDTEST-1399 | Linux/X11 runtime smoke: Settings palette toggle off → on | SDUC-434 | Green | The live Settings event persisted `global_palette_shortcut_enabled`, crossed the Workspace/runtime boundary, logged immediate native `unregistered`, then `registered` on re-enable without restarting ShellDeck. The preference was restored enabled afterward. |
| SDTEST-1401 | `settings::tests::shortcut_capture_requires_modifier_and_rejects_duplicate` | SDUC-434 | Green | Capture rejects a bare key, rejects the other Companion shortcut, accepts and canonicalizes a modified key, and formats the persisted combination for display. |
| SDTEST-1402 | `main::tests::custom_shortcuts_replace_native_registration_and_surface_conflicts` | SDUC-434 | Green | Changing a persisted combination unregisters the previous native binding before registering the replacement; a duplicate leaves only one binding and exposes Conflict for the other. |
| SDTEST-1403 | `main::tests::wayland_portal_result_replaces_pending_status` | SDUC-434 | Green | The runtime state reducer replaces Wayland pending with Registered or the exact asynchronous portal error. |
| SDTEST-1404 | `global_hotkey::wayland::tests::portal_registration_results_report_partial_acceptance` | SDUC-434 | Green | A partial portal response reports accepted IDs and explicit failure for omitted shortcuts, allowing Settings to leave pending state per shortcut. |
| SDTEST-1405 | `main::tests::assistant_deep_link_show_is_idempotent` | SDUC-407, SDUC-434, SDUC-435 | Green | After the shared auth gate, the Assistant deep link creates a missing Dock and shows a hidden or already-visible Dock without toggling it off. |
| SDTEST-1406 | `command_palette::tests::keyboard_navigation_wraps_and_pages_without_leaving_results` | SDUC-436 | Green | Arrow/Tab navigation wraps, Home/End select bounds, Page Up/Page Down clamp by eight results, and an empty result set stays at index zero. |
| SDTEST-1408 | `tray::tests::ai_tasks_menu_id_routes_to_task_center` | SDUC-429, SDUC-434 | Green | The stable clickable AI-task row routes to the task center command rather than toggling the Dock or revealing the main window. |
| SDTEST-1409 | `main::tests::task_center_request_always_shows_the_existing_dock` | SDUC-429, SDUC-434 | Green | A missing Dock is created and a hidden or visible Dock is shown idempotently, so selecting the tray task indicator never hides it. |
| SDTEST-1410 | `tray::tests::macos_template_asset_is_retina_monochrome_with_transparent_background` | SDUC-434 | Green | The dedicated 36×36 Retina asset decodes, contains only black visible pixels, keeps transparent corners/background, and has non-trivial bounded mark coverage. macOS alone enables AppKit template rendering. |
| SDTEST-1411 | `tray::tests::tray_state_pump_forwards_every_snapshot_until_shutdown` | SDUC-429, SDUC-434 | Green | The shared async pump forwards every live snapshot until all publishers close. Linux consumes it on the GTK owner thread; macOS/Windows retain `muda` handles on GPUI's foreground executor. Native visual smoke remains a release check. |
| SDTEST-1412 | `workspace::ssh::tests::only_unexpected_ssh_transport_loss_notifies_with_exact_identity` | SDUC-439 | Green | The session-end reducer keeps explicit tab closes and clean remote exits silent, while unexpected transport loss emits one notification carrying the exact connection display name. |
| SDTEST-1414 | *to write* — User/Support home dashboards route to their operational tabs | SDUC-440 | **Red / P1** | GPUI integration: both modes start on Accueil; every Support counter clears stale constraints and opens the exact advertised queue; priority-ticket and recent-request rows open their real detail; User quick actions select the exact list/composer; sync acts on the current Manage account; onboarding omits Dev cards/media/shortcuts for non-Dev roles. |
| SDTEST-1614 | `support_view::home::tests::support_home_targets_route_to_the_expected_section_and_ticket_filter` | SDUC-440 | Green | The five home destinations map exhaustively to Requests or to the exact All/Open/SLA/Unassigned ticket filter, preventing a visually correct card from opening the wrong queue. |
| SDTEST-1615 | `support_view::home::tests::support_home_attention_orders_sla_then_urgent_then_unassigned` | SDUC-440 | Green | The attention preview excludes closed tickets and orders actionable work by SLA risk, urgent priority, missing owner, then recency. |
| SDTEST-1616 | `workspace::render::tests::workspace_status_bar_is_exclusive_to_authenticated_dev_mode` | SDUC-437 | Green | User, Support and the mandatory welcome screen omit the technical footer; an authenticated Dev workspace retains it. |
| SDTEST-1617 | *to write* — Support Tickets and Requests expose the same functional refresh control | SDUC-462 | **Red / P1** | GPUI integration: both list headers show the same icon, localized label, geometry and focusable `Button`; clicking Tickets emits `Refresh`, clicking Requests emits `IssuesRefresh`, and neither control alters filters or selection. Manually validated on X11 on 2026-08-17 after rebuilding `dev`; the automated GPUI harness is still missing. |
| SDTEST-1662 | `onboarding_view::tests::sdtest_1662_the_run_follows_the_mode_and_the_modes_slide_follows_capability` | SDUC-469 | Green | One table pins all three runs plus the cross cases: the sequence follows the effective mode while the closing modes slide follows `allowed_modes`, so a super-admin in User mode gets the User run with the Dev-accented slide. Across every tier and landing mode the modes slide appears exactly once, only when switchable, and always last. |
| SDTEST-1663 | `onboarding_view::tests::sdtest_1663_every_slide_resolves_its_copy` | SDUC-469 | Green | Every slide resolves `title`/`intro`/`media_caption` and a title/body pair per bullet, plus the shared mode and shortcut keys. `rust_i18n` renders a missing key as the key itself, so a typo in `key()`/`bullets()` or a forgotten locale line would otherwise ship `onboarding.dev.tunnels.presets_body` to the user. Uses the ambient locale deliberately — `set_locale` is process-global and races the suite; SDTEST-1302 already pins fr/en parity. |
| SDTEST-1664 | `onboarding_view::tests::sdtest_1664_every_slide_asset_is_embedded_and_listed` | SDUC-469 | Green | Each slide's artwork appears in both `main.rs` tables (`include_bytes!` and `Assets::list`). Neither is compiler-checked, and an unregistered path fails the image load silently, leaving an empty hero zone. |
| SDTEST-1665 | `onboarding_view::tests::sdtest_1665_runs_cover_every_slide` | SDUC-469 | Green | The union of every reachable run accounts for all fifteen slides, so the test-local `ALL_STEPS` cannot drift from the enum and no slide becomes unreachable. |
| SDTEST-1666 | *to write* — the tour card keeps its footer reachable at every window size and UI scale | SDUC-469 | **Red / P2** | GPUI integration: the card caps at 90% of window height, the body scrolls, and Passer/Précédent/Terminer stay visible on the longest run (Dev: three modes plus four shortcut rows). Manually validated on X11 on 2026-08-21 at 1210×810 for all three roles after the clipped-footer fix; the automated harness is still missing. |
| SDTEST-1667 | `workspace::palette::tests::sdtest_1667_quit_is_never_the_preselected_first_entry` | SDUC-152 | Green | La palette présélectionne son premier résultat. « Quitter » y figurait en tête, si bien qu'ouvrir la palette et valider fermait l'application — sans recours clavier, puisque Échap n'était pas délivré non plus. Le test vérifie sur les quatre combinaisons de rôle que Quitter reste joignable mais jamais en première position. |
| SDTEST-1668 | `settings::tests::sdtest_1668_shortcut_failures_are_classified_never_dumped` | SDUC-449 | Green | Aucun échec d'enregistrement de raccourci global ne doit atteindre l'écran sous forme de `Debug` Rust. L'onglet Général affichait littéralement `X11 error X11Error { error_kind: Access, error_code:`, tronqué au milieu, dès qu'une autre application détenait déjà la combinaison — le cas courant, pas une anomalie. Le test pinne les trois classes, dont la forme exacte relevée sur cette machine. |
| SDTEST-1618 | `support_view::tests::support_master_detail_switches_at_a_scale_aware_width` | SDUC-463 | Green | The compact master/detail predicate switches immediately below 760 px at baseline rem size and scales the same threshold at 2× UI size. |
| SDTEST-1619 | *to write* — Support master lists remain usable from minimum to wide window sizes | SDUC-463 | **Red / P2** | GPUI integration: at 600, 1200 and 1500 px, Tickets and Requests share the same bounded proportional column; below the compact threshold list and detail replace each other, Back restores the filtered list, copy never overflows, resizing preserves the open record, and rendering an empty detail performs no selection or network action. Manually validated on X11 on 2026-08-17 at 600 and 1210 px for both Tickets and Requests, including each distinct Back action and the wide empty detail. |
| SDTEST-1415 | `workspace::tests::only_conflict_and_error_count_as_failures` | SDUC-444 | Green | `Disabled`, `Applying` and `PendingPortal` are in-flight or intentional states and must never announce a failure. |
| SDTEST-1416 | `workspace::tests::only_the_transition_into_failure_toasts` | SDUC-444 | Green | Settings republishes statuses on every save, so only the transition into a failure toasts; repeats and recoveries stay silent. |
| SDTEST-1417 | `workspace::tests::each_shortcut_reports_independently` | SDUC-444 | Green | The Dock and palette shortcuts report separately; one failing neither masks nor duplicates the other. |
| SDTEST-1418 | `workspace::tests::portal_absence_is_explained_but_other_errors_pass_through` | SDUC-444 | Green | A Wayland session with no Global Shortcuts portal reaches the user as the translated explanation, never as the ashpd/D-Bus sentence; unrecognized platform errors still arrive verbatim. |
| SDTEST-1419 | `settings::tests::portal_missing_matches_ashpd_shapes_only` | SDUC-444 | Green | The classifier catches both ashpd shapes (resolved interface name, raw `ServiceUnknown`) without swallowing keycode, `BadAccess`, or portal-refused errors. |
| SDTEST-1420 | *to write* — tray-mode registration results survive until the Workspace exists | SDUC-444 | **Red / P1** | GPUI integration: with `start_hidden`, a portal answer arriving before the first window must be the status the Workspace is seeded with — the bug behind the silent `PendingPortal` badge. Covered by construction today (the root reads the live registration state instead of a boot snapshot). |
| SDTEST-1478 | `companion_desktop.rs::runtime_route_requires_enabled_character` | SDUC-447, SDUC-448 | Green | The overlay exists only when desktop mode is explicitly enabled and the selected roster entry is not None. |
| SDTEST-1479 | `companion_desktop.rs::runtime_uses_core_simulation_and_clamps_after_monitor_removal` | SDUC-448, SDUC-449 | Green | The native owner consumes the tested core simulation and recovers into a smaller surviving work area. |
| SDTEST-1480 | `companion_desktop.rs::paused_and_reduced_motion_request_no_continuous_frames` | SDUC-448, SDUC-449 | Green | Pause and reduced motion make the overlay event-idle rather than maintaining a render poll. |
| SDTEST-1481 | `companion_desktop.rs::character_assets_route_to_existing_pngs` | SDUC-447 | Green | Runtime roster IDs resolve to embedded production PNG paths. |
| SDTEST-1483 | `.github/workflows/release.yml` Linux/macOS/Windows compile matrix | SDUC-448, SDUC-449 | Green | The GPUI top-level movement and geometry APIs are platform-gated; every release target must type-check before assets or manifests publish. |
| SDTEST-1484 | `tray::tests::clippy_menu_id_routes_directly_to_clippy` | SDUC-445 | Green | The stable native menu id opens the dedicated Clippy surface instead of toggling the generic Assistant or revealing the main window. |
| SDTEST-1485 | `companion_desktop.rs::external_window_target_perches_above_the_window_inside_the_work_area` | SDUC-448, SDUC-449 | Green | Eligible external window tops produce an on-screen perch target; tiny invalid surfaces are rejected before the native overlay moves. |
| SDTEST-1486 | `companion_desktop.rs::frame_elapsed_uses_real_refresh_delta_after_the_first_frame` | SDUC-448 | Green | Animation-frame simulation uses real elapsed time and accumulates sub-33 ms refresh deltas, so 60 Hz and 30 Hz displays both advance at the configured fixed-step speed without freezing or running fast. |
| SDTEST-1487 | `ai_assistant.rs::automatic_clipboard_import_requires_opt_in_and_an_empty_draft` | SDUC-445, SDUC-446 | Green | Shortcut clipboard import is disabled by default and never overwrites an existing Clippy draft. |
| SDTEST-1488 | `ai_assistant.rs::backend_result_must_satisfy_clippy_proposal_bounds_before_display` | SDUC-445, SDUC-446 | Green | Blank or oversized model output is rejected through the core proposal contract before it can enter the result preview, diff, clipboard, or edit paths. |
| SDTEST-1489 | `settings::tests::choosing_a_visible_character_enables_it_and_none_disables_it` | SDUC-447, SDUC-450 | Green | Selecting a real mascot cannot leave a hidden selected-but-disabled state; choosing None reliably turns the desktop runtime off. |
| SDTEST-1490 | `tray::tests::choose_character_menu_id_routes_to_targeted_settings` | SDUC-450 | Green | The stable native tray ID opens the targeted character settings command rather than pausing movement or opening generic Settings. |
| SDTEST-1603 | `main::tests::tray_session_commands_require_authentication` | SDUC-152, SDUC-412, SDUC-434 | Green | Show, palette and quit remain public recovery commands; Assistant, Clippy, AI tasks, character controls and pinned connections are independently auth-gated by the dispatcher rather than trusting disabled native rows. |
| SDTEST-1491 | `companion_desktop.rs::user_drag_preserves_grab_offset_and_routes_across_display_bounds` | SDUC-448, SDUC-451 | Green | Native hit windows exactly track the selected mascot scale; dragging keeps the pointer-to-character grab offset, scales deltas for mixed-DPI Windows displays, supports negative monitor origins, selects the display under the character center, and clamps the final window origin to it. |
| SDTEST-1492 | `companion_desktop.rs::clicks_start_bounded_playful_reactions_from_the_current_position` | SDUC-448, SDUC-451 | Green | A click starts a bounded short hop; a sequential double-click upgrades that in-flight hop to the opposite display edge while preserving vertical work-area bounds and continuous-frame policy only during motion. |
| SDTEST-1493 | `main.rs::every_authored_character_state_is_embedded_in_the_binary_asset_source` | SDUC-447, SDUC-451 | Green | The production asset source lists and loads all 48 idle, listening, thinking, success, warning, error, and sleeping resources across the six mascots, preventing live drag/click reactions from resolving to missing embedded files. |
| SDTEST-1494 | `companion_desktop.rs::procedural_pose_values_stay_inside_safe_visual_bounds` | SDUC-451 | Green | Every mascot and semantic visual state produces bounded translation, scale, rotation, opacity, and sparkle values while continuing to render the production PNG. |
| SDTEST-1495 | `companion_desktop.rs::character_personalities_are_visibly_and_kinetically_distinct` | SDUC-451 | Green | Grounded and airborne mascots have distinct speed, bounce, tilt, and pose profiles rather than sharing one generic motion. |
| SDTEST-1496 | `companion_desktop.rs::playful_targets_are_bounded_varied_and_cooldown_aware` | SDUC-448, SDUC-451 | Green | Deterministic personality-driven roam candidates remain inside the work area, vary their routes, examine every candidate exactly once before repeating a cooled action, and keep speeds within the character profile. |
| SDTEST-1497 | `companion_desktop.rs::frame_scheduling_policy_is_event_driven_and_honors_reduced_motion` | SDUC-449, SDUC-451 | Green | Static idle and one-shot idle flourishes do not request a companion runtime frame loop; movement/reaction/landing do, while reduced/off/still modes suppress continuous frames. |
| SDTEST-1498 | `companion_desktop.rs::dpi_aware_drag_threshold_preserves_clicks_and_native_moves_are_gated` | SDUC-448, SDUC-451 | Green | Drag classification scales with desktop coordinates on high-DPI displays; sub-threshold pointer jitter leaves the overlay stationary for click delivery, and visual-only animation frames cannot issue redundant native origin updates after initial placement. |
| SDTEST-1499 | `companion_desktop.rs::procedural_pose_phase_tracks_wall_clock_time_not_refresh_count` | SDUC-451 | Green | Procedural pose phase is derived from elapsed wall-clock time, so visual animation cadence remains consistent on 30 Hz, 60 Hz, and higher-refresh displays. |
| SDTEST-1500 | `companion_desktop.rs::attachment_preserves_top_edge_offset_when_window_moves` | SDUC-448, SDUC-451 | Green | A perched character retains the same pixel offset from the external window's left edge when the window moves, enters the stable Perched state, and marks exactly one native origin update. |
| SDTEST-1501 | `companion_desktop.rs::attachment_clamps_offset_when_window_resizes` | SDUC-448, SDUC-451 | Green | Resizing a followed window clamps the saved perch offset to its new usable top edge instead of leaving the companion floating beyond the window. |
| SDTEST-1502 | `companion_desktop.rs::attachment_unchanged_window_does_not_request_native_move` | SDUC-448, SDUC-451 | Green | Repeating an identical external-window snapshot keeps the attachment alive without scheduling a redundant native overlay move. |
| SDTEST-1503 | `companion_desktop.rs::attachment_missing_window_detaches_safely` | SDUC-449, SDUC-451 | Green | When the stable native window ID disappears from the next snapshot, the runtime invalidates the follow generation, clears the attachment, and returns to normal event-driven roaming. |
| SDTEST-1504 | `companion_desktop.rs::attachment_cancellation_and_scheduler_policy_bump_generations` | SDUC-449, SDUC-451 | Green | The 100 ms follow refresh exists only while attached; drag and explicit cancellation invalidate stale callbacks by generation and leave no scheduled watcher behind. |
| SDTEST-1512 | `companion_desktop.rs::drag_release_snap_candidate_ranking_prefers_vertical_gap_then_horizontal_distance` | SDUC-451 | Green | Drag release near multiple eligible tops chooses deterministically by smallest vertical gap, then nearest horizontal distance. |
| SDTEST-1513 | `companion_desktop.rs::drag_release_rejects_maximized_like_snap_windows` | SDUC-451 | Green | Maximized-like windows are rejected for drag-release snapping and physics platforms. |
| SDTEST-1514 | `companion_desktop.rs::drag_release_does_not_snap_outside_vertical_or_overlap_thresholds` | SDUC-451 | Green | Candidate tops outside the vertical snap band or horizontal overlap threshold are ignored. |
| SDTEST-1515 | `companion_desktop.rs::drag_release_velocity_is_bounded_from_logical_desktop_samples` | SDUC-451 | Green | Release velocity comes from logical desktop samples and is bounded by horizontal speed and terminal-velocity limits. |
| SDTEST-1516 | `companion_desktop.rs::dynamic_fall_lands_on_window_and_maps_attachment` | SDUC-451 | Green | After release without immediate snap, gravity lands on a window top and maps the stable contact back to its native window ID for attachment. |
| SDTEST-1517 | `companion_desktop.rs::dynamic_fall_lands_on_display_floor_without_window` | SDUC-451 | Green | Falling with no eligible window lands on the display work-area floor and transitions to Sleeping. |
| SDTEST-1518 | `companion_desktop.rs::missing_attached_window_falls_only_when_full_motion_allowed` | SDUC-451 | Green | A disappeared attachment restarts gravity in full motion, but reduced motion suppresses dynamic falling. |
| SDTEST-1519 | `companion_desktop.rs::reduced_motion_release_never_starts_dynamic_frames` | SDUC-451 | Green | Reduced-motion drag release never enters Dynamic or requests continuous physics frames. |
| SDTEST-1520 | `companion_desktop.rs::subthreshold_drag_jitter_does_not_move_native_overlay` | SDUC-451 | Green | Pointer jitter below the drag threshold preserves click delivery and does not move the native overlay. |
| SDTEST-1521 | `companion_desktop.rs::immediate_snap_release_starts_attachment_follow_lifecycle` | SDUC-451 | Green | A valid near-top release snaps immediately, creates an attachment, and starts the follow lifecycle without a redundant timer. |
| SDTEST-1522 | `companion_desktop.rs::floor_landing_policy_schedules_roam_only_when_unattached_and_not_dragging` | SDUC-451 | Green | Screen-floor landing schedules normal roaming only for unattached, non-dragging landings. |
| SDTEST-1523 | `companion_desktop.rs::disappeared_attachment_requests_dynamic_frame_restart_when_falling` | SDUC-451 | Green | Losing the attached window requests a dynamic frame restart when the character begins falling. |
| SDTEST-1524 | `companion_desktop.rs::window_climbing_disabled_prevents_snap_platforms_and_window_attachment` | SDUC-451 | Green | Disabling window climbing prevents snap platforms and window attachment, so the character falls to the screen floor. |
| SDTEST-1525 | `companion_desktop.rs::maximized_like_rejects_taskbar_inset_but_keeps_ordinary_large_window` | SDUC-451 | Green | Taskbar-inset maximized windows are rejected while ordinary large windows remain eligible. |
| SDTEST-1526 | `companion_desktop.rs::stale_mouse_up_velocity_is_zeroed` | SDUC-451 | Green | Stale drag velocity samples are zeroed at mouse-up instead of launching the character with old motion. |
| SDTEST-1527 | `companion_desktop.rs::disabling_window_climbing_mid_fall_clears_cached_window_platforms` | SDUC-451 | Green | Turning off window climbing while already falling clears cached window platforms/windows and lands on the screen floor instead of a stale top. |
| SDTEST-1528 | `companion_desktop.rs::snapped_window_display_becomes_disappearance_fall_floor_context` | SDUC-451 | Green | Snapping to a window on another display updates the simulation display, so a later disappearance fall lands on that monitor floor instead of an old display or virtual gap. |
| SDTEST-1529 | `companion_desktop.rs::magnetic_snap_acquires_early_and_releases_only_beyond_hysteresis` | SDUC-451 | Green | Dragging near an eligible outer top acquires a visible magnetic preview before the strict release band and keeps the same stable window ID until the wider exit threshold is crossed. |
| SDTEST-1530 | `companion_desktop.rs::magnetic_snap_position_lands_on_top_and_preserves_minimum_overlap` | SDUC-451 | Green | Magnetic placement aligns the mascot bottom exactly with the window top while retaining the configured minimum horizontal overlap at either edge. |
| SDTEST-1531 | `companion_desktop.rs::active_magnetic_preview_commits_attachment_on_release` | SDUC-451 | Green | Releasing an active preview commits the same native window ID, exact perch origin, stable attachment, and follow lifecycle. |
| SDTEST-1534 | `companion_desktop.rs::drag_window_snapshots_are_throttled_to_the_refresh_interval` | SDUC-451 | Green | Full external-window enumeration obeys its low-rate refresh interval instead of running on every pointer or animation frame. |
| SDTEST-1535 | `companion_desktop.rs::runtime_floor_collision_subtracts_companion_extent_exactly_once` | SDUC-451 | Green | Runtime passes the raw work area to core physics, so the floor settles at work-area bottom minus one mascot extent rather than leaving an extra-height gap. |
| SDTEST-1536 | `companion_desktop.rs::magnetic_release_never_switches_preview_window_id` | SDUC-451 | Green | Fresh release enumeration may reorder or reveal a closer competitor, but release revalidates and commits only the stable ID shown by the preview. |
| SDTEST-1537 | `companion_desktop.rs::missing_preview_window_does_not_fallback_to_another_window` | SDUC-451 | Green | If the previewed native ID closes, minimizes, or vanishes before mouse-up, release falls normally and never silently attaches to another candidate. |
| SDTEST-1538 | `companion_desktop.rs::preview_release_and_follow_share_the_same_perch_origin` | SDUC-451 | Green | Preview, release commit, and the first follow refresh use identical top-edge geometry, including a window narrower than the mascot, preventing visible jumps. |
| SDTEST-1539 | `companion_desktop.rs::windows_snap_metrics_scale_extent_and_thresholds_for_target_display` | SDUC-451 | Green | Windows physical-desktop coordinates scale mascot extent, acquisition bands, and overlap thresholds with the candidate display's scale factor; logical-coordinate platforms remain unchanged. |
| SDTEST-1540 | `companion_desktop.rs::drag_snapshot_policy_separates_full_list_and_locked_window_refreshes` | SDUC-451 | Green | A locked preview uses targeted stable-ID geometry refreshes while the expensive full external-window list remains on its slower cadence. |
| SDTEST-1541 | `companion_desktop.rs::autonomous_climb_ignores_maximized_like_windows` | SDUC-451 | Green | Autonomous climbing applies the same eligibility filter as snapping and physics, ignoring the nearer maximized/taskbar-inset surface in favor of an ordinary unmaximized window. |
| SDTEST-1542 | `companion_desktop.rs::invalidated_preview_blocks_retargeting_for_the_rest_of_the_drag` | SDUC-451 | Green | Once a locked preview native ID disappears, the current drag cannot silently acquire or release onto a different cached window. |
| SDTEST-1543 | `companion_desktop.rs::hysteresis_preview_release_and_follow_use_the_canonical_perch_overlap` | SDUC-451 | Green | Hysteresis uses a relaxed eligibility overlap but preview placement, release, and follow retain the canonical perch overlap without a first-refresh jump. |
| SDTEST-1544 | `companion_desktop.rs::autonomous_climb_respects_single_monitor_routing` | SDUC-451 | Green | Disabling multi-monitor movement restricts autonomous climbing to eligible windows on the character's current display. |
| SDTEST-1545 | `companion_desktop.rs::autonomous_climb_ranks_vertical_gap_then_horizontal_distance_then_native_id` | SDUC-451 | Green | Autonomous targets are ranked deterministically by vertical gap, horizontal distance, and stable native ID. |
| SDTEST-1546 | `companion_desktop.rs::moved_preview_geometry_invalidates_the_drag_without_competitor_fallback` | SDUC-451 | Green | A still-visible preview that moves outside hysteresis or becomes ineligible invalidates the locked drag and cannot retarget to a competing window. |
| SDTEST-1551 | `companion_desktop.rs::physics_catchup_reports_first_landing_while_preserving_final_body_state` | SDUC-448, SDUC-451 | Green | Multi-step catch-up preserves the first landing event while returning the final sleeping body position and velocity, so the frame loop terminates correctly. |
| SDTEST-1552 | `companion_desktop.rs::duration_timing_preserves_submillisecond_remainder_and_no_first_frame_invention` | SDUC-448, SDUC-451 | Green | Fixed-step timing retains sub-millisecond remainder and contributes zero on an uninitialized first frame instead of inventing 33 ms. |
| SDTEST-1553 | `companion_desktop.rs::dynamic_physics_refresh_tracks_captured_windows_on_cadence` | SDUC-448, SDUC-449, SDUC-451 | Green | After initial discovery, dynamic falls refresh the selected stable ID at a bounded cadence, track its moved top, and remove it when closed before collision. |
| SDTEST-1554 | `companion_desktop.rs::unrelated_config_transition_preserves_attachment_but_disabling_climb_detaches_safely` | SDUC-449, SDUC-451 | Green | Unrelated settings keep an attachment; disabling climbing immediately falls under full motion or rests safely under reduced motion. |
| SDTEST-1555 | `companion_desktop.rs::invalid_attachment_geometry_enters_dynamic_fall_or_reduced_motion_rest` | SDUC-449, SDUC-451 | Green | A resized target that no longer supports the mascot uses the same safe detach lifecycle as a missing native window. |
| SDTEST-1556 | `companion_desktop.rs::release_commit_position_change_marks_pending_native_move` | SDUC-448, SDUC-451 | Green | Fresh same-ID release revalidation requests the canonical native origin when the target moved inside hysteresis after the last preview sample. |
| SDTEST-1557 | `companion_desktop.rs::stale_late_mouse_move_zeroes_throw_velocity_sample` | SDUC-451 | Green | A pointer event after the velocity sampling deadline clears the old throw vector rather than making it look fresh. |
| SDTEST-1558 | `companion_desktop.rs::pause_rest_stops_dynamic_body_and_requests_idle_visual_state` | SDUC-449, SDUC-451 | Green | Pause and motion-policy interruption clear dynamic velocity/contact and return to an event-idle visual state. |
| SDTEST-1559 | `companion_desktop.rs::active_drag_lifecycle_types_are_zero_sized_and_outside_release_is_idempotent` | SDUC-448, SDUC-451 | Green | GPUI active-drag delivery plus inside/outside mouse-up share one idempotent release lifecycle, preventing a native snap from stranding drag state. |
| SDTEST-1560 | `companion_desktop.rs::rounded_native_origin_gate_skips_subpixel_redundant_moves` | SDUC-448 | Green | Native placement is issued only when the rounded platform origin changes, suppressing redundant subpixel movement calls. |
| SDTEST-1561 | `companion_desktop.rs::system_motion_preference_honors_platform_reduced_motion` | SDUC-449, SDUC-451 | Green | System motion follows the OS reduced-motion preference, while explicit Full remains an intentional override. |
| SDTEST-1562 | `companion_desktop.rs::idle_flourish_duty_cycle_stays_below_the_playful_background_budget` | SDUC-448, SDUC-451 | Green | Idle flourishes remain occasional and below one fifth of their waiting interval, reducing background GPU/CPU activity without removing personality. |
| SDTEST-1570 | `settings::tests::native_wayland_companion_limitation_is_reported_without_misclassifying_x11` | SDUC-449, SDUC-450 | Green | Appearance uses the same GPUI compositor detection as the runtime, shows a localized native-Wayland limitation, and does not misclassify X11. |
| SDTEST-1571 | `companion_desktop.rs::gravity_fall_discovers_window_platforms_after_starting_with_an_empty_snapshot` | SDUC-448, SDUC-451 | Green | Gravity that starts without cached geometry discovers current visible window tops, lands at the companion extent above the chrome, and maps the contact back to the stable window ID. |
| SDTEST-1572 | `companion_desktop.rs::dynamic_physics_refresh_switches_to_stable_id_updates_after_initial_scan` | SDUC-448, SDUC-449, SDUC-451 | Green | A fall performs one full visible-window discovery, then updates only the trajectory-selected captured stable ID without admitting newly appeared windows or repeating an expensive full X11 scan. |
| SDTEST-1573 | `adabraka_gpui::platform::linux::x11::client::companion_external_window_tests::x11_frame_extents_require_cardinal_32_and_bounded_exact_values` | SDUC-448, SDUC-449, SDUC-451 | Yellow | The vendored-fork parser test accepts only exact CARDINAL/32 four-value `_NET_FRAME_EXTENTS` payloads inside a defensive bound. It passes in the isolated patch harness but is not yet executed by the root workspace test command. |
| SDTEST-1574 | `companion_desktop.rs::dynamic_physics_refresh_budget_prefers_a_reachable_diagonal_collision` | SDUC-448, SDUC-449, SDUC-451 | Green | Each 100 ms physics refresh simulates the next fixed-step trajectory and targets at most the first reachable captured window, including fast diagonal motion without current horizontal overlap. |
| SDTEST-1575 | `companion_desktop.rs::targeted_refresh_does_not_promote_an_unvalidated_cached_fallback` | SDUC-448, SDUC-449, SDUC-451 | Green | If the one revalidated target closes or disappears, no older unrefreshed cached window is promoted into the active collision set and no phantom landing can occur. |
| SDTEST-1576 | `companion_desktop.rs::trajectory_refresh_orders_collisions_within_the_same_fixed_step` | SDUC-448, SDUC-451 | Green | Two reachable tops crossed in one fixed step are ordered by exact projected vertical time of impact, not current overlap or coarse step number. |
| SDTEST-1577 | `companion_desktop.rs::drag_release_predicts_first_interval_platforms_from_release_velocity` | SDUC-448, SDUC-451 | Green | The production drag-release path enters Dynamic mode with the bounded throw velocity before selecting its first 100 ms collision platform. |
| SDTEST-1594 | `main.rs::x11_pointer_coordinates_are_not_offset_twice` | SDUC-448, SDUC-449 | Green | Back-labelled 2026-08-06. X11 pointer math applies the display origin exactly once. The coordinate-space decision feeding it now comes from `companion_desktop::is_x11_session` (`gpui::guess_compositor()`), never `XDG_SESSION_TYPE` — fixing the `XDG_SESSION_TYPE=x11` + `WAYLAND_DISPLAY` disagreement. |
| SDTEST-1595 | `main.rs::xrandr_geometry_preserves_each_monitor_origin` | SDUC-448, SDUC-449 | Green | Back-labelled 2026-08-06. `xrandr`-parsed monitor bounds keep every monitor's true origin. `xrandr`/`xprop` are optional runtime soft deps: spawn errors, non-zero exits and unparseable output warn and fall back to GPUI display bounds. |
| SDTEST-1596 | `main.rs::x11_workarea_excludes_the_system_toolbar` | SDUC-448, SDUC-449 | Green | Back-labelled 2026-08-06. `_NET_WORKAREA` (via `xprop`) excludes panels/taskbars from the roaming area; absence degrades to full display bounds. |
| SDTEST-1597 | SDPATCH-113 X11 external-window filter — compile-check only (`cargo check -p adabraka-gpui`) | SDUC-449 | Yellow | The EWMH dock/menu/toolbar/tooltip/popup-menu/dropdown-menu/splash/notification/utility + `WM_TRANSIENT_FOR` exclusions in the vendored fork have no runnable test: a live X server would be required, and the fork's own tests are not executed by the workspace test command (same limitation as SDTEST-1573). |
| SDTEST-1599 | `support_view::requests::tests::thread_refresh_preserves_reading_position_but_not_new_or_bottom_threads` | SDUC-459 | Green | Poll-driven rebuilds restore an active virtual-list offset; a newly selected thread and a reader already at the bottom retain bottom alignment. |

### `shelldeck-ui/server_sync_view.rs` — file-browser breadcrumbs

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1589 | `server_sync_view.rs::breadcrumb_segments_remote_is_unix_regardless_of_host` + `server_sync_view.rs::breadcrumb_segments_local_matches_remote_shape_on_unix` | SDUC-457 | Green | 2 tests, added 2026-08-06. Remote panes keep pure Unix string math on every host platform (a Windows host never receives backslash paths from a Linux server); the local pane's `std::path`-components breadcrumbs are byte-identical to the remote flavor on Unix, while Windows locals get `C:\` round-trip and a `..` row that disappears at filesystem roots. |

### Application chrome (menu bar, sidebar rail, scaling)

| ID | Test | Use case | Status | Notes |
|---|---|---|---|---|
| SDTEST-1200 | `menu_bar::tests::logged_out_bar_exposes_no_session_commands` | SDUC-442 | Green | Logged out the bar offers sign-in and quit only; every session-dependent command and the whole Go menu are absent, matching the no-guest-mode rule in `.agents/roles.md`. |
| SDTEST-1201 | `menu_bar::tests::user_mode_hides_every_dev_only_command` | SDUC-442 | Green | User mode keeps requests but drops quick-connect, terminals, scripts, editor, sidebar toggle, splits and terminal zoom, and omits the Terminal menu entirely. |
| SDTEST-1202 | `menu_bar::tests::staff_consoles_follow_availability_flags` | SDUC-442 | Green | Monique and Fleet appear only when configured, so a super-admin without a Monique config gets no dead entry. bext Cloud is capability-gated only. |
| SDTEST-1203 | `menu_bar::tests::view_toggles_reflect_current_state` | SDUC-442 | Green | The sidebar and menu-bar checkmarks report live state rather than a constant, so the tick never lies about what is on screen. |
| SDTEST-1204 | `menu_bar::tests::entry_ids_are_unique_across_the_whole_bar` | SDUC-442 | Green | Entry ids become GPUI `ElementId`s; duplicates would make two rows share hover/click state. Checked across all four mode/sign-in combinations with every optional menu enabled. |
| SDTEST-1205 | `menu_bar::tests::accel_renders_platform_modifiers` | SDUC-442 | Green | Shortcut hints resolve `secondary` to Cmd on macOS and Ctrl elsewhere, from the same vocabulary `actions.rs` binds with. |
| SDTEST-1210 | `sidebar::tests::total_width_is_rail_plus_panel` | SDUC-443 | Green | The rail and the panel each contribute exactly their own width. A collapsed panel must still reserve the rail, or the terminal grid is sized underneath it. |
| SDTEST-1211 | `sidebar::tests::collapsed_panel_width_is_ignored_at_any_size` | SDUC-443 | Green | A collapsed panel leaks no width back in at either end of the 180–400px resize clamp. |
| SDTEST-1213 | `sidebar::tests::activity_without_a_panel_contributes_no_panel_width` | SDUC-443 | Green | An activity with no contextual rows hides the panel while not collapsed; counting the panel there would offset every terminal past a column that is not on screen. The two reasons to hide must not compound. |
| SDTEST-1216 | `sidebar::tests::total_width_always_reserves_the_rail` | SDUC-443 | Green | The rail is unconditional across every panel-collapse / panel-less combination. Guards the v0.6.3 retirement of the hide-nav toggle, whose "hidden" state swapped in a second navigation UI that had already drifted out of sync with the rail. |
| SDTEST-1214 | `sidebar::tests::rail_lists_activities_not_destinations` | SDUC-443 | Green | Monique, Fleet, bext Cloud and Settings never take a rail slot, and every rail entry either has a panel or is a spelled-out main-view entry (Server Sync or Agents) — the guard against re-adding a rail icon with nothing behind it. |
| SDTEST-1215 | *to write* — panel content follows the selected activity | SDUC-443 | **Red / P1** | GPUI integration: selecting each rail activity swaps the panel to that activity's rows, a row click performs its open/focus action, empty activities show their localized hint, and a panel-less activity collapses the panel. Regression guard for the 2026-07-25 mislabelled-panel defect. |
| SDTEST-1676 | `workspace::agents::tests::sdtest_1676_local_runner_streams_and_stops_the_process_group` (`#[cfg(unix)]`) | SDUC-475 | Green | A fake streaming agent crosses the real local runner, then Stop kills its shell plus sleeping child and returns promptly instead of hanging on inherited output pipes. |
| SDTEST-1677 | *to write* — local and SSH agent console end-to-end smoke | SDUC-475 | **Red / P1** | GPUI + fake SSH harness: select each provider and target, stream remote output, close the remote channel, cover Windows process-tree Stop, and prove a removed connection cannot launch. The release-critical access gate is covered separately by SDTEST-1679 without requiring the missing GPUI harness. |
| SDTEST-1678 | `agent_console_view::tests::sdtest_1678_session_resume_requires_the_exact_execution_context` | SDUC-475 | Green | A provider session token is reused only when provider, target, access, workdir, and model still match; changing permissions or hosts necessarily starts a new session. |
| SDTEST-1679 | `agent_console_view::tests::sdtest_1679_mutating_access_always_requires_confirmation` | SDUC-475 | Green | Read-only runs may start directly; workspace-write and full-access runs must stop at the separate confirmation disposition before Workspace receives a launch event. |
| SDTEST-1681 | `workspace::agents::tests::sdtest_1681_remote_stream_reassembles_split_utf8_and_json_lines` | SDUC-475 | Green | SSH packet boundaries may split a multibyte glyph or JSON line; the byte accumulator reconstructs the complete UTF-8 line before provider parsing. |
| SDTEST-1212 | *to write* — Workspace surfaces re-layout at non-default App Font Size | SDUC-441 | **Red / P2** | GPUI integration: at 10px and 22px the User home, welcome screen and account/site/mode titlebar dropdowns scale proportionally while the client inset, shadow geometry and window-resize border stay in device pixels. Needs a `TestAppContext` harness we do not have yet. |

---

## 8e. `shelldeck-ui/settings.rs` — interface typography

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1653 | `settings.rs::the_ui_font_shortlist_offers_no_monospace_family` | SDUC-467 | Green | The interface shortlist and the terminal/editor shortlist stay disjoint, and Inter leads the interface list. |
| SDTEST-1654 | `settings.rs::monospace_and_the_legacy_sentinel_never_survive_as_interface_families` | SDUC-467 | Green | The branches of `normalize_ui_font_family` that need no `TextSystem`: monospace families and the retired sentinel are both refused as interface families. |
| SDTEST-1656 | `i18n.rs::assert_portal_failures_stay_readable` (appelé par `locale_fr_and_en`) | SDUC-468 | Green | In both locales, a portal failure never shows the internal URL, reqwest's wording or an HTTP code, and "unreachable" and "session expired" do not collapse into one message. Folded into the single locale test because `set_locale` is process-global. |

---

## 8f. `patches/adabraka-ui` — coloured text runs

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1658 | `input_state.rs::runs_tile_the_text_exactly_once` | SDUC-468 | Green | The split runs cover the text exactly once and only the highlighted one carries the colour and its background. |
| SDTEST-1659 | `input_state.rs::invalid_ranges_are_dropped_rather_than_clamped` | SDUC-468 | Green | Out-of-bounds, inverted, empty and mid-character ranges are dropped; the run lengths still sum to the text. |
| SDTEST-1660 | `input_state.rs::overlapping_ranges_keep_the_first_and_never_double_count` | SDUC-468 | Green | Overlapping highlights cannot make the run lengths exceed the text, which would panic the shaper. |
| SDTEST-1661 | `input_state.rs::no_highlight_yields_the_untouched_run` | SDUC-468 | Green | The no-highlight path is the untouched single run. |

---

## 9. Cross-platform coverage (referenced from everywhere)

CI matrix already runs `cargo check` on all three targets. The SDTEST
entries that carry cross-platform stakes and must run on multiple
targets (not just Linux) are cross-linked here for the release
checklist:

- SDTEST-121, SDTEST-122 (keychain macOS/Windows)
- SDTEST-960..968 (PTY spawn on all three)
- SDTEST-1579..1581 (shell resolution — pure fn asserts both platform
  branches from any target, no cfg gate)
- SDTEST-1201, SDTEST-1202 (platform key mapping)
- SDTEST-1242, SDTEST-1243 (installer replace on Unix / Windows)
- SDTEST-1260, SDTEST-1261 (install-script + manifest parity)
- SDTEST-1483 (desktop overlay native movement, work-area, reduced-motion, and no-focus platform APIs)
- SDTEST-1584 (`#[cfg(windows)]` Jcode executor spawn — **no CI target
  compiles or runs it today**; needs a windows-latest test or
  `cargo check --tests --target x86_64-pc-windows-msvc` job)

The release-day rule: **all P0 cross-platform tests must be green on
the matching CI runner before the tag goes out.** This maps directly
to AGENTS.md's `cross-platform.md` mandate that "if any of the three
builds fails, the release + manifest jobs are skipped entirely".

---

## Retired tests

- **SDTEST-1392 / SDTEST-1395** (2026-08-13) — hidden-start Dock runtime
  smokes whose core assertion required `Workspace` to remain absent. The Dock
  now deliberately initializes Workspace to validate the authoritative
  session before exposing account-bound AI. Their visual/idempotency portions
  remain covered separately; a replacement signed-in runtime smoke is needed.
- **SDTEST-1333 / SDTEST-1334** (2026-08-06) — `terminal_view.rs` command
  discovery tests. `command_available` now delegates to
  `shelldeck_core::util::executable_on_path`, whose contracts (multi-dir PATH
  walk, PATHEXT extensions, unix `+x` check — a stricter superset) are pinned
  by SDTEST-1591 in `tests-core.md`. IDs stay reserved per the sticky-ID rule.
