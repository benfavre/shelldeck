# SDTEST inventory — `shelldeck-core`

> Rules for this file live in [`.agents/testing.md`](../../.agents/testing.md).
> Use case IDs (`SDUC-…`) resolve in [`USE_CASES.md`](./USE_CASES.md).
>
> Status: **Green** exists & passes · **Yellow** exists but weak/flaky ·
> **Red** to write (priority P0/P1/P2) · **Retired** removed on purpose.

Convention for the *Location* column: `<file>::<fn>`. For Green
entries, `git grep <fn>` lands on the code.

---

## 1. `util.rs` — atomic write + cross-platform env helpers

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-001 | `util.rs::atomic_write_creates_new_file` | SDUC-091 | Green | |
| SDTEST-002 | `util.rs::atomic_write_overwrites_existing_file` | SDUC-091 | Green | |
| SDTEST-003 | `util.rs::atomic_write_leaves_no_tmp_files` | SDUC-091 | Green | |
| SDTEST-004 | *to write* — atomic_write preserves prior file when write fails mid-way | SDUC-091 | **Red / P1** | Simulate a fake writer that Errs after N bytes; assert the target path is either the *prior* content or absent, never partial. |
| SDTEST-005 | *to write* — atomic_write fsync semantics on Windows | SDUC-091 | **Red / P2** | Windows rename semantics are different; add a Windows-gated regression once the pattern hits a real bug. |
| SDTEST-1590 | `util.rs::{home_dir_prefers_home_then_userprofile_and_rejects_blanks, username_prefers_user_then_logname_then_username, hostname_env_prefers_hostname_then_computername_and_trims, hostname_is_never_empty}` | SDUC-330 | Green | 4 tests, added 2026-08-06. Injectable env-precedence contracts behind `home_dir()` / `current_username()` / `hostname()` — the helpers the Windows-portability wave rewired PTY cwd, SSH known_hosts/key discovery, fleet workdir, and cloud device names onto. Blank values are rejected; the hostname terminal fallback is `"ShellDeck"`. |
| SDTEST-1591 | `util.rs::{path_extensions_windows_parses_pathext_and_defaults, executable_lookup_searches_all_path_dirs_with_extensions, executable_lookup_rejects_directories_and_non_executables}` | SDUC-330, SDUC-413 | Green | 3 tests. PATHEXT-aware `executable_on_path_in`: every `PATH` entry searched, Windows extension candidates honored, directories and non-executables rejected. `ai::command_available` now delegates here — its coverage moved with the logic. (Behavior note: the old Windows path also probed the bare extension-less name; PATHEXT-only is correct cmd.exe semantics and no caller passes a name with its own extension.) |

---

## 2. `models/discovery.rs` — remote inventory parsers

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-010 | `discovery.rs::test_parse_stat_output` | SDUC-070 | Green | |
| SDTEST-011 | `discovery.rs::test_parse_ls_output` | SDUC-071 | Green | |
| SDTEST-012 | `discovery.rs::test_parse_nginx_configs` | SDUC-072 | Green | |
| SDTEST-013 | `discovery.rs::test_parse_mysql_discovery` | SDUC-073 | Green | |
| SDTEST-014 | `discovery.rs::test_parse_pg_discovery` | SDUC-074 | Green | |
| SDTEST-015 | `discovery.rs::test_rsync_command` | SDUC-075 | Green | |
| SDTEST-016 | `discovery.rs::parse_ls_output_handles_spaces_in_names_and_dotfiles` + `parse_ls_output_skips_malformed_lines` | SDUC-071 | Green | 2 tests, added 2026-07-09. Filenames with spaces re-joined intact via `parts[7..].join(" ")`, dotfiles kept, ragged lines silently skipped (never panics). |
| SDTEST-017 | `discovery.rs::parse_nginx_configs_tolerates_include_directive` | SDUC-072 | Green | Added 2026-07-09. Real `include` expansion is the shell command's job; the parser just tolerates the directive without emitting a bogus site. |
| SDTEST-018 | `discovery.rs::parse_nginx_configs_takes_first_server_name_when_multiple_listed` | SDUC-072 | Green | Added 2026-07-09. **Pins current limitation** — the parser calls `split_whitespace().next()`, so only the first host wins. Future TODO is to emit all names; this test locks the shape so a well-meaning refactor doesn't regress to picking the last. |
| SDTEST-019 | `server_sync.rs::percent_is_none_when_total_unknown` + `percent_zero_total_returns_100` + `percent_clamps_to_100_even_if_transferred_exceeds_total` + `percent_normal_case` + `overall_percent_is_size_weighted_not_count_weighted` + `overall_percent_empty_operation_is_none` + `overall_percent_none_when_no_item_knows_its_total` | SDUC-076 | Green | 7 tests, added 2026-07-09. **Contract correction** — `percent()` is a percentage (0..=100), not a ratio (0..=1). Size-weighting test uses a 1 GB@50% + 10× 1 KB@100% fixture: naive count-weighting would report ~95%, correct size-weighting reports ~50%. |
| SDTEST-020 | `discovery.rs::rsync_command_includes_delete_and_ignore_existing_switches` + `rsync_command_shell_escapes_source_and_dest_paths` + `rsync_command_emits_one_exclude_per_pattern` | SDUC-075 | Green | 3 tests, added 2026-07-09. Extends the existing `test_rsync_command` (SDTEST-015) with the untouched switches (`delete_extra`, `skip_existing`), verifies `shell_escape` wraps paths containing spaces, and asserts one `--exclude=` emitted per pattern. |
| SDTEST-1585 | `discovery.rs::join_child_path_matches_native_expectations` | SDUC-457 | Green | Added 2026-08-06, made native-runner-aware 2026-08-18. Unix pins root/trailing-slash behavior; Windows CI pins drive letters and backslashes through `std::path` instead of asserting Unix output on a Windows host. |
| SDTEST-1586 | `discovery.rs::format_readonly_permissions_never_fabricates_mode_bits` | SDUC-457 | Green | Added 2026-08-06. The non-Unix local listing derives `drw`/`dr-`/`-rw`/`-r-` from the readonly flag only — never the fabricated `drwxr-xr-x` it used to report. |
| SDTEST-1587 | `discovery.rs::read_local_nginx_configs_feeds_parse_nginx_configs` | SDUC-457, SDUC-072 | Green | Added 2026-08-06. Local vhost files read via `std::fs` produce the same `---FILE:` wire format the SSH command emits (symlinks followed, dotfiles skipped, name order) and flow through the existing nginx parser unchanged. |
| SDTEST-1588 | `discovery.rs::discover_argv_forms_match_shell_commands` | SDUC-457, SDUC-073, SDUC-074 | Green | Added 2026-08-06. `mysql_discover_argv`/`pg_discover_argv` carry the same SQL as the remote shell command strings (shared consts; real tab instead of the `$'\t'` bash-ism, no empty arg on empty credentials) — and the remote shell strings themselves are pinned byte-identical to HEAD. |

---

## 3. `models/{connection,port_forward,script,script_runner,execution,templates,managed_site}.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-030 | `port_forward.rs::zero_is_rejected` + `all_non_zero_ports_are_accepted` | SDUC-313 | Green | 2 tests, added 2026-07-09. Boundary sweep covers 1 / 22 / 1023 / 1024 / 65535. Regression sensor if someone re-adds a `< 1024` privileged-port restriction. |
| SDTEST-031 | *to write* — port forward presets produce valid PortForward objects | SDUC-049 | **Red / P2** | `chrome_devtools_preset`, `web_server_preset`, `opencode_preset`, `dev_server_preset`. |
| SDTEST-032 | `connection.rs::display_name_prefers_alias_falls_back_to_hostname` + `display_name_returns_borrowed_slice` + `new_manual_sets_manual_source_and_default_port` | SDUC-104bis | Green | 3 tests, added 2026-07-09. **Contract correction** — fallback is alias → hostname only, NO UUID fallback (my initial inventory was wrong). Bonus test proves no allocation on paint (`ptr::eq` on the borrowed slice). |
| SDTEST-033 | `connection.rs::connection_string_always_includes_port` | SDUC-104bis | Green | Added 2026-07-09. Port is always in the output, even when it's the default 22 (opinionated contract). |
| SDTEST-034 | `script.rs::extracts_bare_names_dedup_preserves_first_occurrence` + `extracts_defaults_after_colon` + `trims_inner_whitespace_and_ignores_empty` + `same_name_second_occurrence_ignored_even_with_default` + `unclosed_placeholder_is_silently_dropped` | SDUC-060 | Green | 5 tests, added 2026-07-09. Split-on-first-`:` (colon in default preserved), first-occurrence wins on dedup, unclosed `{{…` tolerated. |
| SDTEST-035 | `script.rs::extracts_placeholders_even_inside_code_fences` | SDUC-060 | Green | Added 2026-07-09. **Pins current limitation, not the ideal behavior** — the parser does NOT skip triple-backtick fences today, so `{{ansible_var}}` inside a YAML block is still extracted. Test locks the shape so a future fence-aware refactor is a deliberate contract change. Original inventory called this a P1 gap; keeping it as a locked-in reality until someone implements the fence skip. |
| SDTEST-036 | `script_runner.rs::provided_value_replaces_placeholder` + `missing_value_falls_back_to_inline_default` + `missing_value_without_default_leaves_placeholder` + `extra_values_in_map_are_ignored` + `substitution_is_utf8_safe` + `unclosed_placeholder_does_not_panic` | SDUC-061 | Green | 6 tests, added 2026-07-09. Key contract: **no value + no default → placeholder LEFT UNCHANGED**, not empty. Downstream re-prompt logic depends on this. |
| SDTEST-037 | `script.rs::every_builtin_language_has_a_runnable_spec` + `file_based_languages_declare_an_extension` + `each_builtin_has_a_unique_runner_binary_or_args` | SDUC-062 | Green | 3 tests, added 2026-07-09. Table-driven over `ScriptLanguage::ALL` — adding a new variant without wiring `runner_spec` trips the test. Separates file-based (Shell/Python/Node/Bun/Php/Mysql/Postgresql — non-empty `file_ext`) from subcommand-style (Docker/Compose/Systemd/Nginx — empty `file_ext`, uses `{body_as_args}`). |
| SDTEST-038 | `script_runner.rs::detect_command_probes_every_supported_package_manager` + `detect_command_runs_on_local_shell` (`#[cfg(unix)]`) | SDUC-063 | Green | 2 tests, added 2026-07-09. Shape check probes for `echo 'apt'`/`dnf`/`yum`/`pacman`/`brew`/`apk` + `unknown` fallback; integration test runs the command through `sh -c` on Unix and asserts stdout is one of the recognized labels. Windows-skipped because the detect script is POSIX-only. |
| SDTEST-039 | `script_runner.rs::build_dependency_check_command_shapes` | SDUC-064 | Green | Added 2026-07-09. Empty input → sentinel `"No dependencies to check"`; N deps → N `if…else…fi` guarded probes joined by `&&`. |
| SDTEST-040 | `script_runner.rs::get_install_command_per_package_manager` | SDUC-064 | Green | Added 2026-07-09. Table-driven per PM: matching PM → its command; valid PM without an `InstallCommand` for this dep → None; unknown PM string → None. Guards against a typo like `"choco"` silently returning a valid command from another PM. |
| SDTEST-041 | *to write* — Script::builtin_* round-trip through serde | SDUC-065 | **Red / P2** | |
| SDTEST-042 | `templates.rs::all_templates_have_unique_ids` + `all_templates_have_non_empty_body_and_name` + `all_templates_ids_are_kebab_and_prefixed` | SDUC-066 | Green | 3 tests, added 2026-07-09. Sweep across the shipped catalog: no duplicate IDs, non-empty name/body/description, IDs are kebab-case ASCII with a `<category>-<slug>` prefix. Grows for free with every new template. |
| SDTEST-043 | `templates.rs::to_script_carries_template_metadata` | SDUC-066 | Green | Added 2026-07-09. Finds a template that exercises both dependencies AND variables (there's at least one), materializes it, asserts template_id link + body/language/category/deps/vars preserved and `is_template=false`. |
| SDTEST-044 | `execution.rs::new_starts_in_running_state` + `append_output_accumulates` + `finish_with_zero_marks_succeeded_and_produces_duration` + `finish_with_non_zero_marks_failure` + `connection_id_is_preserved` | SDUC-067 | Green | 5 tests, added 2026-07-09. Full lifecycle sweep: `is_running` / `succeeded` / `duration_secs` transitions, non-zero exit codes (including negative like `-1` and 127), local vs remote (`connection_id`) round-trip. 5ms sleep in the finish test to make duration observable at ms precision. |
| SDTEST-045 | `managed_site.rs::from_nginx_preserves_server_name_port_and_ssl` | SDUC-072 | Green | Added 2026-07-09 (cluster M). |
| SDTEST-046 | `managed_site.rs::url_elides_default_ports_and_keeps_custom_ones` | SDUC-072 | Green | Added 2026-07-09 (cluster M). `url()` elides port for scheme defaults (443 https / 80 http), keeps for custom (8080, 8443). |
| SDTEST-047 | `managed_site.rs::from_database_preserves_engine_and_reports_no_url` | SDUC-073 | Green | Added 2026-07-09 (cluster M). Engine (PostgreSQL/MySQL) preserved, `url()`/`port()` return None (no HTTP surface on databases). |

---

## 4. `config/app_config.rs` — `shelldeck.toml`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-060 | `app_config.rs::round_trip_non_default` | SDUC-080 | Green | |
| SDTEST-061 | `app_config.rs::cloud_sync_round_trips` | SDUC-080 | Green | |
| SDTEST-062 | `app_config.rs::account_round_trips_and_omits_when_logged_out` | SDUC-082 | Green | |
| SDTEST-063 | `app_config.rs::jeanclaude_override_round_trips_and_omits_when_unset` | SDUC-083 | Green | |
| SDTEST-064 | `app_config.rs::jean_runtime_round_trips_and_defaults_off` | SDUC-084 | Green | |
| SDTEST-065 | `app_config.rs::config_without_cloud_sync_section_still_parses` | SDUC-081 | Green | |
| SDTEST-066 | `app_config.rs::load_from_missing_creates_defaults` | SDUC-085 | Green | |
| SDTEST-067 | `app_config.rs::load_from_corrupt_returns_err` | SDUC-086 | Green | |
| SDTEST-068 | *to write* — config with unknown fields still loads (forward compat) | SDUC-081 | **Red / P1** | Server may add a `[foo]` we don't know about yet; must not Err. |
| SDTEST-069 | `app_config.rs::default_matches_documented_first_run_values` | SDUC-093 | Green | Added 2026-07-09. Pins every default: Dark theme, JetBrains Mono 14pt, 10 000-line scrollback, block cursor with blink, sidebar 260px, notifications on, confirm-close on, auto-update on, `ui_language = System`. All session flags OFF (account None, cloud_sync/jean_runtime/bext_cloud all disabled). Sensor for silent drift on any first-run field. |
| SDTEST-070 | `app_config.rs::save_to_replaces_config_file_atomically` | SDUC-091 | Green | A hard link preserves the prior file identity and contents while `save_to` atomically replaces the configured path. |
| SDTEST-071 | *to write* — ConfigWatcher fires the callback on external edit (debounced) | SDUC-090 | **Red / P1** | Use a `TempDir` + `std::fs::write` twice within the debounce window. |
| SDTEST-1335 | `app_config.rs::older_config_defaults_pinned_connections_to_empty` + `round_trip_non_default` | SDUC-411 | Green | Pins backward compatibility plus UUID/order persistence for quick favorites. |
| SDTEST-1382 | `app_config.rs::config_without_companion_section_defaults_to_visible_start` | SDUC-435 | Green | Old configs remain visible by default; an explicit `[companion] start_hidden = true` round-trips through serde. |
| SDTEST-1400 | `app_config.rs::companion_shortcuts_default_for_old_configs_and_round_trip_custom_values` | SDUC-434 | Green | Older `[companion]` sections receive platform-specific Dock/palette defaults; custom GPUI keystroke strings survive TOML serialization. |

---

## 5. `config/store.rs` — connection store

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-080 | `store.rs::round_trip_with_data` | SDUC-087 | Green | |
| SDTEST-081 | `store.rs::load_from_missing_creates_empty` | SDUC-088 | Green | |
| SDTEST-082 | `store.rs::load_from_corrupt_returns_err` | SDUC-088 | Green | |
| SDTEST-083 | `store.rs::save_to_replaces_connection_store_atomically` | SDUC-091 | Green | The linked prior store remains empty while the replaced path contains the new connection. |
| SDTEST-084 | `store.rs::round_trip_preserves_manual_ssh_config_and_cloud_sync_sources` | SDUC-087 | Green | Added 2026-07-09 (cluster M). Regression sensor for cloud_sync merge (SDUC-104): 3 connections (one per source) survive save/load with sources + tags preserved. |

---

## 6. `config/workspace_state.rs` — restored tabs

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-090 | `workspace_state.rs::round_trip_with_tabs` | SDUC-089 | Green | |
| SDTEST-091 | `workspace_state.rs::load_from_missing_returns_default` | SDUC-089 | Green | |
| SDTEST-092 | `workspace_state.rs::clear_at_removes_file` | SDUC-089 | Green | |
| SDTEST-093 | `workspace_state.rs::load_from_corrupt_returns_err` | SDUC-089 | Green | |

---

## 6a. `config/activity.rs` — recent activity log

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1330 | `activity.rs::append_to_and_load_recent_return_newest_first_with_limit` | SDUC-408 | Green | Appends JSONL entries, then loads newest-first with a limit. |
| SDTEST-1331 | `activity.rs::load_recent_ignores_blank_and_malformed_lines` | SDUC-408 | Green | One corrupt line cannot brick startup. |
| SDTEST-1332 | `activity.rs::old_entries_without_action_default_to_none` | SDUC-408 | Green | Back-compat for entries written before route actions existed. |

---

## 7. `config/ssh_config.rs` — parser

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-100 | `ssh_config.rs::test_is_wildcard_only` | SDUC-040 | Green | |
| SDTEST-101 | `ssh_config.rs::test_parse_host_port` | SDUC-040 | Green | |
| SDTEST-102 | `ssh_config.rs::test_strip_keyword` | SDUC-040 | Green | |
| SDTEST-103 | `ssh_config.rs::test_parse_forward_directive` | SDUC-040 | Green | |
| SDTEST-104 | `ssh_config.rs::test_parse_extra_fields` | SDUC-040 | Green | |
| SDTEST-105 | `ssh_config.rs::test_expand_tilde` | SDUC-040 | Green | |
| SDTEST-106 | `ssh_config.rs::include_directive_does_not_break_parse` | SDUC-040 | Green | Added 2026-07-09 (cluster M). Common shape `Include ~/.ssh/conf.d/*` is tolerated (`ALLOW_UNKNOWN_FIELDS`) — top-level hosts still extracted even if the underlying `ssh2_config` crate doesn't expand the Include itself. |
| SDTEST-107 | *to write* — wildcard `Host *` fields apply as defaults to specific hosts | SDUC-040 | **Red / P1** | Handled by the `ssh2_config` crate; needs a functional smoke test to lock the merge behaviour. |
| SDTEST-108 | `ssh_config.rs::parse_never_mutates_the_input_file` | SDUC-040 | Green | Added 2026-07-09 (cluster M). AGENTS.md "Critical Rules" guarantee: mtime + size + content unchanged after parse. Uses `TempDir` + `std::fs::metadata` sensor. |

---

## 8. `config/keychain.rs` — OS keychain wrapper

Existing: **0 tests**.

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-120 | `keychain.rs::live_password_round_trip` (`#[ignore]`, `SHELLDECK_LIVE_KEYCHAIN=1`) | SDUC-042 | Green | Added 2026-07-09 (cluster L). **This test was failing the whole time and nobody knew**: `keyring = "3"` shipped with no backend feature, so every store went to the in-process mock. Being `#[ignore]`d, it never ran. Green since the platform features were enabled; CI now runs it on macOS and Windows. |
| SDTEST-121 | `keychain.rs::live_*` on the `macos-aarch64` runner (`Native keychain round-trip (macOS)` step) | SDUC-042, SDUC-334 | Green | The step creates a throwaway keychain, makes it default, runs the round-trips and deletes it — the runner's login keychain is never touched and nothing outlives the job. |
| SDTEST-122 | `keychain.rs::live_*` on the `windows-x86_64` runner (`Native credential round-trip (Windows)` step) | SDUC-042, SDUC-334 | Green | Credential Manager is per-user and the runner user is disposable, so no isolation dance is needed. Entries still carry a pid+nanos host name and are deleted. |
| SDTEST-123 | `keychain.rs::live_get_password_none_for_missing_entry` + `live_delete_password_missing_entry_is_ok` (`#[ignore]`) | SDUC-042 | Green | 2 tests, added 2026-07-09 (cluster L). Ok(None) distinction pinned; delete on missing = Ok(()) for idempotent logout. These two passed even against the mock — only the round-trip could expose a store that never stores. |
| SDTEST-124 | `keychain.rs::password_and_passphrase_key_namespaces_do_not_collide` + `entry_key_is_user_at_host` + `passphrase_key_carries_prefix_and_path` | SDUC-042 | Green | 3 tests, added 2026-07-09 (cluster L). Pure fns — no OS keychain. Hostile fixture: SSH key path spelling out `user@host.example` proves the `passphrase:` prefix is load-bearing. |

---

## 9. `config/themes.rs` — builtin themes

Existing: **0 tests**.

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-130 | `themes.rs::builtins_returns_the_four_shipped_themes` + `by_name_returns_the_matching_builtin` | SDUC-092 | Green | 2 tests, added 2026-07-09 (cluster M). Catalog is Dark/Light/Pastel Dark/High Contrast. |
| SDTEST-131 | `themes.rs::by_name_unknown_falls_back_to_dark_no_panic` | SDUC-092 | Green | Added 2026-07-09 (cluster M). Load-bearing safety — a stale theme name (renamed upstream, corrupt config) falls back to Dark instead of crashing at boot. Empty string also covered. |
| SDTEST-132 | `themes.rs::every_builtin_has_name_bg_and_fg` | SDUC-092, SDUC-025 | Green | Added 2026-07-09 (cluster M). Cheap invariant — accidental `""` field in a refactor would trip. |

---

## 10. `config/cloud_sync.rs` — Manage sync + merge

| ID         | Location                                                                                 | SDUC     | Status       | Notes                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ---------- | ---------------------------------------------------------------------------------------- | -------- | ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| SDTEST-140 | `cloud_sync.rs::merge_adds_new_profiles`                                                 | SDUC-101 | Green        |                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| SDTEST-141 | `cloud_sync.rs::merge_copies_site_binding`                                               | SDUC-106 | Green        |                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| SDTEST-142 | `cloud_sync.rs::merge_updates_existing_and_preserves_local_only_fields`                  | SDUC-102 | Green        |                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| SDTEST-143 | `cloud_sync.rs::merge_removes_vanished_cloud_profiles`                                   | SDUC-103 | Green        |                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| SDTEST-144 | `cloud_sync.rs::merge_never_touches_manual_or_ssh_config`                                | SDUC-104 | Green        |                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| SDTEST-145 | `cloud_sync.rs::merge_skips_unparseable_ids`                                             | SDUC-105 | Green        |                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| SDTEST-146 | `cloud_sync.rs::cloud_sync_config_parses_without_active_site_fields`                     | SDUC-108 | Green        |                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| SDTEST-147 | `cloud_sync.rs::is_configured_semantics`                                                 | SDUC-109 | Green        |                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| SDTEST-148 | `cloud_sync.rs::remote_profile_parses_nulls_and_missing_fields`                          | SDUC-110 | Green        |                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| SDTEST-149 | `cloud_sync.rs::sync_payload_parses_contract_example`                                    | SDUC-111 | Green        |                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| SDTEST-150 | `cloud_sync.rs::merge_reports_no_change_when_nothing_moves`                              | SDUC-107 | Green        |                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| SDTEST-151 | `cloud_sync.rs::live_fetch_sync` (`#[ignore]`)                                           | SDUC-112 | Yellow       | Live smoke — gated by env token. Keep.                                                                                                                                                                                                                                                                                                                                                                                                          |
| SDTEST-152 | `cloud_sync.rs::sync_now_falls_back_get_after_404_post`                                  | SDUC-100 | Green        | Added 2026-07-09. Loopback `TcpListener` mock counts POST + GET hits and serves a canonical `SyncPayload` on the fallback GET. Asserts POST fired once, GET fired once, payload parsed.                                                                                                                                                                                                                                                         |
| SDTEST-153 | `cloud_sync.rs::sync_now_falls_back_get_after_405_post`                                  | SDUC-100 | Green        | Added 2026-07-09. Same shape as SDTEST-152, 405 as trigger.                                                                                                                                                                                                                                                                                                                                                                                     |
| SDTEST-154 | `cloud_sync.rs::sync_now_401_surfaces_and_does_not_retry_get`                            | SDUC-100 | Green        | Added 2026-07-09. Critical safety invariant: on 401 the mock verifies **zero GET retries fired** — a rejected token must NOT silently degrade to an unauthenticated GET. Combined with the `sync_now` shape (fetch → merge → save), this guarantees a bad token can never reach `merge_profiles(empty_payload)` and prune every CloudSync connection in the local store. Error message must mention `401` or `rejected` for the toast contract. |
| SDTEST-155 | `cloud_sync.rs::merge_overwrites_local_tags_with_remote_tags` | SDUC-102 | Green | Added 2026-07-09 (cluster M). **Contract correction** — my initial inventory said "preserves local tags"; the actual impl OVERWRITES them (cloud is authoritative). Test pins current shape so a future "merge tags" change is a deliberate contract decision. |
| SDTEST-156 | `cloud_sync.rs::merge_does_not_duplicate_when_same_profile_arrives_twice` | SDUC-101 | Green | Added 2026-07-09 (cluster M). Defence against a Manage pagination-boundary duplicate. First occurrence pushes, second updates in place — final count exactly 1, last-write-wins on fields. |

---

## 11. `config/cloud_account.rs` — auth + browser flow

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-170 | `cloud_account.rs::account_info_initial_and_display` | SDUC-141 | Green | |
| SDTEST-171 | `cloud_account.rs::whoami_account_info_falls_back_to_label` | SDUC-141 | Green | |
| SDTEST-172 | `cloud_account.rs::whoami_parses_is_superadmin_into_account` | SDUC-142 | Green | |
| SDTEST-173 | `cloud_account.rs::app_mode_default_is_dev` | SDUC-151 | Green | |
| SDTEST-174 | `cloud_account.rs::browser_connect_url_encodes_and_appends_provider` | SDUC-144, SDUC-150 | Green | |
| SDTEST-175 | `cloud_account.rs::percent_roundtrip` | SDUC-144 | Green | |
| SDTEST-176 | `cloud_account.rs::is_auth_rejected_detects_401_403` | SDUC-148 | Green | |
| SDTEST-177 | `cloud_account.rs::browser_connect_returns_token_on_matching_state` | SDUC-145 | Green | |
| SDTEST-178 | `cloud_account.rs::browser_connect_ignores_wrong_state_and_favicon_then_accepts` | SDUC-145 | Green | |
| SDTEST-179 | `cloud_account.rs::browser_connect_times_out` | SDUC-146 | Green | |
| SDTEST-180 | `cloud_account.rs::browser_connect_percent_decodes_token` | SDUC-147 | Green | |
| SDTEST-181 | `cloud_account.rs::login_password_sends_credentials_and_device_name` | SDUC-140 | Green | Contract mock asserts the POST route and exact JSON body, then parses the returned token and identity. |
| SDTEST-182 | *to write* — logout POSTs `{action:"logout"}` and swallows errors | SDUC-143 | **Red / P1** | Assert local state clears even when server 500s. |
| SDTEST-183 | *to write* — provider=None targets the password page URL | SDUC-149 | **Red / P1** | Regression sensor for the URL shape. |
| SDTEST-184 | `cloud_account.rs::resolve_effective_mode_*` + `can_switch_true_for_signed_in_inklura_support_or_superadmin` + `allowed_modes_matches_the_tier_table` | SDUC-152 | Green | Full 24-cell role/mode truth table: logged-out is defensive User behind welcome; regular/customer-admin is User-only; `inklura_support` gets User+Support; super-admin gets all three. Workspace delegates effective mode, switcher, palette and execution guards to this matrix. |

---

## 12. `config/manage_sites.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-200 | `manage_sites.rs::area_url_encodes_all_params` | SDUC-122 | Green | |
| SDTEST-201 | `manage_sites.rs::area_url_handles_empty_host` | SDUC-122 | Green | |
| SDTEST-202 | `manage_sites.rs::sites_payload_parses_contract_example` | SDUC-121 | Green | |
| SDTEST-203 | `manage_sites.rs::display_label_falls_back` | SDUC-123 | Green | |
| SDTEST-204 | *to write* — fetch_sites Bearer header shape | SDUC-120 | **Red / P1** | Mock TcpListener assertion on `Authorization` header. |
| SDTEST-205 | *to write* — SitesPayload accepts an empty `sites` array without erroring | SDUC-121 | **Red / P1** | Fresh tenants have zero sites. |
| SDTEST-206 | *to write* — SitesPayload with unknown extra fields still parses | SDUC-121 | **Red / P1** | Forward compat. |

---

## 13. `config/manage_support.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-220 | `manage_support.rs::parse_list_fixture` | SDUC-160 | Green | |
| SDTEST-221 | `manage_support.rs::parse_ticket_fixture_classifies_messages` | SDUC-161 | Green | |
| SDTEST-222 | `manage_support.rs::parses_null_message_and_ticket_strings` | SDUC-162 | Green | |
| SDTEST-223 | `manage_support.rs::parses_iso_string_and_numeric_timestamps` | SDUC-163 | Green | |
| SDTEST-224 | `manage_support.rs::channel_glyphs_have_a_fallback` | SDUC-164 | Green | |
| SDTEST-225 | `manage_support.rs::support_{reply,note,status,priority,assign,resolve,read}_*` (7 fns) + `support_writes_surface_401_when_bearer_missing` | SDUC-166 | Green | 8 tests, added 2026-07-09. `TcpListener` mock records POST bodies before responding with the canonical ticket echo. Each test parses the recorded body as `serde_json::Value` and asserts `action`, `id`, and the endpoint-specific field. `support_assign` covers both `"me"` (self-assign) and `""` (unassign) in a single test. `support_read` also asserts no leaked extras (`text`/`status` are null). Bonus 401 test proves a missing Bearer surfaces the typed error. |
| SDTEST-226 | *to write* — non-staff caller receives 403 on staff-only endpoints | SDUC-166 | **Red / P1** | Mock TcpListener returns 403; assert typed error. |
| SDTEST-227 | `manage_support.rs::support_agents_returns_empty_vec_cleanly` | SDUC-165 | Green | Added 2026-07-09. Uses a one-shot canned GET mock (`spawn_canned_get`) that serves `{"ok":true,"agents":[]}`. Guards against a fresh tenant's empty picker crashing the composer. |
| SDTEST-228 | `manage_support.rs::support_list_preserves_server_order` | SDUC-160 | Green | Added 2026-07-09. Fixture is deliberately anti-sorted alphabetically (`z`, `a`, `m`) so a stray `sort_by(|t| t.id)` refactor would flip the order and trip the test. Server-side `lastAt desc` ordering is the contract — client-side re-sort would drop unread/breaching tickets from the top. |
| SDTEST-229 | `manage_support.rs::parses_created_at_alias_and_epoch_seconds` | SDUC-170 | Green | Added 2026-07-08. |
| SDTEST-230 | `manage_support.rs::parses_message_last_at_alias` | SDUC-171 | Green | Added 2026-07-08. Older Manage builds emit `lastAt` on messages. |
| SDTEST-231 | `manage_support.rs::channel_lucide_maps_known_channels` | SDUC-172 | Green | Added 2026-07-08 as part of the Lucide icon migration. |

---

## 14. `config/jeanclaude.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-240 | `jeanclaude.rs::parse_state` | SDUC-180 | Green | |
| SDTEST-241 | `jeanclaude.rs::parse_history_ticket_targets_memory` | SDUC-181 | Green | |
| SDTEST-242 | `jeanclaude.rs::post_actions_and_error_surface` | SDUC-182 | Green | |
| SDTEST-243 | `jeanclaude.rs::wrong_credentials_surface_401` | SDUC-183 | Green | |
| SDTEST-244 | `jeanclaude.rs::is_set_semantics` | SDUC-184 | Green | |
| SDTEST-245 | *to write* — Basic auth header exact base64 shape | SDUC-183 | **Red / P1** | Right now the mock accepts *any* Basic auth. Assert the encoded `user:pass`. |
| SDTEST-246 | `jeanclaude.rs::format_via_shelldeck_prefix_shape_is_pinned` + `format_via_shelldeck_empty_name_still_brackets_cleanly` + `format_via_shelldeck_preserves_text_verbatim` | SDUC-187 | Green | 3 tests, added 2026-07-09. Extracted `jeanclaude::format_via_shelldeck(name, text) -> String` as a pure helper (the inline `format!` in `Workspace::send_jean_ask` now calls this). Contract pinned: square brackets, U+2014 em-dash, trailing space after `]`. Empty-name case still brackets so Slack channel filters stay greppable. Text payload copied byte-for-byte (multi-line + unicode preserved). |
| SDTEST-247 | *to write* — numeric epoch-ms timestamps parse into DateTime<Utc> | SDUC-186 | **Red / P1** | Currently only implicitly checked via history parse — an explicit round-trip test protects it. |
| SDTEST-1054 (jean) | `jeanclaude.rs::resolve_effective_local_wins_over_server` + `resolve_effective_falls_back_to_server_when_local_unset` + `resolve_effective_falls_back_to_server_when_local_none` + `resolve_effective_none_when_neither_set` | SDUC-185 | Green | 4 tests, added 2026-07-09. Ported `JeanConfig::resolve_effective(local, server) -> Option<JeanConfig>` as a pure fn. Local `[jeanclaude]` wins when `is_set()`; unset local falls through to server; neither set → None (feature unavailable). Also see UI inventory (SDTEST-1054). |

---

## 15. `config/jean_fleet.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-260 | `jean_fleet.rs::get_fleet_parses` | SDUC-200 | Green | |
| SDTEST-261 | `jean_fleet.rs::register_heartbeat_dispatch` | SDUC-201 | Green | |
| SDTEST-262 | `jean_fleet.rs::auto_tick_claims_and_executes` | SDUC-204 | Green | |
| SDTEST-263 | `jean_fleet.rs::confirm_tick_claims_but_does_not_execute` | SDUC-205 | Green | |
| SDTEST-264 | `jean_fleet.rs::wrong_auth_surfaces_401` | SDUC-208 | Green | |
| SDTEST-265 | `jean_fleet.rs::parses_iso_and_null_timestamps` | SDUC-200 | Green | |
| SDTEST-266 | `jean_fleet.rs::parse_stream_json_finds_result` | SDUC-203 | Green | |
| SDTEST-267 | `jean_fleet.rs::claude_executor_command_matches_bot_argv_and_auth_contract` | SDUC-202 | Green | Inspects the non-spawned `Command` builder, including optional-model omission. |
| SDTEST-268 | `jean_fleet.rs::claude_executor_command_matches_bot_argv_and_auth_contract` | SDUC-202 | Green | `ANTHROPIC_API_KEY` is explicitly removed before spawn. |
| SDTEST-269 | `jean_fleet.rs::claude_executor_command_matches_bot_argv_and_auth_contract` | SDUC-202 | Green | `CLAUDE_CODE_OAUTH_TOKEN` is not overridden or removed, so parent inheritance remains intact. |
| SDTEST-270 | *to write* — runtime_busy prevents concurrent execution | SDUC-207 | **Red / P1** | Fake executor that blocks + a concurrent tick attempt. |
| SDTEST-271 | *to write* — first successful register() persists instance_id, second call reuses it | SDUC-209 | **Red / P1** | Guard against re-registering per boot. |
| SDTEST-272 | `workspace/fleet.rs::runtime_loop_requested_requires_explicit_enablement_and_credentials` | SDUC-206 | Green | The Workspace gate is the layer that prevents `runtime_tick` from ever being reached while disabled; its complete enablement/credentials truth table is pinned in `shelldeck-ui`. |
| SDTEST-1584 | `jean_fleet.rs::jcode_executor_parses_json_output_from_cmd_fake` (`#[cfg(windows)]`) | SDUC-458 | Green | Added 2026-08-06, activated in CI 2026-08-18. The `Core tests (windows-x86_64)` job runs the `.cmd` batch fake through the real `JcodeExecutor` spawn→parse path on `windows-latest`. |

> ⚠️ **Inventory debt:** the Jcode executor tests that shipped with
> `c5dd9c2`/`148b975` (`jcode_executor_uses_run_ndjson_flags_and_prompt_arg`,
> `jcode_executor_parses_json_output`, `jcode_acp_probe_is_explicitly_disabled_by_contract`,
> `jcode_acp_transport_falls_back_to_process_run`,
> `jcode_executor_rejects_relative_or_missing_workdir_before_spawn`,
> `jcode_executor_kills_child_on_timeout`,
> `jcode_acp_fallback_preserves_process_timeout_cancellation`,
> `configured_executor_falls_back_to_legacy_claude_only_when_jcode_cannot_start`)
> are Green in code but have no SDTEST rows yet. SDUC-458 now exists to map
> them to — back-fill pending.

---

## 16. `config/issues.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-290 | `issues.rs::parse_list` | SDUC-220 | Green | |
| SDTEST-291 | `issues.rs::parse_detail` | SDUC-221 | Green | |
| SDTEST-1598 | `issues.rs::detail_future_thread_fields_default_when_absent` | SDUC-459 | Green | Current Manage payloads remain parseable when optional comment channel/quote/delivery and issue thread-state fields are absent; every future field defaults empty. |
| SDTEST-292 | `issues.rs::create_and_comment_bodies` | SDUC-222, SDUC-223 | Green | |
| SDTEST-293 | `issues.rs::staff_actions_surface_403` | SDUC-225 | Green | |
| SDTEST-294 | `issues.rs::missing_bearer_surfaces_401` | SDUC-226 | Green | |
| SDTEST-295 | `issues.rs::create_issue_source_field_is_omitted_when_empty_and_present_when_support` | SDUC-222, SDUC-169 | Green | Added 2026-07-09. Complementary edges to `create_and_comment_bodies`: source="" ⇒ the field is OMITTED from the wire body (not sent as `""`), source="support" ⇒ present as a JSON string. Server default of "user" applies only when the key is absent. |
| SDTEST-1389 | `issues.rs::create_issue_site_target_is_present_or_fully_omitted` | SDUC-222, SDUC-228 | Green | A targeted create carries both `site_id` and `site_label`; the explicit general choice omits both keys so Manage retains its null defaults. |
| SDTEST-1373 | `issues.rs::attachment_receipt_bodies_match_manage_contract` + `attachment_upload_rejects_spoofed_image_bytes` + `attachment_limit_keeps_multipart_below_bext_request_cap` + `upload_issue_attachments_uses_ticket_and_multipart` | SDUC-432 | Green | Pins receipts on request/comment actions, rejects extension/MIME spoofing, reserves multipart headroom below Bext's request cap, and traverses the real ticket → Bearer multipart upload client path against a local mock. |
| SDTEST-1377 | `manage_support.rs::support_messages_parse_share_attachments` + `support_reply_and_note_send_attachment_receipts` | SDUC-432 | Green | Pins the structured Support-message attachment response and the receipt arrays sent for both customer replies and internal notes. |
| SDTEST-1422 | `manage_support.rs::support_posted_attachment_delete_body_matches_manage_contract` | SDUC-432 | Green | Pins the staff Support deletion discriminator and snake_case attachment id sent to Manage. |
| SDTEST-1421 | `issues.rs::posted_attachment_delete_body_matches_manage_contract` | SDUC-432 | Green | Pins the owner/staff deletion discriminator, request id, and snake_case attachment id sent to Manage. |
| SDTEST-296 | *to write* — set_status / assign / set_priority body shapes (mock-asserted) | SDUC-225 | **Red / P1** | Table-driven. |
| SDTEST-297 | *to write* — github_push / github_refresh route shapes (not the GitHub call itself) | SDUC-225 | **Red / P2** | Mock only; the real GH call is out-of-scope. |
| SDTEST-298 | `issues.rs::dispatch_issue_body_carries_id_and_instance_id` | SDUC-225 | Green | Added 2026-07-09. Fleet routing is never exercised live (would fire a real claude job), so this mock-only body assertion is the only guard against a rename like `target_instance`/`instanceId` silently 400ing in prod. The existing mock returns 403 on `dispatch` (non-staff path) but records the POST body BEFORE the 403 fires — assertion happens on the recorder. Snake_case field name `instance_id` is pinned. |

---

## 17. `config/bext_cloud.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-310 | `bext_cloud.rs::config_default_and_connected` | SDUC-240 | Green | |
| SDTEST-311 | `bext_cloud.rs::cli_url_shape` | SDUC-241 | Green | |
| SDTEST-312 | `bext_cloud.rs::parses_sites_with_nulls` | SDUC-244 | Green | |
| SDTEST-313 | `bext_cloud.rs::parses_dashboard_and_instances` | SDUC-248 | Green | |
| SDTEST-314 | `bext_cloud.rs::browser_connect_returns_token` | SDUC-242 | Green | |
| SDTEST-315 | `bext_cloud.rs::browser_connect_ignores_favicon_then_accepts` | SDUC-242 | Green | |
| SDTEST-316 | *to write* — whoami parses super_admin flag | SDUC-243 | **Red / P1** | Downstream gates the "Instances" tab. |
| SDTEST-317 | *to write* — create_site body shape (name / plan / region) | SDUC-245 | **Red / P1** | |
| SDTEST-318 | *to write* — site_action route shapes for go_live / config / destroy | SDUC-246 | **Red / P1** | Table-driven. |
| SDTEST-319 | *to write* — list_instances only invoked with super_admin token (guard at call site) | SDUC-248 | **Red / P2** | Test lives on Workspace side (see UI inventory) — cross-linked. |

---

## 18. `config/bext_instance.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-330 | `bext_instance.rs::list_sites_parses_and_sends_app_id` | SDUC-260 | Green | |
| SDTEST-331 | `bext_instance.rs::create_body_shape` | SDUC-261 | Green | |
| SDTEST-332 | `bext_instance.rs::site_actions_send_expected_routes_bodies_and_app_id` | SDUC-262 | Green | Contract mock covers the GET query plus every per-site POST route and body shape. |
| SDTEST-333 | `bext_instance.rs::{list_sites_parses_and_sends_app_id,create_body_shape,site_actions_send_expected_routes_bodies_and_app_id}` | SDUC-260 | Green | The public SDK surface is covered across the shared GET and POST paths; missing header = 400 from the plugin. |

---

## 19. `config/deep_link.rs` + `config/single_instance.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1320 | `deep_link.rs::parses_every_documented_verb` (+ `scheme_is_case_insensitive_but_id_is_not`, `ignores_query_and_fragment_and_trailing_slash`, `rejects_bad_scheme_and_unknown_verbs`, `rejects_malformed_uuid`, `looks_like_prefix_check`) | SDUC-406 | Green | Single choke point every OS-delivered URL flows through, including the standalone Assistant target. |
| SDTEST-1321 | `single_instance.rs::primary_then_secondary_forwards_payload` | SDUC-407 | Green | First = primary, second forwards + bows out, primary receives the link. |
| SDTEST-1322 | `single_instance.rs::stale_discovery_file_is_taken_over` | SDUC-407 | Green | Dead primary → next launch takes over instead of stranding. |
| SDTEST-1323 | `single_instance.rs::wrong_token_handoff_is_rejected` | SDUC-407 | Green | Token guard drops a rogue local hand-off. |

---

## 20. `ai.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1338 | `ai.rs::fake_local_clis_complete_the_real_connection_test_path` | SDUC-413, SDUC-416 | Green | Fake executable Claude/Codex clients traverse config → subprocess → provider parsing → exact connection-test response without contacting a real provider. |
| SDTEST-1339 | `ai.rs::api_payloads_keep_guardrails_outside_untrusted_input_and_disable_storage` | SDUC-415 | Green | Pins OpenAI `instructions` + `store=false` and Anthropic `system`, separate from untrusted user context. |
| SDTEST-1340 | `ai.rs::configured_cli_requires_an_executable_file` | SDUC-413 | Green | A present but non-executable custom CLI path cannot be reported as available. |
| SDTEST-1342 | `app_config.rs::ai_config_round_trips_without_any_credential_field` | SDUC-413, SDUC-415 | Green | Pins backward-compatible `[ai]` persistence while proving API credentials have no serializable config field. |
| SDTEST-1344 | `ai.rs::pending_ai_drafts_survive_disk_round_trip_and_keep_latest_hundred` | SDUC-418 | Green | Persists pending integrated-workflow drafts, restores their typed target/provider fields, and caps the durable file to the latest 100 entries. |
| SDTEST-1347 | `ai.rs::integrated_analysis_capabilities_have_stable_distinct_storage_keys` | SDUC-418 | Green | Support summary/triage and Script explanation/review keep distinct stable snake_case keys in the persistent draft store. |
| SDTEST-1348 | `ai.rs::host_context_exposes_identity_without_credential_paths` | SDUC-415 | Green | The host directory contains the alias/address/user/port needed for contextual references but excludes SSH identity-file paths. |
| SDTEST-1350 | `ai.rs::generated_script_json_populates_metadata_and_strips_markdown_fences` | SDUC-418 | Green | Structured Script-form output maps language/category, requires name/body, and tolerates accidental outer JSON or inner code fences without leaking them into the editor. |
| SDTEST-1353 | `ai.rs::script_review_diff_preserves_context_and_marks_replacements` | SDUC-420 | Green | The bounded line diff keeps unchanged lines and marks removed/added script lines for review before replacement. |
| SDTEST-1356 | `ai.rs::generated_request_json_populates_reviewable_form_fields` | SDUC-422 | Green | Structured request output requires title/description, validates the supported priority enum, and tolerates an accidental outer JSON fence before filling the unsent form. |
| SDTEST-1358 | `ai.rs::issue_triage_json_preserves_explicit_changes_and_validates_priority` | SDUC-423 | Green | Strict triage JSON preserves nullable priority/assignee mutations, validates the supported priority enum, bounds next actions, and distinguishes analysis-only output from applicable changes. |
| SDTEST-1367 | `ai.rs::legacy_ai_drafts_load_as_pending_tasks_and_status_changes_persist` | SDUC-418, SDUC-429 | Green | Proves old draft JSON remains readable as a pending task and that the new durable lifecycle status survives the same bounded store. |
| SDTEST-1369 | `ai.rs::ai_action_policies_default_to_confirmation_and_map_exact_capabilities` | SDUC-430 | Green | Pins safe defaults, exact capability mapping, moderate automatic execution, and forced confirmation for every high-risk plan. |
| SDTEST-1371 | `ai.rs::diagnostic_plans_are_bounded_and_reject_mutating_or_unbounded_commands` | SDUC-431 | Green | Accepts one to five distinct read-only steps and rejects elevation, mutation, shell operators, duplicate commands, and unbounded follow modes. |
| SDTEST-1407 | `ai.rs::ai_running_status_excludes_drafts_and_confirmation_waits` | SDUC-429 | Green | The tray-running contract includes only `Generating` and `Executing`; ready/pending drafts, confirmation waits, and terminal states remain excluded. |
| SDTEST-1427 | `ai.rs::assistant_turn_routes_request_drafts_and_preserves_normal_chat` | SDUC-452, SDUC-445 | Green | A strict `create_request` route yields one validated draft without a chat completion; `chat` preserves Markdown through the normal completion, and malformed routing safely falls back to chat. The latest message is isolated in bounded untrusted context, and a turn with no user message (Clippy clipboard transform) never calls the action router at all. |
| SDTEST-1430 | `ai.rs::assistant_action_router_accepts_only_bounded_typed_workflow_payloads` | SDUC-454 | Green | Script, Terminal, Support, Jean, and existing-request navigation routes parse into distinct typed actions; empty targets and oversized dispatch content are rejected before Workspace orchestration. |

---

## 21. `config/autostart.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1352 | `.github/workflows/release.yml::Build (macos-aarch64)` | SDUC-419 | Green | The release matrix compiles the macOS-only `AutoLaunch::new(app, path, use_launch_agent, args)` branch; Linux CI cannot type-check this platform-specific signature. |

---

## 22. `git.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1387 | `git.rs::porcelain_branch_status_parses_in_one_pass` | SDUC-437 | Green | Pins normal/upstream, unborn, and detached branch headers plus staged/modified/untracked counts from the single `git status --porcelain=v1 --branch` response. |

---

## 23. `ai/clippy.rs` + `companion/` + `[clippy]` config

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1460 | `ai/clippy.rs::defaults_are_safe_and_unknown_character_falls_back` | SDUC-447 | Green | Clippy and desktop roaming default off; unknown roster IDs safely resolve to Clippy. |
| SDTEST-1461 | `ai/clippy.rs::context_rejects_blank_oversized_and_password_roles` | SDUC-446 | Green | Blank/oversized input and password-role accessibility context cannot reach a provider. |
| SDTEST-1462 | `ai/clippy.rs::prompt_delimits_and_redacts_untrusted_context` | SDUC-445, SDUC-446 | Green | Provider input carries explicit trust boundaries and removes common bearer-token material. |
| SDTEST-1463 | `ai/clippy.rs::ai_context_omits_screenshot_bytes_and_delimits_titles` | SDUC-446 | Green | Context metadata never serializes screenshot bytes and treats titles as untrusted data. |
| SDTEST-1464 | `ai/clippy.rs::proposal_and_replace_payload_are_bounded` | SDUC-446 | Green | Result and replacement payloads reject blank or excessive content. |
| SDTEST-1465 | `ai/clippy.rs::stale_selection_identity_is_detected` | SDUC-446 | Green | Window/range identity and selected text must still match before replacement. |
| SDTEST-1466 | `ai/clippy.rs::audit_metadata_excludes_source_and_result_content` | SDUC-446 | Green | Durable audit copy records operation and counts, never private text. |
| SDTEST-1467 | `companion/geometry.rs::window_filter_rejects_fullscreen_and_invalid_windows` | SDUC-449 | Green | Invalid, minimized, fullscreen and desktop surfaces are not walkable. |
| SDTEST-1468 | `companion/geometry.rs::work_area_clamps_points` | SDUC-448 | Green | Recovery cannot strand the overlay beyond a display work area. |
| SDTEST-1469 | `companion/navigation.rs::shared_edge_route_prefers_overlap` | SDUC-448 | Green | Adjacent monitors route through their real overlapping edge. |
| SDTEST-1470 | `companion/navigation.rs::disconnected_displays_use_portal_route` | SDUC-448 | Green | Gapped monitor layouts use an explicit portal transition rather than an invalid walk. |
| SDTEST-1471 | `companion/navigation.rs::removed_monitor_recovers_to_primary_work_area` | SDUC-448, SDUC-449 | Green | Hot-unplug recovers to a valid remaining display. |
| SDTEST-1472 | `companion/simulation.rs::movement_stays_inside_work_area_and_catch_up_is_capped` | SDUC-448 | Green | Fixed-step movement clamps coordinates and limits resume catch-up work. |
| SDTEST-1473 | `companion/simulation.rs::reduced_motion_and_sleeping_request_no_frames` | SDUC-448, SDUC-449 | Green | Static states do not keep the GPU animation loop awake. |
| SDTEST-1474 | `companion/simulation.rs::stale_surface_moves_to_recovering` | SDUC-449 | Green | Invalidated external surfaces cancel the action and enter recovery. |
| SDTEST-1475 | `companion/simulation.rs::seeded_random_source_is_deterministic_and_cooldowns_work` | SDUC-448 | Green | Behavior selection is reproducible and avoids immediate repetition. |
| SDTEST-1476 | `companion/simulation.rs::duty_cycle_blocks_excessive_movement` | SDUC-448 | Green | The simulation enforces its movement duty-cycle budget. |
| SDTEST-1477 | `app_config.rs::clippy_config_defaults_and_surface_opt_in_round_trip` | SDUC-445, SDUC-447 | Green | Old configs parse safely and selected character/desktop preferences persist without enabling AI implicitly. |
| SDTEST-1482 | `ai/clippy.rs::fake_adapter_preserves_copy_fallback_and_rejects_stale_replacement` | SDUC-446 | Green | One fake-adapter workflow covers apply, unsupported, closed target, stale focus/text, password role, and permission-denied replacement while retaining the reviewed draft. |
| SDTEST-1505 | `companion/physics.rs::gravity_accelerates_dynamic_body_and_clamps_terminal_velocity` | SDUC-451 | Green | The dedicated single-body AABB solver applies gravity to a Dynamic companion and clamps falling speed at the configured terminal velocity. |
| SDTEST-1506 | `companion/physics.rs::swept_descending_collision_does_not_tunnel_through_window_top` | SDUC-451 | Green | Descending swept collision catches a window top crossed in one fixed step instead of tunneling through it. |
| SDTEST-1507 | `companion/physics.rs::descending_collision_selects_nearest_crossed_platform` | SDUC-451 | Green | When multiple one-way tops are crossed, the solver lands on the nearest upper platform deterministically. |
| SDTEST-1508 | `companion/physics.rs::platforms_without_horizontal_overlap_are_rejected` | SDUC-451 | Green | A window top is eligible only when the falling AABB overlaps it horizontally. |
| SDTEST-1509 | `companion/physics.rs::display_work_area_floor_is_used_when_no_platform_matches` | SDUC-451 | Green | With no valid platform, the display work-area floor becomes the fallback contact and landing surface. |
| SDTEST-1510 | `companion/physics.rs::release_from_drag_bounds_velocity_and_clears_contact` | SDUC-451 | Green | Drag release switches to Dynamic, clears the previous stable contact, and clamps the sampled release velocity. |
| SDTEST-1511 | `companion/physics.rs::repeated_steps_are_deterministic_and_stale_contacts_invalidate` | SDUC-451 | Green | Equal fixed-step inputs produce equal results, and source-generation changes invalidate stale surface contacts before falling resumes. |
| SDTEST-1532 | `companion/physics.rs::side_wall_collision_clamps_and_reflects_horizontal_velocity` | SDUC-451 | Green | The single-body AABB solver cannot leave the display horizontally; wall impacts reflect with bounded restitution and tiny residual velocities settle to zero. |
| SDTEST-1533 | `companion/physics.rs::ceiling_collision_clamps_and_reflects_upward_velocity_downward` | SDUC-451 | Green | Upward movement clamps at the work-area ceiling and reflects downward without creating a false landing contact. |
| SDTEST-1547 | `companion/physics.rs::diagonal_sweep_does_not_land_on_platform_only_overlapped_by_union_corridor` | SDUC-451 | Green | Descending diagonal collision evaluates horizontal overlap at vertical time-of-impact, preventing false landings on platforms crossed only by the broad movement corridor. |
| SDTEST-1548 | `companion/physics.rs::equal_height_platform_selection_is_stable_when_input_order_is_reversed` | SDUC-451 | Green | Equal-height overlapping platforms use a stable identity/generation/geometry tie-break rather than native enumeration order. |
| SDTEST-1549 | `companion/physics.rs::expanded_work_area_floor_wakes_sleeping_body_instead_of_leaving_it_suspended` | SDUC-448, SDUC-451 | Green | A changed work-area floor invalidates an old sleeping floor contact and resumes falling instead of leaving the mascot suspended. |
| SDTEST-1550 | `companion/physics.rs::zero_vertical_span_collision_check_is_safe_and_uses_current_horizontal_interval` | SDUC-451 | Green | Zero-span collision checks avoid division errors and use the current horizontal interval deterministically. |

---

## 24. `ai/mentions.rs` + `ai/attachments.rs` + `config/manage_directory.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-1622 | `ai/mentions.rs::user_mode_cannot_reference_dev_only_kinds` | SDUC-464 | Green | The kind gate follows the effective mode: User reaches sites/requests/people, never hosts, terminals or tickets. |
| SDTEST-1652 | `ai/mentions.rs::a_dev_super_admin_reaches_every_kind` | SDUC-464 | Green | The positive half of the scope gate. SDTEST-1622 proves a customer cannot reach Dev entities and SDTEST-1623 proves a closed session reaches none — neither would notice a filter that hid everything from everybody. |
| SDTEST-1623 | `ai/mentions.rs::signed_out_scope_offers_nothing` | SDUC-464 | Green | No kind survives a signed-out scope, even one carrying super-admin flags. |
| SDTEST-1624 | `ai/mentions.rs::foreign_site_rows_are_dropped_for_non_staff_and_kept_for_staff` | SDUC-464 | Green | The tenant/site gate: another site's row is invisible to a customer, visible to staff, and an unbound row is always in scope. |
| SDTEST-1625 | `ai/mentions.rs::scoped_candidates_filters_the_whole_directory` | SDUC-464 | Green | Both gates applied over a mixed directory in one pass. |
| SDTEST-1626 | `ai/mentions.rs::super_admins_are_never_mentionable` | SDUC-464 | Green | Every spelling of the role is denied, and the server's `mentionable: true` does not override it. |
| SDTEST-1627 | `ai/mentions.rs::caret_query_ignores_email_addresses` | SDUC-464 | Green | `user@host` is an address, not a mention; a query is only recognised when `@` starts a word. |
| SDTEST-1628 | `ai/mentions.rs::inserting_replaces_the_partial_query_and_spaces_the_token` | SDUC-464 | Green | Completion rewrites the typed `@query` in place and does not double a space that is already there. |
| SDTEST-1629 | `ai/mentions.rs::deleting_the_text_deletes_the_mention` | SDUC-464 | Green | The draft is authoritative: a reference whose token left the text does not travel. |
| SDTEST-1630 | `ai/mentions.rs::repeated_labels_are_matched_by_occurrence_count` | SDUC-464 | Green | Two hosts sharing an alias survive exactly as many times as the token appears. |
| SDTEST-1631 | `ai/mentions.rs::removing_a_chip_removes_one_token_and_its_space` | SDUC-464 | Green | Chip removal edits one occurrence and collapses the space it leaves. |
| SDTEST-1632 | `ai/mentions.rs::picker_ranks_prefix_matches_first_and_is_stable` | SDUC-464 | Green | Prefix beats substring, the order is stable between keystrokes, and a kind token narrows to that kind. |
| SDTEST-1633 | `ai/mentions.rs::candidate_payloads_are_redacted_and_bounded` | SDUC-464 | Green | Credential-looking keys are redacted and long strings truncate with a visible marker. |
| SDTEST-1634 | `ai/mentions.rs::prompt_block_is_empty_without_mentions` | SDUC-464 | Green | An ordinary turn keeps its exact previous shape; a mentioned turn carries the resolved facts. |
| SDTEST-1635 | `ai/mentions.rs::every_kind_has_a_bundled_icon_and_a_distinct_token` | SDUC-464 | Green | Guards the catalogue: unique tokens, non-empty bundled icons, namespaced label keys. |
| SDTEST-1636 | `ai/attachments.rs::kind_is_detected_from_content_not_from_the_extension` | SDUC-465 | Green | A `.txt` holding PNG magic is an image; a `.png` holding text is text. |
| SDTEST-1637 | `ai/attachments.rs::binary_that_is_neither_image_nor_utf8_is_rejected` | SDUC-465 | Green | Unsupported binary and empty files are refused rather than inlined as mojibake. |
| SDTEST-1638 | `ai/attachments.rs::oversized_files_are_refused_per_kind` | SDUC-465 | Green | Separate image and text ceilings, each reported with its own limit. |
| SDTEST-1639 | `ai/attachments.rs::long_text_is_truncated_with_a_visible_marker` | SDUC-465 | Green | Truncation is announced and the original size is preserved in the metadata. |
| SDTEST-1640 | `ai/attachments.rs::cli_backends_never_accept_images` | SDUC-465 | Green | The capability matrix: CLI backends refuse images, API backends accept them. |
| SDTEST-1641 | `ai/attachments.rs::text_attachments_reach_every_backend_and_are_delimited` | SDUC-465 | Green | Text is portable and always inlined inside `<untrusted>` delimiters. |
| SDTEST-1642 | `ai/attachments.rs::image_bytes_never_enter_the_prompt_text` | SDUC-465 | Green | Same rule as Clippy screenshots: the transcript carries the name, never the payload. |
| SDTEST-1643 | `ai/attachments.rs::attachment_count_is_capped` | SDUC-465 | Green | |
| SDTEST-1644 | `ai/attachments.rs::names_are_reduced_to_a_basename` | SDUC-465 | Green | Unix and Windows paths both lose their directory before leaving the machine. |
| SDTEST-1645 | `ai.rs::image_attachments_ride_a_content_block_and_leave_the_guardrail_alone` | SDUC-465 | Green | OpenAI `input_image` and Anthropic `image` blocks are built with the text part first; the guardrail stays out of the untrusted content. |
| SDTEST-1646 | `ai.rs::mentions_and_attachments_land_in_the_user_message_only` | SDUC-464, SDUC-465 | Green | Both blocks precede the user request and neither reaches `SYSTEM_GUARDRAIL`. |
| SDTEST-1647 | `ai.rs::a_text_only_backend_refuses_an_image_instead_of_dropping_it` | SDUC-465 | Green | The turn fails loudly on a CLI backend rather than answering about evidence it never received. |
| SDTEST-1648 | `config/manage_directory.rs::people_request_carries_the_bearer_token_and_the_site_scope` | SDUC-464 | Green | Mock listener asserts the route, the `site_id` scope and the Bearer header. |
| SDTEST-1649 | `config/manage_directory.rs::a_missing_endpoint_is_an_empty_directory_not_a_failure` | SDUC-464 | Green | 404/403/400 degrade to "no people" — the endpoint ships in a separate `bext` PR. |
| SDTEST-1650 | `config/manage_directory.rs::an_expired_token_is_reported_so_the_session_can_be_invalidated` | SDUC-464 | Green | 401 stays an error so the session-invalidation path can run. |
| SDTEST-1651 | `config/manage_directory.rs::super_admins_are_dropped_even_when_the_server_marks_them_mentionable` | SDUC-464 | Green | Client-side defense in depth over the server's own verdict. |

---

## Retired tests

*(none yet)*
