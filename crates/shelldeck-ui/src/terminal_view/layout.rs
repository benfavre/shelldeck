use super::*;
use crate::view_attention::ViewAttention;
use shelldeck_core::config::workspace_state::DaemonPaneNode;
use shelldeck_terminal::attached::AttachedPhase;
use shelldeck_terminal::program_status::{BlockedKind, ProgramHeadline, ProgramState};

/// A single tab in the terminal tab bar
#[derive(Debug, Clone)]
pub struct TerminalTab {
    pub id: Uuid,
    pub title: String,
    pub is_active: bool,
    pub state: SessionState,
    pub zoom_level: f32,
    /// The connection ID this tab is associated with, if any (None for local terminals).
    pub connection_id: Option<Uuid>,
    /// Title the running program set through OSC 0 / OSC 2 (shell prompt,
    /// vim, ssh, Claude Code...). Empty until a program sets one.
    pub program_title: String,
    /// The tab carries a deliberate name (AI naming) that program titles must
    /// not override.
    pub named: bool,
    /// What the program in the tab says it is doing (OSC 7501): the most
    /// urgent report among the tab's panes. `None` until a program reports.
    pub status: Option<ProgramHeadline>,
    /// The user has had this tab in front since `status` last changed.
    pub status_seen: bool,
}

/// How the tab dot shows a reported status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusTone {
    /// Nothing to add to the session state (no report, idle, done and seen).
    Plain,
    Working,
    /// Finished and not looked at yet.
    Finished,
    NeedsYou,
    Failed,
}

/// The most urgent of several reports; the first wins between equals.
pub fn most_urgent(
    headlines: impl IntoIterator<Item = Option<ProgramHeadline>>,
) -> Option<ProgramHeadline> {
    let mut best: Option<ProgramHeadline> = None;
    for headline in headlines.into_iter().flatten() {
        if best
            .as_ref()
            .is_none_or(|current| headline.state.urgency() > current.state.urgency())
        {
            best = Some(headline);
        }
    }
    best
}

/// One line for the tab tooltip: who, what, and the program's own words.
pub fn status_tooltip(status: &ProgramHeadline) -> String {
    let what = match (status.state, status.kind) {
        (ProgramState::Idle, _) => t!("terminal.status.idle").to_string(),
        (ProgramState::Working, _) => match status.progress {
            Some(percent) => t!("terminal.status.working_percent", percent = percent).to_string(),
            None => t!("terminal.status.working").to_string(),
        },
        (ProgramState::Done, _) => t!("terminal.status.done").to_string(),
        (ProgramState::Error, _) => t!("terminal.status.error").to_string(),
        (ProgramState::Blocked, Some(BlockedKind::Permission)) => {
            t!("terminal.status.blocked_permission").to_string()
        }
        (ProgramState::Blocked, Some(BlockedKind::Auth)) => {
            t!("terminal.status.blocked_auth").to_string()
        }
        (ProgramState::Blocked, _) => t!("terminal.status.blocked").to_string(),
    };
    let mut line = if status.app.is_empty() {
        what
    } else {
        format!("{} · {what}", status.app)
    };
    if !status.text.is_empty() {
        line.push_str(" · ");
        line.push_str(&status.text);
    }
    line
}

impl TerminalTab {
    /// Label shown in the tab strip: the live program title, like other
    /// terminal emulators, unless the tab was deliberately named.
    /// Record what the tab's programs now report. A change is news: the
    /// tab counts as not looked at again. Returns whether anything changed.
    pub fn set_status(&mut self, status: Option<ProgramHeadline>) -> bool {
        if self.status == status {
            return false;
        }
        self.status = status;
        self.status_seen = false;
        true
    }

    pub fn status_tone(&self) -> StatusTone {
        match self.status.as_ref().map(|status| status.state) {
            Some(ProgramState::Blocked) => StatusTone::NeedsYou,
            Some(ProgramState::Error) => StatusTone::Failed,
            Some(ProgramState::Working) => StatusTone::Working,
            Some(ProgramState::Done) if !self.status_seen => StatusTone::Finished,
            Some(ProgramState::Done | ProgramState::Idle) | None => StatusTone::Plain,
        }
    }

    /// What the tab wants from a user who is looking elsewhere. A blocked
    /// or failed program keeps asking until its state changes; finished
    /// work stops asking once the tab has been in front.
    pub fn attention(&self) -> ViewAttention {
        match self.status_tone() {
            StatusTone::NeedsYou | StatusTone::Failed => ViewAttention::NeedsYou,
            StatusTone::Finished => ViewAttention::Finished,
            StatusTone::Working | StatusTone::Plain => ViewAttention::None,
        }
    }

    pub fn display_title(&self) -> &str {
        let program = self.program_title.trim();
        if self.named || program.is_empty() {
            &self.title
        } else {
            program
        }
    }
}

/// Terminal pane holding sessions
pub struct TerminalPane {
    pub sessions: Vec<TerminalSession>,
    pub active_index: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

/// Identifies one leaf pane within a tab's layout tree. `Primary` is the tab's
/// session stored in `pane.sessions[active_index]`; `Extra(id)` is a split
/// session stored in [`TabLayout::extra`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum PaneId {
    Primary,
    Extra(Uuid),
}

/// A node in a tab's recursive pane layout. Leaves reference one session; an
/// internal `Split` divides its area between two children at `ratio`.
pub(super) enum PaneNode {
    Leaf(PaneId),
    Split {
        direction: SplitDirection,
        /// Fraction of the parent given to child `a` (left/top). `b` gets the rest.
        ratio: f32,
        a: Box<PaneNode>,
        b: Box<PaneNode>,
    },
}

/// A rectangle in absolute window pixels.
#[derive(Debug, Clone, Copy)]
pub(super) struct PaneRect {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
}

/// A divider between two children, with the tree path needed to adjust its
/// ratio during a drag.
pub(super) struct DividerRect {
    pub(super) rect: PaneRect,
    pub(super) direction: SplitDirection,
    pub(super) path: Vec<bool>, // route from the root to the owning Split (false=a, true=b)
}

/// The full pane layout for one tab: the tree, which leaf has focus, and the
/// extra (non-primary) sessions keyed by id.
pub(super) struct TabLayout {
    pub(super) tree: PaneNode,
    pub(super) focused: PaneId,
    pub(super) extra: HashMap<Uuid, TerminalSession>,
}

impl TabLayout {
    /// A fresh layout with a single (primary) pane and no splits.
    pub(super) fn single() -> Self {
        Self {
            tree: PaneNode::Leaf(PaneId::Primary),
            focused: PaneId::Primary,
            extra: HashMap::new(),
        }
    }

    /// Whether this tab is currently split into more than one pane.
    pub(super) fn is_split(&self) -> bool {
        matches!(self.tree, PaneNode::Split { .. })
    }

    /// All leaf ids in left-to-right / top-to-bottom order.
    pub(super) fn leaves(&self) -> Vec<PaneId> {
        fn walk(node: &PaneNode, out: &mut Vec<PaneId>) {
            match node {
                PaneNode::Leaf(id) => out.push(*id),
                PaneNode::Split { a, b, .. } => {
                    walk(a, out);
                    walk(b, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.tree, &mut out);
        out
    }

    /// Replace the leaf `target` with a split of `[target, Extra(new_id)]`.
    pub(super) fn split_leaf(&mut self, target: PaneId, direction: SplitDirection, new_id: Uuid) {
        fn walk(node: &mut PaneNode, target: PaneId, direction: SplitDirection, new_id: Uuid) {
            match node {
                PaneNode::Leaf(id) if *id == target => {
                    let old = PaneNode::Leaf(*id);
                    *node = PaneNode::Split {
                        direction,
                        ratio: 0.5,
                        a: Box::new(old),
                        b: Box::new(PaneNode::Leaf(PaneId::Extra(new_id))),
                    };
                }
                PaneNode::Leaf(_) => {}
                PaneNode::Split { a, b, .. } => {
                    walk(a, target, direction, new_id);
                    walk(b, target, direction, new_id);
                }
            }
        }
        walk(&mut self.tree, target, direction, new_id);
    }

    /// Remove the leaf `target`, collapsing its parent into the sibling.
    /// Returns false if `target` is the only pane (caller closes the tab).
    pub(super) fn remove_leaf(&mut self, target: PaneId) -> bool {
        // Recursively rebuild: if a Split has a child that *is* the target leaf,
        // replace the whole Split with the other child.
        fn rebuild(node: PaneNode, target: PaneId) -> PaneNode {
            match node {
                PaneNode::Leaf(id) => PaneNode::Leaf(id),
                PaneNode::Split {
                    direction,
                    ratio,
                    a,
                    b,
                } => {
                    if matches!(*a, PaneNode::Leaf(id) if id == target) {
                        return rebuild(*b, target);
                    }
                    if matches!(*b, PaneNode::Leaf(id) if id == target) {
                        return rebuild(*a, target);
                    }
                    PaneNode::Split {
                        direction,
                        ratio,
                        a: Box::new(rebuild(*a, target)),
                        b: Box::new(rebuild(*b, target)),
                    }
                }
            }
        }
        if matches!(self.tree, PaneNode::Leaf(id) if id == target) {
            return false;
        }
        let tree = std::mem::replace(&mut self.tree, PaneNode::Leaf(PaneId::Primary));
        self.tree = rebuild(tree, target);
        true
    }

    /// Set the ratio of the Split located at `path` (clamped).
    pub(super) fn set_ratio_at(&mut self, path: &[bool], ratio: f32) {
        let mut node = &mut self.tree;
        for &go_b in path {
            match node {
                PaneNode::Split { a, b, .. } => {
                    node = if go_b { b } else { a };
                }
                PaneNode::Leaf(_) => return,
            }
        }
        if let PaneNode::Split { ratio: r, .. } = node {
            *r = ratio.clamp(0.15, 0.85);
        }
    }

    /// The rect of the Split node located at `path` within `area`.
    pub(super) fn node_rect(
        &self,
        path: &[bool],
        area: PaneRect,
        divider: f32,
    ) -> Option<PaneRect> {
        let mut node = &self.tree;
        let mut rect = area;
        for &go_b in path {
            match node {
                PaneNode::Split {
                    direction,
                    ratio,
                    a,
                    b,
                } => match direction {
                    SplitDirection::Horizontal => {
                        let aw = ((rect.w - divider) * *ratio).max(0.0);
                        if go_b {
                            rect = PaneRect {
                                x: rect.x + aw + divider,
                                w: (rect.w - divider - aw).max(0.0),
                                ..rect
                            };
                            node = b;
                        } else {
                            rect = PaneRect { w: aw, ..rect };
                            node = a;
                        }
                    }
                    SplitDirection::Vertical => {
                        let ah = ((rect.h - divider) * *ratio).max(0.0);
                        if go_b {
                            rect = PaneRect {
                                y: rect.y + ah + divider,
                                h: (rect.h - divider - ah).max(0.0),
                                ..rect
                            };
                            node = b;
                        } else {
                            rect = PaneRect { h: ah, ..rect };
                            node = a;
                        }
                    }
                },
                PaneNode::Leaf(_) => return None,
            }
        }
        matches!(node, PaneNode::Split { .. }).then_some(rect)
    }

    /// Compute the absolute rect of every leaf and every divider for `area`.
    pub(super) fn compute(
        &self,
        area: PaneRect,
        divider: f32,
    ) -> (Vec<(PaneId, PaneRect)>, Vec<DividerRect>) {
        fn walk(
            node: &PaneNode,
            rect: PaneRect,
            divider: f32,
            path: &mut Vec<bool>,
            leaves: &mut Vec<(PaneId, PaneRect)>,
            dividers: &mut Vec<DividerRect>,
        ) {
            match node {
                PaneNode::Leaf(id) => leaves.push((*id, rect)),
                PaneNode::Split {
                    direction,
                    ratio,
                    a,
                    b,
                } => match direction {
                    SplitDirection::Horizontal => {
                        let aw = ((rect.w - divider) * *ratio).max(0.0);
                        let bw = (rect.w - divider - aw).max(0.0);
                        let a_rect = PaneRect { w: aw, ..rect };
                        let div_rect = PaneRect {
                            x: rect.x + aw,
                            w: divider,
                            ..rect
                        };
                        let b_rect = PaneRect {
                            x: rect.x + aw + divider,
                            w: bw,
                            ..rect
                        };
                        dividers.push(DividerRect {
                            rect: div_rect,
                            direction: *direction,
                            path: path.clone(),
                        });
                        path.push(false);
                        walk(a, a_rect, divider, path, leaves, dividers);
                        path.pop();
                        path.push(true);
                        walk(b, b_rect, divider, path, leaves, dividers);
                        path.pop();
                    }
                    SplitDirection::Vertical => {
                        let ah = ((rect.h - divider) * *ratio).max(0.0);
                        let bh = (rect.h - divider - ah).max(0.0);
                        let a_rect = PaneRect { h: ah, ..rect };
                        let div_rect = PaneRect {
                            y: rect.y + ah,
                            h: divider,
                            ..rect
                        };
                        let b_rect = PaneRect {
                            y: rect.y + ah + divider,
                            h: bh,
                            ..rect
                        };
                        dividers.push(DividerRect {
                            rect: div_rect,
                            direction: *direction,
                            path: path.clone(),
                        });
                        path.push(false);
                        walk(a, a_rect, divider, path, leaves, dividers);
                        path.pop();
                        path.push(true);
                        walk(b, b_rect, divider, path, leaves, dividers);
                        path.pop();
                    }
                },
            }
        }
        let mut leaves = Vec::new();
        let mut dividers = Vec::new();
        let mut path = Vec::new();
        walk(
            &self.tree,
            area,
            divider,
            &mut path,
            &mut leaves,
            &mut dividers,
        );
        (leaves, dividers)
    }
}

/// Describe a native pane tree by daemon pane id, for the saved session.
/// `pane_of` gives the daemon pane behind a leaf, or `None` for a leaf
/// that must not be saved (not a daemon pane, or one that has ended): that
/// leaf is left out and its split collapses into the other side. `None`
/// when no leaf remains.
pub(super) fn daemon_tree_of(
    node: &PaneNode,
    pane_of: &impl Fn(PaneId) -> Option<Uuid>,
) -> Option<DaemonPaneNode> {
    match node {
        PaneNode::Leaf(id) => pane_of(*id).map(|pane| DaemonPaneNode::Pane { pane }),
        PaneNode::Split {
            direction,
            ratio,
            a,
            b,
        } => DaemonPaneNode::split(
            matches!(direction, SplitDirection::Horizontal),
            *ratio,
            daemon_tree_of(a, pane_of),
            daemon_tree_of(b, pane_of),
        ),
    }
}

/// Rebuild a native pane tree from a saved one. `leaf_of` maps each daemon
/// pane to the leaf that now shows it; a pane without a leaf is left out,
/// like a pane that is gone. `None` when no leaf remains.
pub(super) fn pane_tree_from_daemon(
    node: &DaemonPaneNode,
    leaf_of: &HashMap<Uuid, PaneId>,
) -> Option<PaneNode> {
    match node {
        DaemonPaneNode::Pane { pane } => leaf_of.get(pane).map(|id| PaneNode::Leaf(*id)),
        DaemonPaneNode::Split {
            horizontal,
            ratio,
            first,
            second,
        } => {
            let a = pane_tree_from_daemon(first, leaf_of);
            let b = pane_tree_from_daemon(second, leaf_of);
            match (a, b) {
                (Some(a), Some(b)) => Some(PaneNode::Split {
                    direction: if *horizontal {
                        SplitDirection::Horizontal
                    } else {
                        SplitDirection::Vertical
                    },
                    ratio: if ratio.is_finite() {
                        ratio.clamp(0.15, 0.85)
                    } else {
                        0.5
                    },
                    a: Box::new(a),
                    b: Box::new(b),
                }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            }
        }
    }
}

/// Tab state shown for a daemon-attached session. A pane that was lost
/// (with its daemon, or closed elsewhere) or never reachable is an error,
/// so the tab does not look alive; a process that exited is an exit.
pub(super) fn attached_tab_state(
    phase: &AttachedPhase,
    ended: &str,
    unavailable: &str,
    other_version: &str,
) -> SessionState {
    match phase {
        AttachedPhase::Connecting | AttachedPhase::Live | AttachedPhase::Reconnecting => {
            SessionState::Running
        }
        AttachedPhase::Ended {
            exit_code: Some(code),
        } => SessionState::Exited(*code),
        AttachedPhase::Ended { exit_code: None } => SessionState::Error(ended.to_string()),
        AttachedPhase::Unavailable => SessionState::Error(unavailable.to_string()),
        // Not shown as running: this app cannot reach the pane. Not shown
        // as ended either: it keeps running under the other version.
        AttachedPhase::OtherVersion => SessionState::Error(other_version.to_string()),
    }
}

#[cfg(test)]
mod daemon_layout_tests {
    // No glob import: `super::*` carries gpui's `test` attribute.
    use super::{
        attached_tab_state, daemon_tree_of, pane_tree_from_daemon, PaneId, PaneNode, SplitDirection,
    };
    use shelldeck_terminal::attached::AttachedPhase;
    use shelldeck_terminal::session::SessionState;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn pane(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn describe(node: &PaneNode) -> String {
        match node {
            PaneNode::Leaf(PaneId::Primary) => "P".into(),
            PaneNode::Leaf(PaneId::Extra(id)) => format!("E{}", id.as_u128()),
            PaneNode::Split {
                direction,
                ratio,
                a,
                b,
            } => format!(
                "{}({} {ratio} {})",
                match direction {
                    SplitDirection::Horizontal => "H",
                    SplitDirection::Vertical => "V",
                },
                describe(a),
                describe(b)
            ),
        }
    }

    // SDTEST-3011 (SDUC-574): the native split tree is saved by daemon
    // pane id and rebuilt with other leaf ids: same shape, directions and
    // ratios; leaves that are not live daemon panes are left out.
    #[test]
    fn sdtest_3011_native_layout_maps_to_daemon_panes_and_back() {
        // P | (E1 / (E2 | E3))
        let tree = PaneNode::Split {
            direction: SplitDirection::Horizontal,
            ratio: 0.3,
            a: Box::new(PaneNode::Leaf(PaneId::Primary)),
            b: Box::new(PaneNode::Split {
                direction: SplitDirection::Vertical,
                ratio: 0.6,
                a: Box::new(PaneNode::Leaf(PaneId::Extra(pane(1)))),
                b: Box::new(PaneNode::Split {
                    direction: SplitDirection::Horizontal,
                    ratio: 0.25,
                    a: Box::new(PaneNode::Leaf(PaneId::Extra(pane(2)))),
                    b: Box::new(PaneNode::Leaf(PaneId::Extra(pane(3)))),
                }),
            }),
        };
        // Daemon pane ids 10, 11, 12, 13 behind the four leaves.
        let all = |id: PaneId| match id {
            PaneId::Primary => Some(pane(10)),
            PaneId::Extra(id) => Some(pane(id.as_u128() + 10)),
        };
        let saved = daemon_tree_of(&tree, &all).expect("saved");
        assert_eq!(saved.panes(), [10, 11, 12, 13].map(pane));

        // After a restart the leaves have new ids, and the leftmost pane
        // is the primary one.
        let leaves: HashMap<Uuid, PaneId> = [
            (pane(10), PaneId::Primary),
            (pane(11), PaneId::Extra(pane(21))),
            (pane(12), PaneId::Extra(pane(22))),
            (pane(13), PaneId::Extra(pane(23))),
        ]
        .into();
        let rebuilt = pane_tree_from_daemon(&saved, &leaves).expect("rebuilt");
        assert_eq!(describe(&rebuilt), "H(P 0.3 V(E21 0.6 H(E22 0.25 E23)))");

        // The primary leaf is an in-process terminal and E2 has ended:
        // neither is saved, and their splits collapse.
        let some = |id: PaneId| match id {
            PaneId::Primary => None,
            PaneId::Extra(id) if id == pane(2) => None,
            PaneId::Extra(id) => Some(pane(id.as_u128() + 10)),
        };
        let saved = daemon_tree_of(&tree, &some).expect("saved");
        assert_eq!(saved.panes(), [11, 13].map(pane));
        let leaves: HashMap<Uuid, PaneId> = [
            (pane(11), PaneId::Primary),
            (pane(13), PaneId::Extra(pane(23))),
        ]
        .into();
        let rebuilt = pane_tree_from_daemon(&saved, &leaves).expect("rebuilt");
        assert_eq!(describe(&rebuilt), "V(P 0.6 E23)");

        // A pane whose session could not be opened is left out too.
        let leaves: HashMap<Uuid, PaneId> = [(pane(13), PaneId::Primary)].into();
        let rebuilt = pane_tree_from_daemon(&saved, &leaves).expect("rebuilt");
        assert_eq!(describe(&rebuilt), "P");
        assert!(pane_tree_from_daemon(&saved, &HashMap::new()).is_none());
        assert!(daemon_tree_of(&tree, &|_| None).is_none());
    }

    // SDTEST-3012 (SDUC-574): what the tab shows for each phase of a
    // daemon pane: waiting phases stay "running", an exit keeps its code,
    // a lost or unreachable pane is an error and never looks alive.
    #[test]
    fn sdtest_3012_tab_state_follows_the_daemon_pane_phase() {
        let state = |phase| attached_tab_state(&phase, "ended", "unavailable", "other version");
        for phase in [
            AttachedPhase::Connecting,
            AttachedPhase::Live,
            AttachedPhase::Reconnecting,
        ] {
            assert_eq!(state(phase), SessionState::Running);
        }
        assert_eq!(
            state(AttachedPhase::Ended { exit_code: Some(3) }),
            SessionState::Exited(3)
        );
        assert_eq!(
            state(AttachedPhase::Ended { exit_code: None }),
            SessionState::Error("ended".into())
        );
        assert_eq!(
            state(AttachedPhase::Unavailable),
            SessionState::Error("unavailable".into())
        );
        assert_eq!(
            state(AttachedPhase::OtherVersion),
            SessionState::Error("other version".into())
        );
    }
}

#[cfg(test)]
mod title_tests {
    use super::{most_urgent, status_tooltip, StatusTone, TerminalTab, ViewAttention};
    use shelldeck_terminal::program_status::{BlockedKind, ProgramHeadline, ProgramState};
    use shelldeck_terminal::session::SessionState;
    use uuid::Uuid;

    fn tab(program: &str, named: bool) -> TerminalTab {
        TerminalTab {
            id: Uuid::nil(),
            title: "Terminal".into(),
            is_active: true,
            state: SessionState::Running,
            zoom_level: 1.0,
            connection_id: None,
            program_title: program.into(),
            named,
            status: None,
            status_seen: false,
        }
    }

    fn headline(state: ProgramState) -> ProgramHeadline {
        ProgramHeadline {
            state,
            kind: None,
            progress: None,
            app: String::new(),
            text: String::new(),
        }
    }

    // SDTEST-3097 / SDUC-576. A blocked or failed program asks for the user
    // until it says otherwise; finished work asks until the tab has been in
    // front; working and idle ask for nothing. Any new report is news again.
    #[test]
    fn sdtest_3097_a_tab_asks_for_attention_until_seen_or_unblocked() {
        let mut tab = tab("", false);
        assert_eq!(
            (tab.status_tone(), tab.attention()),
            (StatusTone::Plain, ViewAttention::None)
        );

        assert!(tab.set_status(Some(headline(ProgramState::Working))));
        assert!(!tab.set_status(Some(headline(ProgramState::Working))));
        assert_eq!(
            (tab.status_tone(), tab.attention()),
            (StatusTone::Working, ViewAttention::None)
        );

        assert!(tab.set_status(Some(headline(ProgramState::Done))));
        assert_eq!(tab.attention(), ViewAttention::Finished);
        tab.status_seen = true;
        assert_eq!(
            (tab.status_tone(), tab.attention()),
            (StatusTone::Plain, ViewAttention::None)
        );

        // Looking at a blocked or failed tab does not make it stop asking.
        for (state, tone) in [
            (ProgramState::Blocked, StatusTone::NeedsYou),
            (ProgramState::Error, StatusTone::Failed),
        ] {
            assert!(tab.set_status(Some(headline(state))));
            assert!(!tab.status_seen, "a new report is not seen yet");
            tab.status_seen = true;
            assert_eq!(
                (tab.status_tone(), tab.attention()),
                (tone, ViewAttention::NeedsYou)
            );
        }

        assert!(tab.set_status(None));
        assert_eq!(tab.attention(), ViewAttention::None);
        assert!(ViewAttention::NeedsYou > ViewAttention::Finished);
        assert!(ViewAttention::Finished > ViewAttention::None);
    }

    // SDTEST-3098 / SDUC-576. A split tab shows its most urgent pane, and the
    // tooltip carries the program's name and own words.
    #[test]
    fn sdtest_3098_a_split_tab_shows_its_most_urgent_pane() {
        let blocked = ProgramHeadline {
            state: ProgramState::Blocked,
            kind: Some(BlockedKind::Permission),
            progress: None,
            app: "claude-code".into(),
            text: "Apply?".into(),
        };
        assert_eq!(most_urgent([None, None]), None);
        assert_eq!(
            most_urgent([
                Some(headline(ProgramState::Working)),
                None,
                Some(blocked.clone()),
                Some(headline(ProgramState::Done)),
            ]),
            Some(blocked.clone())
        );
        let tooltip = status_tooltip(&blocked);
        assert!(tooltip.starts_with("claude-code · "), "{tooltip}");
        assert!(tooltip.ends_with(" · Apply?"), "{tooltip}");
        // No name and no text: only what it is doing.
        assert!(!status_tooltip(&headline(ProgramState::Working)).contains('·'));
    }

    // SDTEST-2912 : tabs follow the running program's title like other
    // terminal emulators, fall back to their base name, and a deliberate
    // (AI) name wins over program titles.
    #[test]
    fn tab_label_follows_program_title_unless_named() {
        assert_eq!(tab("", false).display_title(), "Terminal");
        assert_eq!(tab("  ", false).display_title(), "Terminal");
        assert_eq!(
            tab("user@host: ~/dev", false).display_title(),
            "user@host: ~/dev"
        );
        assert_eq!(tab("user@host: ~/dev", true).display_title(), "Terminal");
    }
}
