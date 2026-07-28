use super::*;

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
