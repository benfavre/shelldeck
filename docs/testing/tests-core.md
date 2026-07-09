# SDTEST inventory — `shelldeck-core`

> Rules for this file live in [`.agents/testing.md`](../../.agents/testing.md).
> Use case IDs (`SDUC-…`) resolve in [`USE_CASES.md`](./USE_CASES.md).
>
> Status: **Green** exists & passes · **Yellow** exists but weak/flaky ·
> **Red** to write (priority P0/P1/P2) · **Retired** removed on purpose.

Convention for the *Location* column: `<file>::<fn>`. For Green
entries, `git grep <fn>` lands on the code.

---

## 1. `util.rs` — atomic write

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-001 | `util.rs::atomic_write_creates_new_file` | SDUC-091 | Green | |
| SDTEST-002 | `util.rs::atomic_write_overwrites_existing_file` | SDUC-091 | Green | |
| SDTEST-003 | `util.rs::atomic_write_leaves_no_tmp_files` | SDUC-091 | Green | |
| SDTEST-004 | *to write* — atomic_write preserves prior file when write fails mid-way | SDUC-091 | **Red / P1** | Simulate a fake writer that Errs after N bytes; assert the target path is either the *prior* content or absent, never partial. |
| SDTEST-005 | *to write* — atomic_write fsync semantics on Windows | SDUC-091 | **Red / P2** | Windows rename semantics are different; add a Windows-gated regression once the pattern hits a real bug. |

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

---

## 3. `models/{connection,port_forward,script,script_runner,execution,templates,managed_site}.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-030 | `port_forward.rs::zero_is_rejected` + `all_non_zero_ports_are_accepted` | SDUC-313 | Green | 2 tests, added 2026-07-09. Boundary sweep covers 1 / 22 / 1023 / 1024 / 65535. Regression sensor if someone re-adds a `< 1024` privileged-port restriction. |
| SDTEST-031 | *to write* — port forward presets produce valid PortForward objects | SDUC-049 | **Red / P2** | `chrome_devtools_preset`, `web_server_preset`, `opencode_preset`, `dev_server_preset`. |
| SDTEST-032 | `connection.rs::display_name_prefers_alias_falls_back_to_hostname` + `display_name_returns_borrowed_slice` + `new_manual_sets_manual_source_and_default_port` | SDUC-104bis | Green | 3 tests, added 2026-07-09. **Contract correction** — fallback is alias → hostname only, NO UUID fallback (my initial inventory was wrong). Bonus test proves no allocation on paint (`ptr::eq` on the borrowed slice). |
| SDTEST-033 | `connection.rs::connection_string_always_includes_port` | SDUC-104bis | Green | Added 2026-07-09. Port is always in the output, even when it's the default 22 (opinionated contract). |
| SDTEST-034 | `script.rs::extracts_bare_names_dedup_preserves_first_occurrence` + `extracts_defaults_after_colon` + `trims_inner_whitespace_and_ignores_empty` + `same_name_second_occurrence_ignored_even_with_default` + `unclosed_placeholder_is_silently_dropped` | SDUC-060 | Green | 5 tests, added 2026-07-09. Split-on-first-`:` (colon in default preserved), first-occurrence wins on dedup, unclosed `{{…` tolerated. |
| SDTEST-035 | *to write* — extract_variables ignores placeholders inside fenced code blocks | SDUC-060 | **Red / P1** | User pastes a script that itself uses `{{…}}` (Ansible, Vue) — must not be prompted. |
| SDTEST-036 | `script_runner.rs::provided_value_replaces_placeholder` + `missing_value_falls_back_to_inline_default` + `missing_value_without_default_leaves_placeholder` + `extra_values_in_map_are_ignored` + `substitution_is_utf8_safe` + `unclosed_placeholder_does_not_panic` | SDUC-061 | Green | 6 tests, added 2026-07-09. Key contract: **no value + no default → placeholder LEFT UNCHANGED**, not empty. Downstream re-prompt logic depends on this. |
| SDTEST-037 | `script.rs::every_builtin_language_has_a_runnable_spec` + `file_based_languages_declare_an_extension` + `each_builtin_has_a_unique_runner_binary_or_args` | SDUC-062 | Green | 3 tests, added 2026-07-09. Table-driven over `ScriptLanguage::ALL` — adding a new variant without wiring `runner_spec` trips the test. Separates file-based (Shell/Python/Node/Bun/Php/Mysql/Postgresql — non-empty `file_ext`) from subcommand-style (Docker/Compose/Systemd/Nginx — empty `file_ext`, uses `{body_as_args}`). |
| SDTEST-038 | *to write* — build_package_manager_detect_command runs on a real shell (integration) | SDUC-063 | **Red / P1** | Use `bash -c "…"` locally; assert exit + expected PM name. |
| SDTEST-039 | *to write* — build_dependency_check_command shape | SDUC-064 | **Red / P1** | |
| SDTEST-040 | *to write* — get_install_command per package manager × dep | SDUC-064 | **Red / P1** | Table-driven test: apt/yum/dnf/apk/brew/pacman/zypper × representative deps. |
| SDTEST-041 | *to write* — Script::builtin_* round-trip through serde | SDUC-065 | **Red / P2** | |
| SDTEST-042 | *to write* — all_templates() has unique IDs, non-empty bodies, at least one var each | SDUC-066 | **Red / P1** | Cheap invariant check; grows for free with new templates. |
| SDTEST-043 | *to write* — ScriptTemplate::to_script produces a Script with matching runner spec | SDUC-066 | **Red / P2** | |
| SDTEST-044 | `execution.rs::new_starts_in_running_state` + `append_output_accumulates` + `finish_with_zero_marks_succeeded_and_produces_duration` + `finish_with_non_zero_marks_failure` + `connection_id_is_preserved` | SDUC-067 | Green | 5 tests, added 2026-07-09. Full lifecycle sweep: `is_running` / `succeeded` / `duration_secs` transitions, non-zero exit codes (including negative like `-1` and 127), local vs remote (`connection_id`) round-trip. 5ms sleep in the finish test to make duration observable at ms precision. |
| SDTEST-045 | *to write* — ManagedSite::from_nginx maps a DiscoveredSite → ManagedSite correctly | SDUC-072 | **Red / P2** | Currently no test on the model constructors. |
| SDTEST-046 | *to write* — ManagedSite::url, has_ssl, port derived from the source site | SDUC-072 | **Red / P2** | |
| SDTEST-047 | *to write* — ManagedSite::from_database preserves engine + size | SDUC-073 | **Red / P2** | |

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
| SDTEST-069 | *to write* — AppConfig::default matches documented first-run values | SDUC-093 | **Red / P2** | Cheap; catches accidental default drift. |
| SDTEST-070 | *to write* — save_to writes atomically | SDUC-091 | **Red / P0** | The config is the user's investment; a torn write on power loss is unrecoverable. |
| SDTEST-071 | *to write* — ConfigWatcher fires the callback on external edit (debounced) | SDUC-090 | **Red / P1** | Use a `TempDir` + `std::fs::write` twice within the debounce window. |

---

## 5. `config/store.rs` — connection store

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-080 | `store.rs::round_trip_with_data` | SDUC-087 | Green | |
| SDTEST-081 | `store.rs::load_from_missing_creates_empty` | SDUC-088 | Green | |
| SDTEST-082 | `store.rs::load_from_corrupt_returns_err` | SDUC-088 | Green | |
| SDTEST-083 | *to write* — save writes atomically | SDUC-091 | **Red / P0** | Same rationale as SDTEST-070. |
| SDTEST-084 | *to write* — Manual + SshConfig + CloudSync coexist in the same store round-trip | SDUC-087 | **Red / P1** | Regression sensor for cloud_sync merge (SDUC-104). |

---

## 6. `config/workspace_state.rs` — restored tabs

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-090 | `workspace_state.rs::round_trip_with_tabs` | SDUC-089 | Green | |
| SDTEST-091 | `workspace_state.rs::load_from_missing_returns_default` | SDUC-089 | Green | |
| SDTEST-092 | `workspace_state.rs::clear_at_removes_file` | SDUC-089 | Green | |
| SDTEST-093 | `workspace_state.rs::load_from_corrupt_returns_err` | SDUC-089 | Green | |

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
| SDTEST-106 | *to write* — Include directive expands nested files | SDUC-040 | **Red / P1** | Common shape (`Include ~/.ssh/conf.d/*`); currently unlabelled. |
| SDTEST-107 | *to write* — wildcard `Host *` fields apply as defaults to specific hosts | SDUC-040 | **Red / P1** | |
| SDTEST-108 | *to write* — never writes to ~/.ssh/config (guard test) | SDUC-040 | **Red / P0** | Contract per AGENTS.md "Critical Rules". Use a `TempDir` + `std::fs::metadata` sensor. |

---

## 8. `config/keychain.rs` — OS keychain wrapper

Existing: **0 tests**.

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-120 | *to write* — store / get / delete password round-trip (Linux) | SDUC-042 | **Red / P0** | On Linux dev machine, the `keyring` crate uses Secret Service — mock it via `SecretServiceMock` or run only when `SHELLDECK_LIVE_KEYCHAIN=1`. |
| SDTEST-121 | *to write* — same on macOS | SDUC-042, SDUC-334 | **Red / P0** | CI-gated (macOS runner). |
| SDTEST-122 | *to write* — same on Windows | SDUC-042, SDUC-334 | **Red / P0** | CI-gated (Windows runner). |
| SDTEST-123 | *to write* — get_password returns Ok(None) for missing entry (not Err) | SDUC-042 | **Red / P1** | Consumers rely on this distinction. |
| SDTEST-124 | *to write* — key passphrase namespace does not collide with password namespace | SDUC-042 | **Red / P1** | Prefix / service-name test. |

---

## 9. `config/themes.rs` — builtin themes

Existing: **0 tests**.

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-130 | *to write* — builtins() returns exactly the four shipped themes with the right names | SDUC-092 | **Red / P1** | Cheap invariant. |
| SDTEST-131 | *to write* — by_name(unknown) falls back to dark, does not panic | SDUC-092 | **Red / P0** | Regression class: a stale theme name in `shelldeck.toml` crashing the boot. |
| SDTEST-132 | *to write* — every builtin has a full 16-colour palette + a background + a foreground | SDUC-092, SDUC-025 | **Red / P2** | Non-nullable field sanity. |

---

## 10. `config/cloud_sync.rs` — Manage sync + merge

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-140 | `cloud_sync.rs::merge_adds_new_profiles` | SDUC-101 | Green | |
| SDTEST-141 | `cloud_sync.rs::merge_copies_site_binding` | SDUC-106 | Green | |
| SDTEST-142 | `cloud_sync.rs::merge_updates_existing_and_preserves_local_only_fields` | SDUC-102 | Green | |
| SDTEST-143 | `cloud_sync.rs::merge_removes_vanished_cloud_profiles` | SDUC-103 | Green | |
| SDTEST-144 | `cloud_sync.rs::merge_never_touches_manual_or_ssh_config` | SDUC-104 | Green | |
| SDTEST-145 | `cloud_sync.rs::merge_skips_unparseable_ids` | SDUC-105 | Green | |
| SDTEST-146 | `cloud_sync.rs::cloud_sync_config_parses_without_active_site_fields` | SDUC-108 | Green | |
| SDTEST-147 | `cloud_sync.rs::is_configured_semantics` | SDUC-109 | Green | |
| SDTEST-148 | `cloud_sync.rs::remote_profile_parses_nulls_and_missing_fields` | SDUC-110 | Green | |
| SDTEST-149 | `cloud_sync.rs::sync_payload_parses_contract_example` | SDUC-111 | Green | |
| SDTEST-150 | `cloud_sync.rs::merge_reports_no_change_when_nothing_moves` | SDUC-107 | Green | |
| SDTEST-151 | `cloud_sync.rs::live_fetch_sync` (`#[ignore]`) | SDUC-112 | Yellow | Live smoke — gated by env token. Keep. |
| SDTEST-152 | *to write* — sync_now falls back GET after 404 POST | SDUC-100 | **Red / P0** | Contract-critical, mock TcpListener that returns 404 on POST then a valid GET payload. |
| SDTEST-153 | *to write* — sync_now falls back GET after 405 POST | SDUC-100 | **Red / P0** | Same rationale. |
| SDTEST-154 | *to write* — sync_now surfaces 401 without wiping local store | SDUC-100 | **Red / P0** | A bad token must not delete cloud connections. |
| SDTEST-155 | *to write* — merge preserves user-added tags on a CloudSync connection | SDUC-102 | **Red / P1** | Local-only field surface. |
| SDTEST-156 | *to write* — merge does not duplicate when the same profile arrives twice in one payload | SDUC-101 | **Red / P1** | Defence against Manage bug. |

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
| SDTEST-181 | *to write* — login_password sends `{action:"login", email, password}` body | SDUC-140 | **Red / P0** | Only URL/whoami paths are covered; the login body shape is not. Mock TcpListener assertion. |
| SDTEST-182 | *to write* — logout POSTs `{action:"logout"}` and swallows errors | SDUC-143 | **Red / P1** | Assert local state clears even when server 500s. |
| SDTEST-183 | *to write* — provider=None targets the password page URL | SDUC-149 | **Red / P1** | Regression sensor for the URL shape. |
| SDTEST-184 | *to write* — effective_mode(): non-superadmin forced to User even when persisted mode is Dev/Support | SDUC-152 | **Red / P0** | This lives on Workspace (see UI inventory) — pure-logic port possible here on `AppMode` + `AccountInfo`. |

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
| SDTEST-225 | *to write* — support_reply / note / status / priority / assign / resolve / read body shapes | SDUC-166 | **Red / P0** | One table-driven test per write endpoint; assert route + JSON body. |
| SDTEST-226 | *to write* — non-staff caller receives 403 on staff-only endpoints | SDUC-166 | **Red / P1** | Mock TcpListener returns 403; assert typed error. |
| SDTEST-227 | *to write* — support_agents returns [] when server responds with `[]` | SDUC-165 | **Red / P2** | Cheap defence. |
| SDTEST-228 | *to write* — support_list ordering is server order (no sort mutation) | SDUC-160 | **Red / P2** | |
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
| SDTEST-246 | *to write* — say body includes `[via ShellDeck — <name>]` prefix when invoked from support | SDUC-187 | **Red / P1** | The prefix contract is behavioural — regression sensor. |
| SDTEST-247 | *to write* — numeric epoch-ms timestamps parse into DateTime<Utc> | SDUC-186 | **Red / P1** | Currently only implicitly checked via history parse — an explicit round-trip test protects it. |

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
| SDTEST-267 | *to write* — ClaudeExecutor argv shape (fake `Command` builder) | SDUC-202 | **Red / P0** | Contract with slack-claude-bot; a rename here silently breaks parity. |
| SDTEST-268 | *to write* — ClaudeExecutor drops `ANTHROPIC_API_KEY` from env | SDUC-202 | **Red / P0** | Security-adjacent. |
| SDTEST-269 | *to write* — ClaudeExecutor preserves `CLAUDE_CODE_OAUTH_TOKEN` | SDUC-202 | **Red / P0** | Same. |
| SDTEST-270 | *to write* — runtime_busy prevents concurrent execution | SDUC-207 | **Red / P1** | Fake executor that blocks + a concurrent tick attempt. |
| SDTEST-271 | *to write* — first successful register() persists instance_id, second call reuses it | SDUC-209 | **Red / P1** | Guard against re-registering per boot. |
| SDTEST-272 | *to write* — runtime_tick with enabled=false is a no-op | SDUC-206 | **Red / P0** | Safety guarantee per AGENTS.md. |

---

## 16. `config/issues.rs`

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-290 | `issues.rs::parse_list` | SDUC-220 | Green | |
| SDTEST-291 | `issues.rs::parse_detail` | SDUC-221 | Green | |
| SDTEST-292 | `issues.rs::create_and_comment_bodies` | SDUC-222, SDUC-223 | Green | |
| SDTEST-293 | `issues.rs::staff_actions_surface_403` | SDUC-225 | Green | |
| SDTEST-294 | `issues.rs::missing_bearer_surfaces_401` | SDUC-226 | Green | |
| SDTEST-295 | *to write* — create_issue with `source="support"` sets the link field | SDUC-222, SDUC-169 | **Red / P1** | Convert-to-request path. |
| SDTEST-296 | *to write* — set_status / assign / set_priority body shapes (mock-asserted) | SDUC-225 | **Red / P1** | Table-driven. |
| SDTEST-297 | *to write* — github_push / github_refresh route shapes (not the GitHub call itself) | SDUC-225 | **Red / P2** | Mock only; the real GH call is out-of-scope. |
| SDTEST-298 | *to write* — dispatch_issue body includes fleet target | SDUC-225 | **Red / P2** | |

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
| SDTEST-332 | *to write* — get_site / go_live / config_site / destroy_site route shapes | SDUC-262 | **Red / P1** | Table-driven mock. |
| SDTEST-333 | *to write* — every request carries `X-Bext-App-Id` | SDUC-260 | **Red / P0** | Contract; missing header = 400 from the plugin. |

---

## Retired tests

*(none yet)*
