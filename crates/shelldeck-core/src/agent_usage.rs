//! Token, cost and account quota reports from agent providers.
//!
//! Claude reports each invocation's tokens and cost in its final `result`
//! record and the account windows in `rate_limit_event`. Codex reports the
//! thread's running token total in `turn.completed`; its account windows are
//! written only to the local session journal under `$CODEX_HOME/sessions`.
//! Nothing here estimates: a figure a provider did not report stays absent.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Day directories of the Codex journal searched for the latest quotas.
const MAX_JOURNAL_DAYS: usize = 14;
/// Entries of one day directory considered as journals.
const MAX_JOURNAL_ENTRIES_PER_DAY: usize = 256;
/// Most recently modified journals opened before giving up.
const MAX_JOURNALS_OPENED: usize = 8;
/// Tail of a journal read when looking for its last quota record.
const JOURNAL_TAIL_BYTES: u64 = 512 * 1024;

/// Tokens processed by a provider. `input` and `output` are totals; the other
/// fields are the parts of the input read from or written to a prompt cache
/// and the reasoning part of the output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTokenUsage {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub cached_input: u64,
    #[serde(default)]
    pub cache_write: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub reasoning: u64,
}

impl AgentTokenUsage {
    #[must_use]
    pub fn total(&self) -> u64 {
        self.input.saturating_add(self.output)
    }

    #[must_use]
    pub fn saturating_add(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_add(other.input),
            cached_input: self.cached_input.saturating_add(other.cached_input),
            cache_write: self.cache_write.saturating_add(other.cache_write),
            output: self.output.saturating_add(other.output),
            reasoning: self.reasoning.saturating_add(other.reasoning),
        }
    }
}

/// Account usage window. Providers name and order them differently, so a
/// window is identified by its length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentQuotaWindow {
    FiveHours,
    SevenDays,
}

impl AgentQuotaWindow {
    #[must_use]
    pub fn from_minutes(minutes: u64) -> Option<Self> {
        match minutes {
            300 => Some(Self::FiveHours),
            10_080 => Some(Self::SevenDays),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentQuota {
    pub window: AgentQuotaWindow,
    /// Share of the window already used, rounded and capped at 100.
    pub used_percent: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<i64>,
}

/// One usage record extracted from a provider stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentUsageReport {
    pub tokens: Option<AgentTokenUsage>,
    /// `tokens` is the provider thread's running total, not this run's share.
    pub cumulative: bool,
    pub cost_micro_usd: Option<u64>,
    pub quotas: Vec<AgentQuota>,
}

impl AgentUsageReport {
    #[must_use]
    pub fn quotas(quotas: Vec<AgentQuota>) -> Self {
        Self {
            quotas,
            ..Self::default()
        }
    }
}

/// Tokens and cost of one Claude invocation, from its final `result` record.
#[must_use]
pub fn claude_result_usage(value: &Value) -> Option<AgentUsageReport> {
    if value.get("type").and_then(Value::as_str) != Some("result") {
        return None;
    }
    let tokens = value
        .get("usage")
        .filter(|usage| usage.is_object())
        .map(|usage| {
            let field = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
            let cached_input = field("cache_read_input_tokens");
            let cache_write = field("cache_creation_input_tokens");
            AgentTokenUsage {
                // Claude counts cache reads and writes outside `input_tokens`.
                input: field("input_tokens")
                    .saturating_add(cached_input)
                    .saturating_add(cache_write),
                cached_input,
                cache_write,
                output: field("output_tokens"),
                reasoning: usage
                    .pointer("/output_tokens_details/thinking_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            }
        });
    let cost_micro_usd = value
        .get("total_cost_usd")
        .and_then(Value::as_f64)
        .and_then(micro_usd);
    (tokens.is_some() || cost_micro_usd.is_some()).then_some(AgentUsageReport {
        tokens,
        cumulative: false,
        cost_micro_usd,
        quotas: Vec::new(),
    })
}

/// Account windows from a Claude `rate_limit_event`.
#[must_use]
pub fn claude_rate_limit_quotas(value: &Value) -> Vec<AgentQuota> {
    if value.get("type").and_then(Value::as_str) != Some("rate_limit_event") {
        return Vec::new();
    }
    let Some(info) = value.get("rate_limit_info") else {
        return Vec::new();
    };
    let mut quotas = Vec::new();
    if let Some(windows) = info.get("unifiedWindows").and_then(Value::as_object) {
        for (key, window) in [
            ("five_hour", AgentQuotaWindow::FiveHours),
            ("seven_day", AgentQuotaWindow::SevenDays),
        ] {
            if let Some(entry) = windows.get(key) {
                push_claude_window(&mut quotas, window, entry);
            }
        }
    } else if let Some(window) = info
        .get("rateLimitType")
        .and_then(Value::as_str)
        .and_then(claude_window)
    {
        push_claude_window(&mut quotas, window, info);
    }
    quotas
}

fn claude_window(name: &str) -> Option<AgentQuotaWindow> {
    match name {
        "five_hour" => Some(AgentQuotaWindow::FiveHours),
        "seven_day" => Some(AgentQuotaWindow::SevenDays),
        _ => None,
    }
}

fn push_claude_window(quotas: &mut Vec<AgentQuota>, window: AgentQuotaWindow, entry: &Value) {
    // Claude reports the used share as a fraction of the window.
    let Some(used_percent) = entry
        .get("utilization")
        .and_then(Value::as_f64)
        .and_then(|fraction| percent(fraction * 100.0))
    else {
        return;
    };
    quotas.push(AgentQuota {
        window,
        used_percent,
        resets_at_ms: entry
            .get("resetsAt")
            .and_then(Value::as_i64)
            .and_then(seconds_to_ms),
    });
}

/// The Codex thread's running token total from `turn.completed`.
#[must_use]
pub fn codex_turn_usage(value: &Value) -> Option<AgentUsageReport> {
    if value.get("type").and_then(Value::as_str) != Some("turn.completed") {
        return None;
    }
    let usage = value.get("usage").filter(|usage| usage.is_object())?;
    let field = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    Some(AgentUsageReport {
        tokens: Some(AgentTokenUsage {
            input: field("input_tokens"),
            cached_input: field("cached_input_tokens"),
            cache_write: field("cache_write_input_tokens"),
            output: field("output_tokens"),
            reasoning: field("reasoning_output_tokens"),
        }),
        cumulative: true,
        cost_micro_usd: None,
        quotas: Vec::new(),
    })
}

/// Windows of a Codex `rate_limits` object. `primary` and `secondary` hold
/// whichever windows the plan has, so each is classified by its length.
#[must_use]
pub fn codex_rate_limit_quotas(rate_limits: &Value) -> Vec<AgentQuota> {
    let mut quotas: Vec<AgentQuota> = ["primary", "secondary"]
        .into_iter()
        .filter_map(|slot| rate_limits.get(slot))
        .filter_map(|entry| {
            let window = entry
                .get("window_minutes")
                .and_then(Value::as_u64)
                .and_then(AgentQuotaWindow::from_minutes)?;
            let used_percent = entry
                .get("used_percent")
                .and_then(Value::as_f64)
                .and_then(percent)?;
            Some(AgentQuota {
                window,
                used_percent,
                resets_at_ms: entry
                    .get("resets_at")
                    .and_then(Value::as_i64)
                    .and_then(seconds_to_ms),
            })
        })
        .collect();
    quotas.sort_by_key(|quota| quota.window);
    quotas.dedup_by_key(|quota| quota.window);
    quotas
}

/// Windows from the last quota record of a Codex session journal.
#[must_use]
pub fn codex_journal_quotas(journal: &str) -> Option<Vec<AgentQuota>> {
    journal.lines().rev().find_map(|line| {
        if !line.contains("\"rate_limits\"") {
            return None;
        }
        let value: Value = serde_json::from_str(line.trim()).ok()?;
        let payload = value.get("payload")?;
        if payload.get("type").and_then(Value::as_str) != Some("token_count") {
            return None;
        }
        let rate_limits = payload.get("rate_limits")?;
        // Codex also journals model-specific buckets (`codex_bengalfox` for
        // GPT-5.3-Codex-Spark, for example); only the account-wide `codex`
        // limit describes Codex usage. Older records carry no identifier.
        if rate_limits
            .get("limit_id")
            .and_then(Value::as_str)
            .is_some_and(|id| id != "codex")
        {
            return None;
        }
        let quotas = codex_rate_limit_quotas(rate_limits);
        (!quotas.is_empty()).then_some(quotas)
    })
}

/// `$CODEX_HOME`, or `.codex` in the home directory.
#[must_use]
pub fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::util::home_dir().map(|home| home.join(".codex")))
}

/// Latest Codex account windows recorded in the local journal. The search is
/// bounded: recent day directories, the most recently modified journals (a
/// resumed conversation keeps writing to its original day), and the tail of
/// each file.
#[must_use]
pub fn read_codex_quotas(codex_home: &Path) -> Option<Vec<AgentQuota>> {
    let mut journals = recent_codex_journals(&codex_home.join("sessions"));
    journals.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    journals
        .into_iter()
        .take(MAX_JOURNALS_OPENED)
        .find_map(|(_, path)| {
            read_tail(&path, JOURNAL_TAIL_BYTES).and_then(|text| codex_journal_quotas(&text))
        })
}

fn recent_codex_journals(sessions: &Path) -> Vec<(SystemTime, PathBuf)> {
    let mut journals = Vec::new();
    let mut days = 0;
    'search: for year in numeric_dirs_newest_first(sessions) {
        for month in numeric_dirs_newest_first(&year) {
            for day in numeric_dirs_newest_first(&month) {
                if days == MAX_JOURNAL_DAYS {
                    break 'search;
                }
                days += 1;
                let Ok(entries) = std::fs::read_dir(&day) else {
                    continue;
                };
                for entry in entries.flatten().take(MAX_JOURNAL_ENTRIES_PER_DAY) {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if !(name.starts_with("rollout-") && name.ends_with(".jsonl")) {
                        continue;
                    }
                    let Ok(metadata) = entry.metadata() else {
                        continue;
                    };
                    if metadata.is_file() {
                        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                        journals.push((modified, entry.path()));
                    }
                }
            }
        }
    }
    journals
}

/// `YYYY`, `MM` and `DD` directories are zero-padded, so a reverse name order
/// is newest first.
fn numeric_dirs_newest_first(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(|entry| entry.path())
        .collect();
    dirs.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
    dirs
}

fn read_tail(path: &Path, max_bytes: u64) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let start = file.metadata().ok()?.len().saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(max_bytes).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    // A tail can start inside a record; drop that fragment.
    Some(if start > 0 {
        text.split_once('\n')
            .map_or_else(String::new, |(_, rest)| rest.to_string())
    } else {
        text.into_owned()
    })
}

fn percent(value: f64) -> Option<u8> {
    value
        .is_finite()
        .then(|| value.round().clamp(0.0, 100.0) as u8)
}

fn micro_usd(cost: f64) -> Option<u64> {
    (cost.is_finite() && cost >= 0.0).then(|| (cost * 1_000_000.0).round() as u64)
}

fn seconds_to_ms(seconds: i64) -> Option<i64> {
    (seconds > 0).then(|| seconds.saturating_mul(1_000))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_count(rate_limits: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-09-10T12:52:26.000Z","type":"event_msg","payload":{{"type":"token_count","info":null,"rate_limits":{rate_limits}}}}}"#
        )
    }

    const BOTH_WINDOWS: &str = r#"{"limit_id":"codex","primary":{"used_percent":12.4,"window_minutes":300,"resets_at":1789000000},"secondary":{"used_percent":40.6,"window_minutes":10080,"resets_at":1789500000}}"#;
    // A weekly-only plan reports its seven-day window in the primary slot.
    const WEEKLY_ONLY: &str = r#"{"limit_id":"codex","primary":{"used_percent":98.0,"window_minutes":10080,"resets_at":1789505917},"secondary":null}"#;
    // A model-specific bucket journaled alongside the account-wide limit.
    const SPARK_BUCKET: &str = r#"{"limit_id":"codex_bengalfox","limit_name":"GPT-5.3-Codex-Spark","primary":{"used_percent":0.0,"window_minutes":300,"resets_at":1789068238},"secondary":{"used_percent":0.0,"window_minutes":10080,"resets_at":1789655038}}"#;

    fn both_windows() -> Vec<AgentQuota> {
        vec![
            AgentQuota {
                window: AgentQuotaWindow::FiveHours,
                used_percent: 12,
                resets_at_ms: Some(1_789_000_000_000),
            },
            AgentQuota {
                window: AgentQuotaWindow::SevenDays,
                used_percent: 41,
                resets_at_ms: Some(1_789_500_000_000),
            },
        ]
    }

    #[test]
    fn sdtest_1933_codex_journal_quotas_follow_window_length_and_latest_journal() {
        let weekly = vec![AgentQuota {
            window: AgentQuotaWindow::SevenDays,
            used_percent: 98,
            resets_at_ms: Some(1_789_505_917_000),
        }];
        let journal = [
            token_count(BOTH_WINDOWS),
            token_count(WEEKLY_ONLY),
            token_count(SPARK_BUCKET),
            token_count(r#"{"primary":null,"secondary":null}"#),
            token_count(r#"{"primary":{"used_percent":3.0,"window_minutes":60}}"#),
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"ok"}}"#.to_string(),
        ]
        .join("\n");
        // A model-specific bucket and records without a recognised window are
        // skipped, not read as the account's usage.
        assert_eq!(codex_journal_quotas(&journal), Some(weekly));
        assert_eq!(
            codex_journal_quotas(&token_count(BOTH_WINDOWS)),
            Some(both_windows())
        );
        assert_eq!(codex_journal_quotas(&token_count(SPARK_BUCKET)), None);
        assert_eq!(codex_journal_quotas("not json\n{}"), None);

        let home = std::env::temp_dir().join(format!("shelldeck-codex-{}", uuid::Uuid::new_v4()));
        let old_day = home.join("sessions/2026/08/27");
        let new_day = home.join("sessions/2026/09/10");
        std::fs::create_dir_all(&old_day).unwrap();
        std::fs::create_dir_all(&new_day).unwrap();
        let write = |path: PathBuf, text: String, age_s: u64| {
            std::fs::write(&path, text).unwrap();
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(SystemTime::now() - std::time::Duration::from_secs(age_s))
                .unwrap();
        };
        // An older conversation resumed recently holds the freshest
        // account-wide windows; newer journals hold only a model-specific
        // bucket or nothing at all (an interrupted run).
        write(
            new_day.join("rollout-2026-09-10T13-00-00-spark.jsonl"),
            token_count(SPARK_BUCKET),
            5,
        );
        write(
            old_day.join("rollout-2026-08-27T11-44-46-resumed.jsonl"),
            token_count(BOTH_WINDOWS),
            60,
        );
        write(
            new_day.join("rollout-2026-09-10T09-00-00-older.jsonl"),
            token_count(WEEKLY_ONLY),
            3_600,
        );
        write(
            new_day.join("rollout-2026-09-10T14-00-00-aborted.jsonl"),
            r#"{"type":"session_meta","payload":{}}"#.to_string(),
            1,
        );
        write(
            new_day.join("notes.jsonl"),
            token_count(r#"{"primary":{"used_percent":1.0,"window_minutes":300}}"#),
            0,
        );
        assert_eq!(read_codex_quotas(&home), Some(both_windows()));
        std::fs::remove_dir_all(&home).unwrap();
        assert_eq!(read_codex_quotas(&home), None);
    }
}
