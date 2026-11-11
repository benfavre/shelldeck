use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::error::{Result, ShellDeckError};

/// Serialized workspace state for session persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub tabs: Vec<TabState>,
    pub active_tab: usize,
    pub sidebar_visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabState {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub tab_type: TabType,
    /// For SSH tabs: the connection ID to reconnect.
    pub connection_id: Option<Uuid>,
    /// For local tabs: the shell to use.
    pub shell: Option<String>,
    /// For local tabs made of session daemon panes
    /// (`general.session_daemon`): the native layout, keyed by daemon pane
    /// id, so the tab can be rebuilt around panes that are still running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon: Option<DaemonTabLayout>,
}

/// Native layout of one tab whose panes belong to the session daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonTabLayout {
    pub tree: DaemonPaneNode,
    /// Pane that had keyboard focus.
    #[serde(default)]
    pub focused: Option<Uuid>,
    /// The tab title was chosen deliberately (not a program title).
    #[serde(default)]
    pub named: bool,
}

/// Split tree of a daemon tab. A leaf is a daemon pane id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DaemonPaneNode {
    Pane {
        pane: Uuid,
    },
    Split {
        /// Side by side (`true`) or stacked.
        horizontal: bool,
        /// Share of the first child.
        ratio: f32,
        first: Box<DaemonPaneNode>,
        second: Box<DaemonPaneNode>,
    },
}

/// Bounds of a saved split ratio, the same as a divider drag allows.
const MIN_SPLIT_RATIO: f32 = 0.15;
const MAX_SPLIT_RATIO: f32 = 0.85;

impl DaemonPaneNode {
    /// A split of two optional subtrees: with one side missing it is the
    /// other side, with both missing it is nothing. Used to save a tab
    /// some of whose panes are not daemon panes, and to drop dead panes.
    pub fn split(
        horizontal: bool,
        ratio: f32,
        first: Option<Self>,
        second: Option<Self>,
    ) -> Option<Self> {
        match (first, second) {
            (Some(first), Some(second)) => Some(Self::Split {
                horizontal,
                ratio: if ratio.is_finite() {
                    ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO)
                } else {
                    0.5
                },
                first: Box::new(first),
                second: Box::new(second),
            }),
            (Some(only), None) | (None, Some(only)) => Some(only),
            (None, None) => None,
        }
    }

    /// Pane ids, left to right and top to bottom.
    pub fn panes(&self) -> Vec<Uuid> {
        fn walk(node: &DaemonPaneNode, out: &mut Vec<Uuid>) {
            match node {
                DaemonPaneNode::Pane { pane } => out.push(*pane),
                DaemonPaneNode::Split { first, second, .. } => {
                    walk(first, out);
                    walk(second, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(self, &mut out);
        out
    }

    /// The tree without the panes `keep` refuses: a split that loses one
    /// side becomes its other side. `None` when no pane is left.
    pub fn retain(&self, keep: &mut impl FnMut(Uuid) -> bool) -> Option<Self> {
        match self {
            Self::Pane { pane } => keep(*pane).then(|| self.clone()),
            Self::Split {
                horizontal,
                ratio,
                first,
                second,
            } => {
                let first = first.retain(keep);
                let second = second.retain(keep);
                Self::split(*horizontal, *ratio, first, second)
            }
        }
    }
}

/// One tab to open at startup.
#[derive(Debug, Clone, PartialEq)]
pub enum RestoredTab {
    /// Rebuild a tab around daemon panes that are still running. `title`
    /// is `None` for a pane that had no saved slot.
    Daemon {
        title: Option<String>,
        layout: DaemonTabLayout,
    },
    /// The saved tab at this index of [`WorkspaceState::tabs`]: a local
    /// tab without daemon panes, or an SSH tab.
    Saved(usize),
}

/// What to open at startup, in tab order.
#[derive(Debug, Clone, PartialEq)]
pub struct RestorePlan {
    pub tabs: Vec<RestoredTab>,
    /// Index into `tabs` of the tab that was active, if it survived.
    pub active: Option<usize>,
}

impl WorkspaceState {
    /// Match the saved layout against the daemon panes that are still
    /// alive (`live`, in the daemon's order):
    ///
    /// - a saved daemon tab keeps its place, its split tree and ratios for
    ///   the panes that are alive; panes that are gone are dropped (the
    ///   split collapses into its other side) and a tab with none left is
    ///   dropped;
    /// - a live pane with no saved slot comes back as a tab of its own,
    ///   after the saved ones;
    /// - a pane saved twice (damaged file) is used once.
    pub fn daemon_restore_plan(&self, live: &[Uuid]) -> RestorePlan {
        let mut placed = std::collections::HashSet::new();
        let mut tabs = Vec::new();
        let mut active = None;
        for (index, tab) in self.tabs.iter().enumerate() {
            let restored = match &tab.daemon {
                Some(saved) => {
                    let tree = saved
                        .tree
                        .retain(&mut |pane| live.contains(&pane) && placed.insert(pane));
                    let Some(tree) = tree else { continue };
                    let panes = tree.panes();
                    RestoredTab::Daemon {
                        title: Some(tab.title.clone()),
                        layout: DaemonTabLayout {
                            focused: saved.focused.filter(|pane| panes.contains(pane)),
                            named: saved.named,
                            tree,
                        },
                    }
                }
                None => RestoredTab::Saved(index),
            };
            if index == self.active_tab {
                active = Some(tabs.len());
            }
            tabs.push(restored);
        }
        for pane in live {
            if placed.insert(*pane) {
                tabs.push(RestoredTab::Daemon {
                    title: None,
                    layout: DaemonTabLayout {
                        tree: DaemonPaneNode::Pane { pane: *pane },
                        focused: None,
                        named: false,
                    },
                });
            }
        }
        RestorePlan { tabs, active }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TabType {
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "ssh")]
    Ssh,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: 0,
            sidebar_visible: true,
        }
    }
}

impl WorkspaceState {
    /// Get the state file path.
    fn state_path() -> PathBuf {
        super::app_config::AppConfig::config_dir().join("workspace.json")
    }

    /// Save workspace state to disk.
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::state_path())
    }

    /// Load workspace state from disk.
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::state_path())
    }

    /// Delete the saved state (used after successful restore).
    pub fn clear() -> Result<()> {
        Self::clear_at(&Self::state_path())
    }

    /// Save workspace state to a specific path atomically.
    pub(crate) fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() && !dir.exists() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let content = serde_json::to_string_pretty(self).map_err(|e| {
            ShellDeckError::Serialization(format!("Failed to serialize workspace state: {}", e))
        })?;
        crate::util::atomic_write(path, content.as_bytes())?;
        Ok(())
    }

    /// Load workspace state from a specific path, returning defaults if missing.
    pub(crate) fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let state: Self = serde_json::from_str(&content).map_err(|e| {
            ShellDeckError::Serialization(format!("Failed to parse workspace state: {}", e))
        })?;
        Ok(state)
    }

    /// Delete the saved state at a specific path.
    pub(crate) fn clear_at(path: &Path) -> Result<()> {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "shelldeck-wsstate-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.join(name)
    }

    #[test]
    fn round_trip_with_tabs() {
        let path = temp_path("workspace.json");

        let conn_id = Uuid::new_v4();
        let state = WorkspaceState {
            tabs: vec![
                TabState {
                    id: "tab-1".to_string(),
                    title: "Local Shell".to_string(),
                    tab_type: TabType::Local,
                    connection_id: None,
                    shell: Some("/bin/bash".to_string()),
                    daemon: None,
                },
                TabState {
                    id: "tab-2".to_string(),
                    title: "prod ssh".to_string(),
                    tab_type: TabType::Ssh,
                    connection_id: Some(conn_id),
                    shell: None,
                    daemon: None,
                },
            ],
            active_tab: 1,
            sidebar_visible: false,
        };

        state.save_to(&path).expect("save_to");
        let loaded = WorkspaceState::load_from(&path).expect("load_from");

        assert_eq!(loaded.tabs.len(), 2);
        assert_eq!(loaded.active_tab, 1);
        assert!(!loaded.sidebar_visible);
        assert_eq!(loaded.tabs[0].id, "tab-1");
        assert!(matches!(loaded.tabs[0].tab_type, TabType::Local));
        assert_eq!(loaded.tabs[0].shell.as_deref(), Some("/bin/bash"));
        assert!(matches!(loaded.tabs[1].tab_type, TabType::Ssh));
        assert_eq!(loaded.tabs[1].connection_id, Some(conn_id));

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn load_from_missing_returns_default() {
        let path = temp_path("workspace.json");
        assert!(!path.exists());

        let loaded = WorkspaceState::load_from(&path).expect("load_from");

        assert!(loaded.tabs.is_empty());
        assert_eq!(loaded.active_tab, 0);
        assert!(loaded.sidebar_visible);
        // Unlike config/store, missing workspace state does NOT create a file.
        assert!(!path.exists());

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn clear_at_removes_file() {
        let path = temp_path("workspace.json");
        WorkspaceState::default().save_to(&path).expect("save_to");
        assert!(path.exists());

        WorkspaceState::clear_at(&path).expect("clear_at");
        assert!(!path.exists());
        // Clearing a missing file is a no-op, not an error.
        WorkspaceState::clear_at(&path).expect("clear_at idempotent");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn load_from_corrupt_returns_err() {
        let path = temp_path("workspace.json");
        std::fs::write(&path, b"not json at all <<<>>>").expect("seed garbage");

        let result = WorkspaceState::load_from(&path);
        assert!(result.is_err(), "corrupt state should error, not panic");

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    fn pane(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn leaf(n: u128) -> DaemonPaneNode {
        DaemonPaneNode::Pane { pane: pane(n) }
    }

    fn split(horizontal: bool, ratio: f32, a: DaemonPaneNode, b: DaemonPaneNode) -> DaemonPaneNode {
        DaemonPaneNode::Split {
            horizontal,
            ratio,
            first: Box::new(a),
            second: Box::new(b),
        }
    }

    fn daemon_tab(title: &str, tree: DaemonPaneNode, focused: Option<u128>) -> TabState {
        TabState {
            id: title.to_string(),
            title: title.to_string(),
            tab_type: TabType::Local,
            connection_id: None,
            shell: None,
            daemon: Some(DaemonTabLayout {
                tree,
                focused: focused.map(pane),
                named: false,
            }),
        }
    }

    fn plain_tab(title: &str, tab_type: TabType) -> TabState {
        TabState {
            id: title.to_string(),
            title: title.to_string(),
            tab_type,
            connection_id: None,
            shell: None,
            daemon: None,
        }
    }

    // SDTEST-3008 (SDUC-574): the daemon layout is part of the session
    // file. A file written before the field existed still loads, and a
    // tab without daemon panes writes no `daemon` key.
    #[test]
    fn sdtest_3008_daemon_layout_round_trips_and_old_files_still_load() {
        let path = temp_path("workspace.json");
        let state = WorkspaceState {
            tabs: vec![
                daemon_tab(
                    "build",
                    split(true, 0.3, leaf(1), split(false, 0.6, leaf(2), leaf(3))),
                    Some(3),
                ),
                plain_tab("prod", TabType::Ssh),
            ],
            active_tab: 0,
            sidebar_visible: true,
        };
        state.save_to(&path).expect("save_to");
        let text = std::fs::read_to_string(&path).expect("read");
        assert_eq!(text.matches("\"daemon\"").count(), 1, "{text}");
        let loaded = WorkspaceState::load_from(&path).expect("load_from");
        assert_eq!(loaded.tabs[0].daemon, state.tabs[0].daemon);
        assert_eq!(loaded.tabs[1].daemon, None);

        let old = r#"{"tabs":[{"id":"0","title":"Terminal","type":"local",
            "connection_id":null,"shell":null}],"active_tab":0,"sidebar_visible":true}"#;
        std::fs::write(&path, old).expect("write old file");
        let loaded = WorkspaceState::load_from(&path).expect("old file loads");
        assert_eq!(loaded.tabs[0].daemon, None);

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    // SDTEST-3009 (SDUC-574): what comes back at startup for a saved
    // layout and the panes that are still alive.
    #[test]
    fn sdtest_3009_restore_plan_keeps_live_panes_and_drops_dead_slots() {
        let state = WorkspaceState {
            tabs: vec![
                // 1 | (2 / 3), focus on 3.
                daemon_tab(
                    "build",
                    split(true, 0.3, leaf(1), split(false, 0.6, leaf(2), leaf(3))),
                    Some(3),
                ),
                plain_tab("prod", TabType::Ssh),
                // Every pane of this tab is gone.
                daemon_tab("gone", split(true, 0.5, leaf(4), leaf(5)), Some(4)),
                // Was active; its focused pane is gone.
                daemon_tab("logs", split(false, 0.7, leaf(6), leaf(7)), Some(7)),
                plain_tab("local", TabType::Local),
            ],
            active_tab: 3,
            sidebar_visible: true,
        };

        // Everything alive: the layout comes back as saved, in place.
        let all: Vec<Uuid> = (1..=7).map(pane).collect();
        let plan = state.daemon_restore_plan(&all);
        assert_eq!(plan.tabs.len(), 5);
        assert_eq!(plan.active, Some(3));
        let RestoredTab::Daemon { title, layout } = &plan.tabs[0] else {
            panic!("first tab is a daemon tab");
        };
        assert_eq!(title.as_deref(), Some("build"));
        assert_eq!(Some(layout), state.tabs[0].daemon.as_ref());
        assert_eq!(plan.tabs[1], RestoredTab::Saved(1));
        assert_eq!(plan.tabs[4], RestoredTab::Saved(4));

        // Panes 2, 4, 5 and 7 died; 8 and 9 are alive but were never
        // saved (the app was killed before it could save).
        let live = [pane(9), pane(1), pane(3), pane(6), pane(8)];
        let plan = state.daemon_restore_plan(&live);
        let layouts: Vec<Option<&DaemonTabLayout>> = plan
            .tabs
            .iter()
            .map(|tab| match tab {
                RestoredTab::Daemon { layout, .. } => Some(layout),
                RestoredTab::Saved(_) => None,
            })
            .collect();
        assert_eq!(plan.tabs.len(), 6);
        // The inner split lost pane 2 and collapsed; the outer split and
        // its ratio are kept; focus stays on pane 3.
        let build = layouts[0].expect("build");
        assert_eq!(build.tree, split(true, 0.3, leaf(1), leaf(3)));
        assert_eq!(build.focused, Some(pane(3)));
        assert_eq!(plan.tabs[1], RestoredTab::Saved(1));
        // "gone" is dropped; "logs" is a single pane, focus forgotten.
        let logs = layouts[2].expect("logs");
        assert_eq!(logs.tree, leaf(6));
        assert_eq!(logs.focused, None);
        assert_eq!(plan.active, Some(2), "the active tab moved up one place");
        assert_eq!(plan.tabs[3], RestoredTab::Saved(4));
        // Panes without a slot: one tab each, after the saved ones, in
        // the daemon's order, without a saved title.
        assert_eq!(
            plan.tabs[4..],
            [9, 8].map(|n| RestoredTab::Daemon {
                title: None,
                layout: DaemonTabLayout {
                    tree: leaf(n),
                    focused: None,
                    named: false,
                },
            })
        );

        // The active tab did not survive: no tab is forced active.
        let plan = WorkspaceState {
            active_tab: 2,
            ..state.clone()
        }
        .daemon_restore_plan(&live);
        assert_eq!(plan.active, None);

        // No daemon pane alive: only the other saved tabs remain.
        let plan = state.daemon_restore_plan(&[]);
        assert_eq!(
            plan.tabs,
            vec![RestoredTab::Saved(1), RestoredTab::Saved(4)]
        );
    }

    // SDTEST-3010 (SDUC-574): a damaged session file cannot show one pane
    // twice or produce an unusable split.
    #[test]
    fn sdtest_3010_restore_plan_survives_a_damaged_layout() {
        let state = WorkspaceState {
            tabs: vec![
                daemon_tab("a", split(true, f32::NAN, leaf(1), leaf(2)), Some(1)),
                // Pane 1 again, and a ratio far out of range.
                daemon_tab("b", split(false, 40.0, leaf(1), leaf(3)), Some(1)),
                daemon_tab("c", split(true, -1.0, leaf(4), leaf(4)), None),
            ],
            active_tab: 99,
            sidebar_visible: true,
        };
        let plan = state.daemon_restore_plan(&[pane(1), pane(2), pane(3), pane(4)]);
        let trees: Vec<DaemonPaneNode> = plan
            .tabs
            .iter()
            .map(|tab| match tab {
                RestoredTab::Daemon { layout, .. } => layout.tree.clone(),
                RestoredTab::Saved(_) => panic!("only daemon tabs here"),
            })
            .collect();
        assert_eq!(
            trees,
            vec![split(true, 0.5, leaf(1), leaf(2)), leaf(3), leaf(4)]
        );
        assert_eq!(plan.active, None);
        let RestoredTab::Daemon { layout, .. } = &plan.tabs[1] else {
            unreachable!()
        };
        assert_eq!(layout.focused, None, "focus on a pane shown elsewhere");

        // Both sides alive with an out of range ratio: clamped.
        let state = WorkspaceState {
            tabs: vec![daemon_tab("b", split(false, 40.0, leaf(1), leaf(3)), None)],
            active_tab: 0,
            sidebar_visible: true,
        };
        let plan = state.daemon_restore_plan(&[pane(1), pane(3)]);
        let RestoredTab::Daemon { layout, .. } = &plan.tabs[0] else {
            unreachable!()
        };
        assert_eq!(layout.tree, split(false, 0.85, leaf(1), leaf(3)));
    }
}
