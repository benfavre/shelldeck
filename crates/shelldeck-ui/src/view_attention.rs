//! What a view the user is not looking at wants from them.

/// Ordered by how much it matters: the larger value wins when several
/// things in one view report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum ViewAttention {
    #[default]
    None,
    /// Something finished that the user has not looked at yet.
    Finished,
    /// Something cannot continue without the user, or failed.
    NeedsYou,
}
