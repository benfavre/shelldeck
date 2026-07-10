# SDTEST inventory — `shelldeck-terminal`

> Rules for this file live in [`.agents/testing.md`](../../.agents/testing.md).
> Use case IDs (`SDUC-…`) resolve in [`USE_CASES.md`](./USE_CASES.md).

**Big picture.** `parser.rs`, `grid.rs`, and `url.rs` are the best-tested
files in the workspace — 66 unit tests together. The VT100/220 subset
that ShellDeck supports is essentially locked in place. The gaps live
in `pty.rs`, `session.rs`, and `colors.rs` — none of which have any
tests today. `pty` in particular must work identically on all three
targets (SDUC-022), so this crate is the highest-leverage place to add
cross-platform coverage.

---

## 1. `parser.rs` — VTE dispatcher

| ID | Location | SDUC | Status |
|---|---|---|---|
| SDTEST-700 | `parser.rs::printable_text_is_written` | SDUC-001 | Green |
| SDTEST-701 | `parser.rs::control_chars_newline_and_cr` | SDUC-002 | Green |
| SDTEST-702 | `parser.rs::sgr_bold_and_reset` | SDUC-003 | Green |
| SDTEST-703 | `parser.rs::sgr_multiple_attributes_in_one_sequence` | SDUC-003 | Green |
| SDTEST-704 | `parser.rs::sgr_named_fg_and_bg` | SDUC-003 | Green |
| SDTEST-705 | `parser.rs::sgr_256_color_foreground` | SDUC-003 | Green |
| SDTEST-706 | `parser.rs::sgr_truecolor_background` | SDUC-003 | Green |
| SDTEST-707 | `parser.rs::sgr_curly_underline_colon_subparam` | SDUC-003 | Green |
| SDTEST-708 | `parser.rs::sgr_empty_sequence_resets` | SDUC-003 | Green |
| SDTEST-709 | `parser.rs::cup_positions_cursor_one_indexed` | SDUC-004 | Green |
| SDTEST-710 | `parser.rs::cursor_forward_and_up_csi` | SDUC-004 | Green |
| SDTEST-711 | `parser.rs::cha_sets_absolute_column` | SDUC-004 | Green |
| SDTEST-712 | `parser.rs::ed_erase_display_via_csi` | SDUC-005 | Green |
| SDTEST-713 | `parser.rs::el_erase_line_via_csi` | SDUC-005 | Green |
| SDTEST-714 | `parser.rs::alt_screen_enter_leave_via_csi` | SDUC-009 | Green |
| SDTEST-715 | `parser.rs::cursor_visibility_mode_25` | SDUC-017 | Green |
| SDTEST-716 | `parser.rs::bracketed_paste_mode_toggle` | SDUC-016 | Green |
| SDTEST-717 | `parser.rs::scroll_region_set_via_csi` | SDUC-006 | Green |
| SDTEST-718 | `parser.rs::osc_sets_window_title` | SDUC-014 | Green |
| SDTEST-719 | `parser.rs::osc_2_sets_title_st_terminated` | SDUC-014 | Green |
| SDTEST-720 | `parser.rs::osc_palette_color_override` | SDUC-014 | Green |
| SDTEST-721 | `parser.rs::osc_133_prompt_marker` | SDUC-014 | Green |
| SDTEST-722 | `parser.rs::esc_save_restore_cursor` | SDUC-008 | Green |
| SDTEST-723 | `parser.rs::esc_ris_full_reset` | SDUC-018 | Green |
| SDTEST-724 | `parser.rs::dec_special_graphics_charset` | SDUC-015 | Green |
| SDTEST-725 | `parser.rs::partial_and_malformed_sequences_do_not_panic` | SDUC-020 | Green |
| SDTEST-726 | `parser.rs::cpr_response_sent_when_channel_present` | SDUC-019 | Green |

### Gaps

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-750 | *to write* — soft reset (`DECSTR`) does not clear scrollback | SDUC-018 | **Red / P1** | Distinct from RIS; matters for shells that soft-reset on prompt. |
| SDTEST-751 | *to write* — DECSCUSR cursor style (block/bar/underline, blinking) parsed and observable | SDUC-017 | **Red / P2** | Fashionable shells (starship) rely on this. |
| SDTEST-752 | *to write* — Mouse tracking modes (1000, 1002, 1006) toggle via CSI without side effects | SDUC-016 | **Red / P1** | Rare bugs class: paste mode toggled by a false-positive during a scroll event. |
| SDTEST-753 | *to write* — OSC 52 clipboard copy is exposed to the host via a channel | SDUC-014 | **Red / P1** | Needed by tmux + Emacs users. |
| SDTEST-754 | *to write* — OSC 7 working-directory report is observable | SDUC-014 | **Red / P1** | Consumed by shell integration. |
| SDTEST-755 | *to write* — parser handles arbitrarily long OSC strings without buffer growth panic | SDUC-020 | **Red / P1** | Fuzz-style regression — a common attacker/PDF-paste class. |

---

## 2. `grid.rs` — screen model

| ID | Location | SDUC | Status |
|---|---|---|---|
| SDTEST-800 | `grid.rs::ring_buffer_evicts_oldest_when_full` | SDUC-010 | Green |
| SDTEST-801 | `grid.rs::ring_buffer_pop_returns_newest` | SDUC-010 | Green |
| SDTEST-802 | `grid.rs::write_char_advances_cursor_and_stores_glyph` | SDUC-001 | Green |
| SDTEST-803 | `grid.rs::write_string_fills_cells_in_order` | SDUC-001 | Green |
| SDTEST-804 | `grid.rs::line_wraps_at_right_edge` | SDUC-001 | Green |
| SDTEST-805 | `grid.rs::auto_wrap_disabled_overwrites_last_column` | SDUC-001 | Green |
| SDTEST-806 | `grid.rs::wide_char_occupies_two_cells` | SDUC-001 | Green |
| SDTEST-807 | `grid.rs::combining_char_attaches_to_previous_cell` | SDUC-001 | Green |
| SDTEST-808 | `grid.rs::newline_moves_down_carriage_return_resets_col` | SDUC-002 | Green |
| SDTEST-809 | `grid.rs::backspace_does_not_wrap_past_col_zero` | SDUC-002 | Green |
| SDTEST-810 | `grid.rs::tab_advances_to_next_eight_stop` | SDUC-002 | Green |
| SDTEST-811 | `grid.rs::newline_at_bottom_scrolls_and_accumulates_scrollback` | SDUC-002, SDUC-010 | Green |
| SDTEST-812 | `grid.rs::scrollback_capped_by_max_scrollback` | SDUC-010 | Green |
| SDTEST-813 | `grid.rs::set_max_scrollback_shrinks_keeping_newest` | SDUC-010 | Green |
| SDTEST-814 | `grid.rs::erase_display_mode2_clears_everything` | SDUC-005 | Green |
| SDTEST-815 | `grid.rs::erase_display_mode0_clears_cursor_to_end` | SDUC-005 | Green |
| SDTEST-816 | `grid.rs::erase_display_mode1_clears_start_to_cursor` | SDUC-005 | Green |
| SDTEST-817 | `grid.rs::erase_line_variants` | SDUC-005 | Green |
| SDTEST-818 | `grid.rs::erase_display_mode3_clears_scrollback` | SDUC-005 | Green |
| SDTEST-819 | `grid.rs::cursor_to_clamps_to_bounds` | SDUC-004 | Green |
| SDTEST-820 | `grid.rs::relative_cursor_movement_clamps` | SDUC-004 | Green |
| SDTEST-821 | `grid.rs::origin_mode_makes_cursor_relative_to_scroll_region` | SDUC-006 | Green |
| SDTEST-822 | `grid.rs::save_and_restore_cursor` | SDUC-008 | Green |
| SDTEST-823 | `grid.rs::set_scroll_region_homes_cursor_and_bounds_scroll` | SDUC-006 | Green |
| SDTEST-824 | `grid.rs::scroll_region_confines_scrolling` | SDUC-006 | Green |
| SDTEST-825 | `grid.rs::insert_and_delete_chars` | SDUC-007 | Green |
| SDTEST-826 | `grid.rs::insert_and_delete_lines` | SDUC-007 | Green |
| SDTEST-827 | `grid.rs::erase_chars_replaces_without_shift` | SDUC-007 | Green |
| SDTEST-828 | `grid.rs::alt_screen_preserves_and_restores_primary` | SDUC-009 | Green |
| SDTEST-829 | `grid.rs::resize_preserves_content_and_clamps_cursor` | SDUC-011 | Green |
| SDTEST-830 | `grid.rs::resize_grow_reflows_soft_wrapped_lines` | SDUC-011 | Green |
| SDTEST-831 | `grid.rs::reverse_index_scrolls_down_at_top` | SDUC-006 | Green |
| SDTEST-832 | `grid.rs::erase_uses_current_background_color` | SDUC-005 | Green |
| SDTEST-833 | `grid.rs::dirty_tracking_clears_and_sets` | SDUC-012 | Green |
| SDTEST-834 | `grid.rs::scroll_view_up_and_to_bottom` | SDUC-010 | Green |
| SDTEST-835 | `grid.rs::reset_clears_grid_but_keeps_dimensions` | SDUC-018 | Green |
| SDTEST-836 | `grid.rs::simple_selection_membership_and_text` | SDUC-013 | Green |

### Gaps

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-870 | *to write* — resize shrink then grow preserves original content | SDUC-011 | **Red / P1** | Real user path: laptop dock/undock cycle. |
| SDTEST-871 | *to write* — selection across a soft-wrap yields text without the soft-wrap glyph | SDUC-013 | **Red / P1** | Regression sensor. |
| SDTEST-872 | *to write* — selection across the alt-screen boundary is safe (no cross-buffer bleed) | SDUC-009, SDUC-013 | **Red / P2** | |
| SDTEST-873 | *to write* — dirty tracking granularity per line (paint only the changed lines) | SDUC-012 | **Red / P1** | Perf regression sensor — reintroducing full-frame dirty flags would silently pass. |
| SDTEST-874 | *to write* — a wide char written past the last column falls to the next line (does not corrupt cell 0) | SDUC-001 | **Red / P1** | |

---

## 3. `url.rs` — URL & path detection

| ID | Location | SDUC | Status |
|---|---|---|---|
| SDTEST-900 | `url.rs::trims_trailing_punctuation_from_detected_web_url` | SDUC-021 | Green |
| SDTEST-901 | `url.rs::detects_file_path_with_line_and_column` | SDUC-021 | Green |
| SDTEST-902 | `url.rs::parses_path_that_contains_colons_and_line_suffix` | SDUC-021 | Green |

### Gaps

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-910 | *to write* — detects `file://` URLs distinctly from bare paths | SDUC-021 | **Red / P2** | |
| SDTEST-911 | *to write* — does not detect a URL inside a code fence unless explicitly enabled | SDUC-021 | **Red / P2** | |
| SDTEST-912 | *to write* — Windows-style path `C:\foo\bar:12` parsed only on target_os = "windows" | SDUC-021 | **Red / P1** | Cross-platform. |

---

## 4. `colors.rs` — palette

Existing: **0 tests.**

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-930 | *to write* — NamedColor::to_rgb round-trips against a canonical VT100 palette | SDUC-025 | **Red / P1** | Cheap; catches accidental palette drift. |
| SDTEST-931 | *to write* — index_to_rgb boundaries: 0, 15, 16, 231, 232, 255 | SDUC-025 | **Red / P1** | Xterm 256 has three regimes (16-colour, 6×6×6 cube, 24-step grayscale). |
| SDTEST-932 | *to write* — TermColor::to_rgba applies foreground vs background default fallback | SDUC-025 | **Red / P1** | |
| SDTEST-933 | *to write* — Truecolor round-trips 24-bit RGB losslessly | SDUC-003, SDUC-025 | **Red / P1** | Regression sensor for the 32-bit alpha packing. |

---

## 5. `pty.rs` — local PTY spawn

Existing: **0 tests.**

**Critical:** PTY is the cross-platform hotspot. Every SDTEST here
must run on all three targets or be gated with a target-cfg reason.

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-960 | `pty.rs::spawn_returns_alive_pty` (`#[cfg(all(test, unix))]`) | SDUC-022 | Green | Added 2026-07-09 (cluster K). Linux CI covers Unix path. macOS/Windows deferred → [`INFRA_BLOCKED.md`](./INFRA_BLOCKED.md). |
| SDTEST-961 | *to write* — spawn honours explicit shell path | SDUC-022 | **Red / P1** | Partially covered — SDTEST-962 uses explicit `/bin/sh`. Standalone assert on `LocalPty::spawn(Some("/bin/dash"), …)` still Red. |
| SDTEST-962 | `pty.rs::write_and_read_echo_round_trip` (`#[cfg(all(test, unix))]`) | SDUC-022, SDUC-023 | Green | Added 2026-07-09 (cluster K). Sentinel + `exit` + 3s timeout cap on the read loop. |
| SDTEST-963 | `pty.rs::resize_returns_ok` (`#[cfg(all(test, unix))]`) | SDUC-024 | Green | Added 2026-07-09 (cluster K). |
| SDTEST-964 | *to write* — resize triggers SIGWINCH on Unix (verify via `stty size`) | SDUC-024 | **Red / P2** | Portable_pty handles the syscall — verifying SIGWINCH delivery adds fragility for a low-value assertion. |
| SDTEST-965 | `pty.rs::is_alive_and_wait_reflect_child_exit` (`#[cfg(all(test, unix))]`) | SDUC-022 | Green | Added 2026-07-09 (cluster K). Combined with SDTEST-966 — `exit 3` → `wait()` returns non-zero, `is_alive()` flips false. Exact code not pinned (fragile across shell implementations). |
| SDTEST-966 | (same as SDTEST-965) | SDUC-022 | Green | Same test — the wait() return-code assertion is bundled with is_alive(). |
| SDTEST-967 | *to write* — dropping PtyMaster kills the child (no zombies) | SDUC-022 | **Red / P0** | Requires a follow-up: `Drop` for `PtyMaster` doesn't kill by default in portable_pty. Deferred pending impl decision. |
| SDTEST-968 | *to write* — spawn Errs cleanly when shell path is invalid | SDUC-022 | **Red / P1** | |

---

## 6. `session.rs` — `TerminalSession` (async wiring)

Existing: **0 tests.**

| ID | Location | SDUC | Status | Notes |
|---|---|---|---|---|
| SDTEST-980 | *to write* — spawn_local pipes PTY output into the grid via the parser | SDUC-023 | **Red / P0** | End-to-end: `echo hello` should land as `hello` in `grid.line_at(cursor.row)`. |
| SDTEST-981 | *to write* — write_input reaches the child (echoing shell writes it back) | SDUC-023 | **Red / P0** | |
| SDTEST-982 | *to write* — output notifier fires exactly once per grid update batch | SDUC-023 | **Red / P0** | Regression sensor: reintroducing a poll loop would silently pass functional tests but drop this one. |
| SDTEST-983 | *to write* — resize propagates to both the grid and the PTY | SDUC-024 | **Red / P0** | |
| SDTEST-984 | *to write* — is_running is true while child lives, false after exit | SDUC-023 | **Red / P1** | |
| SDTEST-985 | *to write* — session state transitions Running → Exited → Failed | SDUC-023 | **Red / P1** | State-machine sanity. |
| SDTEST-986 | *to write* — spawn_ssh honours title, rows, cols and returns the expected channels | SDUC-023, SDUC-044 | **Red / P1** | Cross-referenced with the SSH inventory (SDTEST-520). |

---

## 7. `error.rs`

No behaviour to test — pure `thiserror` enums. **Do not** write tests
here (they would only pin variant order — see the "no bullshit
tests" rule in `.agents/testing.md`).

---

## Retired tests

*(none yet)*
