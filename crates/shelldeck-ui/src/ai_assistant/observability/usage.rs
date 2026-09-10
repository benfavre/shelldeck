//! Consumption: the observed session's tokens and cost, and the account
//! windows last reported by Claude and Codex. A figure no provider reported
//! is shown as unavailable, never estimated (SDUC-499).

use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::*;
use shelldeck_core::agent_runtime::AgentProvider;
use shelldeck_core::agent_session::{AgentQuotaSnapshot, AgentSession};
use shelldeck_core::agent_usage::{self, AgentQuota, AgentQuotaWindow};

use super::super::AiAssistantView;
use super::{now_ms, select_observed_session, MONO};
use crate::icons::simple_icon;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

/// Minimum spacing between two reads of the local Codex journal.
const QUOTA_READ_INTERVAL: Duration = Duration::from_secs(60);
/// Used share from which a window is drawn as close to its limit.
const QUOTA_WARNING_PERCENT: u8 = 70;
/// Used share from which the composer footnote raises the window.
const QUOTA_CRITICAL_PERCENT: u8 = 90;
/// Providers whose account windows are listed, with their Simple Icons mark.
const QUOTA_PROVIDERS: [(AgentProvider, &str); 2] = [
    (AgentProvider::Claude, "claudecode"),
    (AgentProvider::Codex, "openai"),
];
const NARROW_NO_BREAK_SPACE: char = '\u{202F}';
const NO_BREAK_SPACE: char = '\u{00A0}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QuotaLevel {
    Normal,
    Warning,
    Critical,
}

pub(super) fn quota_level(used_percent: u8) -> QuotaLevel {
    if used_percent >= QUOTA_CRITICAL_PERCENT {
        QuotaLevel::Critical
    } else if used_percent >= QUOTA_WARNING_PERCENT {
        QuotaLevel::Warning
    } else {
        QuotaLevel::Normal
    }
}

/// A window whose reset time has passed no longer describes current use.
pub(super) fn quota_is_current(quota: &AgentQuota, now_ms: i64) -> bool {
    quota
        .resets_at_ms
        .is_none_or(|resets_at| resets_at > now_ms)
}

/// The most used current window among those at the critical level.
pub(super) fn quota_alert<'a>(
    snapshots: impl IntoIterator<Item = (AgentProvider, &'a AgentQuotaSnapshot)>,
    now_ms: i64,
) -> Option<(AgentProvider, AgentQuota)> {
    snapshots
        .into_iter()
        .flat_map(|(provider, snapshot)| {
            snapshot.quotas.iter().map(move |quota| (provider, *quota))
        })
        .filter(|(_, quota)| {
            quota_is_current(quota, now_ms)
                && quota_level(quota.used_percent) == QuotaLevel::Critical
        })
        .max_by_key(|(_, quota)| quota.used_percent)
}

pub(super) fn uses_french() -> bool {
    rust_i18n::locale().starts_with("fr")
}

fn decimal(value: f64, french: bool) -> String {
    let text = format!("{value:.1}");
    let text = text.strip_suffix(".0").unwrap_or(&text);
    if french {
        text.replace('.', ",")
    } else {
        text.to_string()
    }
}

/// `12,4 k` in French and `12.4k` in English.
pub(super) fn format_token_count(count: u64, french: bool) -> String {
    let (value, unit) = match count {
        0..=999 => return count.to_string(),
        1_000..=999_949 => (count as f64 / 1_000.0, "k"),
        _ => (count as f64 / 1_000_000.0, "M"),
    };
    let number = decimal(value, french);
    if french {
        format!("{number}{NARROW_NO_BREAK_SPACE}{unit}")
    } else {
        format!("{number}{unit}")
    }
}

/// `0,20 $` in French and `$0.20` in English. A cost that would round to
/// zero is written as an upper bound instead.
pub(super) fn format_cost(micro_usd: u64, french: bool) -> String {
    let below_cent = micro_usd > 0 && micro_usd < 5_000;
    let cents = if below_cent {
        1
    } else {
        micro_usd.saturating_add(5_000) / 10_000
    };
    let separator = if french { ',' } else { '.' };
    let amount = format!("{}{separator}{:02}", cents / 100, cents % 100);
    match (french, below_cent) {
        (true, false) => format!("{amount}{NARROW_NO_BREAK_SPACE}$"),
        (true, true) => format!("<{NO_BREAK_SPACE}{amount}{NARROW_NO_BREAK_SPACE}$"),
        (false, false) => format!("${amount}"),
        (false, true) => format!("< ${amount}"),
    }
}

/// `98 %` in French and `98%` in English.
pub(super) fn format_percent(percent: u8, french: bool) -> String {
    if french {
        format!("{percent}{NARROW_NO_BREAK_SPACE}%")
    } else {
        format!("{percent}%")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResetUnit {
    Minutes,
    Hours,
    Days,
}

/// Time left before a window resets: minutes (rounded up) below an hour,
/// whole hours below two days, then whole days.
pub(super) fn reset_after(resets_at_ms: i64, now_ms: i64) -> Option<(u64, ResetUnit)> {
    let remaining = u64::try_from(resets_at_ms.checked_sub(now_ms)?)
        .ok()
        .filter(|remaining| *remaining > 0)?;
    let minutes = remaining.div_ceil(60_000);
    Some(if minutes < 60 {
        (minutes, ResetUnit::Minutes)
    } else if minutes < 48 * 60 {
        (minutes / 60, ResetUnit::Hours)
    } else {
        (minutes / (24 * 60), ResetUnit::Days)
    })
}

pub(super) fn window_label(window: AgentQuotaWindow) -> String {
    match window {
        AgentQuotaWindow::FiveHours => t!("ai.observability.usage_window_5h"),
        AgentQuotaWindow::SevenDays => t!("ai.observability.usage_window_7d"),
    }
    .to_string()
}

fn reset_caption(resets_at_ms: Option<i64>, now_ms: i64) -> Option<String> {
    let (count, unit) = reset_after(resets_at_ms?, now_ms)?;
    let value = match unit {
        ResetUnit::Minutes => t!("ai.observability.usage_minutes", count = count),
        ResetUnit::Hours => t!("ai.observability.usage_hours", count = count),
        ResetUnit::Days => t!("ai.observability.usage_days", count = count),
    }
    .to_string();
    Some(t!("ai.observability.usage_resets_in", value = value).to_string())
}

impl AiAssistantView {
    /// Throttled read of this machine's Codex journal, so the Codex windows
    /// are known before ShellDeck runs Codex itself.
    pub(in crate::ai_assistant) fn refresh_agent_quotas(&mut self, cx: &mut Context<Self>) {
        let Some(console) = self.agent_console.as_ref().map(Entity::downgrade) else {
            return;
        };
        if self
            .agent_quotas_read_at
            .is_some_and(|at| at.elapsed() < QUOTA_READ_INTERVAL)
        {
            return;
        }
        self.agent_quotas_read_at = Some(Instant::now());
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let quotas = cx
                .background_executor()
                .spawn(async move {
                    agent_usage::codex_home().and_then(|home| agent_usage::read_codex_quotas(&home))
                })
                .await;
            if let Some(quotas) = quotas {
                let _ = console.update(cx, |console, cx| {
                    console.observe_quotas(AgentProvider::Codex, quotas, cx)
                });
            }
        })
        .detach();
    }

    /// Tokens of the observed session while it runs.
    pub(super) fn running_session_tokens(&self, cx: &App) -> Option<u64> {
        if !self.agent_observability_enabled {
            return None;
        }
        let console = self.agent_console.as_ref()?.read(cx);
        let session = select_observed_session(console)?;
        (session.status.is_active() && !session.usage.is_empty())
            .then(|| session.usage.tokens.total())
    }

    pub(super) fn current_quota_alert(&self, cx: &App) -> Option<(AgentProvider, AgentQuota)> {
        if !self.agent_observability_enabled {
            return None;
        }
        let console = self.agent_console.as_ref()?.read(cx);
        quota_alert(
            QUOTA_PROVIDERS.iter().filter_map(|(provider, _)| {
                console
                    .quota_snapshot(*provider)
                    .map(|snapshot| (*provider, snapshot))
            }),
            now_ms(),
        )
    }

    /// The session's tokens and cost above each provider's account windows.
    pub(super) fn render_usage(&self, session: Option<&AgentSession>, cx: &App) -> AnyElement {
        let french = uses_french();
        let now = now_ms();
        let console = self.agent_console.as_ref().map(|console| console.read(cx));
        let mut card = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(9.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface());
        if let Some(session) = session {
            card = card
                .child(session_usage(session, french))
                .child(div().h(px(1.0)).bg(ShellDeckColors::border()));
        }
        for (provider, mark) in QUOTA_PROVIDERS {
            let snapshot = console.and_then(|console| console.quota_snapshot(provider));
            card = card.child(quota_row(provider, mark, snapshot, now, french));
        }
        card.into_any_element()
    }
}

fn session_usage(session: &AgentSession, french: bool) -> AnyElement {
    let usage = &session.usage;
    let label = div()
        .flex_1()
        .min_w(px(0.0))
        .truncate()
        .text_size(px(10.5))
        .text_color(ShellDeckColors::text_muted())
        .child(t!("ai.observability.usage_session").to_string());
    if usage.is_empty() {
        return div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(label)
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.5))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("ai.observability.usage_unavailable").to_string()),
            )
            .into_any_element();
    }
    let tokens = &usage.tokens;
    let mut total = t!(
        "ai.observability.usage_tokens",
        value = format_token_count(tokens.total(), french)
    )
    .to_string();
    if let Some(cost) = usage.cost_micro_usd {
        total = format!("{total} · {}", format_cost(cost, french));
    }
    let detail = t!(
        "ai.observability.usage_tokens_detail",
        input = format_token_count(tokens.input, french),
        cached = format_token_count(tokens.cached_input, french),
        output = format_token_count(tokens.output, french)
    )
    .to_string();
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div().flex().items_center().gap(px(8.0)).child(label).child(
                div()
                    .flex_shrink_0()
                    .font_family(MONO)
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(total),
            ),
        )
        .child(
            div().flex().child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(9.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(detail),
            ),
        )
        .into_any_element()
}

fn quota_row(
    provider: AgentProvider,
    mark: &'static str,
    snapshot: Option<&AgentQuotaSnapshot>,
    now_ms: i64,
    french: bool,
) -> AnyElement {
    let mut row = div().flex().items_start().gap(px(10.0)).child(
        div()
            .flex()
            .items_center()
            .gap(px(5.0))
            .w(px(84.0))
            .flex_shrink_0()
            .child(simple_icon(mark, 11.0, ShellDeckColors::text_muted()))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(10.5))
                    .font_weight(FontWeight::MEDIUM)
                    .child(provider.display_name()),
            ),
    );
    for window in [AgentQuotaWindow::FiveHours, AgentQuotaWindow::SevenDays] {
        let quota = snapshot
            .and_then(|snapshot| snapshot.quotas.iter().find(|quota| quota.window == window));
        row = row.child(quota_cell(window, quota, now_ms, french));
    }
    row.into_any_element()
}

fn quota_cell(
    window: AgentQuotaWindow,
    quota: Option<&AgentQuota>,
    now_ms: i64,
    french: bool,
) -> AnyElement {
    let mut line = div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .min_w(px(0.0))
        .child(
            div()
                .flex_shrink_0()
                .font_family(MONO)
                .text_size(px(9.5))
                .text_color(ShellDeckColors::text_muted())
                .child(window_label(window)),
        );
    let mut caption = None;
    match quota {
        Some(quota) if quota_is_current(quota, now_ms) => {
            let color = match quota_level(quota.used_percent) {
                QuotaLevel::Normal => ShellDeckColors::primary(),
                QuotaLevel::Warning => ShellDeckColors::warning(),
                QuotaLevel::Critical => ShellDeckColors::error(),
            };
            line = line
                .child(
                    div()
                        .flex_1()
                        .min_w(px(12.0))
                        .h(px(4.0))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(ShellDeckColors::border())
                        .child(
                            div()
                                .h_full()
                                .w(relative(f32::from(quota.used_percent) / 100.0))
                                .rounded_full()
                                .bg(color),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .font_family(MONO)
                        .text_size(px(9.5))
                        .text_color(color)
                        .child(format_percent(quota.used_percent, french)),
                );
            caption = reset_caption(quota.resets_at_ms, now_ms);
        }
        stale => {
            // A window past its reset no longer says anything about current use.
            let key = if stale.is_some() {
                "ai.observability.usage_reset_done"
            } else {
                "ai.observability.usage_unavailable"
            };
            line = line.child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(9.5))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!(key).to_string()),
            );
        }
    }
    let mut cell = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .flex_1()
        .min_w(px(0.0))
        .child(line);
    if let Some(caption) = caption {
        cell = cell.child(
            div().flex().child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(px(8.5))
                    .text_color(ShellDeckColors::text_muted())
                    .child(caption),
            ),
        );
    }
    cell.into_any_element()
}

#[cfg(test)]
mod tests {
    use shelldeck_core::agent_runtime::AgentProvider;
    use shelldeck_core::agent_session::AgentQuotaSnapshot;
    use shelldeck_core::agent_usage::{AgentQuota, AgentQuotaWindow};

    use super::{
        format_cost, format_percent, format_token_count, quota_alert, quota_is_current,
        quota_level, reset_after, QuotaLevel, ResetUnit,
    };

    #[test]
    fn sdtest_1935_usage_figures_follow_locale_typography_and_thresholds() {
        let thin = '\u{202F}';
        assert_eq!(format_token_count(950, true), "950");
        assert_eq!(format_token_count(12_400, true), format!("12,4{thin}k"));
        assert_eq!(format_token_count(12_400, false), "12.4k");
        assert_eq!(format_token_count(16_000, false), "16k");
        assert_eq!(format_token_count(999_960, false), "1M");
        assert_eq!(format_token_count(1_240_000, true), format!("1,2{thin}M"));

        assert_eq!(format_cost(199_913, true), format!("0,20{thin}$"));
        assert_eq!(format_cost(199_913, false), "$0.20");
        assert_eq!(format_cost(12_345_678, false), "$12.35");
        assert_eq!(format_cost(3_000, false), "< $0.01");
        assert_eq!(format_cost(3_000, true), format!("<\u{00A0}0,01{thin}$"));
        assert_eq!(format_percent(98, true), format!("98{thin}%"));
        assert_eq!(format_percent(98, false), "98%");

        let now = 1_789_000_000_000_i64;
        assert_eq!(
            reset_after(now + 30_000, now),
            Some((1, ResetUnit::Minutes))
        );
        assert_eq!(
            reset_after(now + 42 * 60_000, now),
            Some((42, ResetUnit::Minutes))
        );
        assert_eq!(
            reset_after(now + 3 * 3_600_000 + 59 * 60_000, now),
            Some((3, ResetUnit::Hours))
        );
        assert_eq!(
            reset_after(now + 47 * 3_600_000, now),
            Some((47, ResetUnit::Hours))
        );
        assert_eq!(
            reset_after(now + 4 * 86_400_000 + 3_600_000, now),
            Some((4, ResetUnit::Days))
        );
        assert_eq!(reset_after(now, now), None);
        assert_eq!(reset_after(now - 1, now), None);

        assert_eq!(quota_level(69), QuotaLevel::Normal);
        assert_eq!(quota_level(70), QuotaLevel::Warning);
        assert_eq!(quota_level(90), QuotaLevel::Critical);

        let quota = |window, used_percent, resets_at_ms| AgentQuota {
            window,
            used_percent,
            resets_at_ms,
        };
        let claude = AgentQuotaSnapshot {
            quotas: vec![
                quota(AgentQuotaWindow::FiveHours, 93, Some(now + 60_000)),
                quota(AgentQuotaWindow::SevenDays, 40, None),
            ],
            observed_at_ms: now,
        };
        let codex = AgentQuotaSnapshot {
            quotas: vec![
                quota(AgentQuotaWindow::SevenDays, 98, Some(now + 86_400_000)),
                quota(AgentQuotaWindow::FiveHours, 99, Some(now - 1)),
            ],
            observed_at_ms: now,
        };
        // A window past its reset is ignored even when it reads higher.
        assert!(!quota_is_current(&codex.quotas[1], now));
        assert_eq!(
            quota_alert(
                [
                    (AgentProvider::Claude, &claude),
                    (AgentProvider::Codex, &codex)
                ],
                now
            ),
            Some((AgentProvider::Codex, codex.quotas[0]))
        );
        let below = AgentQuotaSnapshot {
            quotas: vec![quota(AgentQuotaWindow::SevenDays, 89, None)],
            observed_at_ms: now,
        };
        assert_eq!(quota_alert([(AgentProvider::Claude, &below)], now), None);
    }
}
