//! History list: conversations grouped by the surface they came from.

use gpui::prelude::*;
use gpui::*;
use shelldeck_core::ai::{AiConversation, AiSurface};

use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

pub(super) enum HistoryEntry<'a> {
    Group(AiSurface),
    Conversation(&'a AiConversation),
}

/// Conversations, already sorted newest first, grouped by surface. Groups
/// follow the order of their most recent conversation and rows keep their
/// order inside a group, so the freshest work stays at the top.
pub(super) fn grouped_entries<'a>(conversations: &[&'a AiConversation]) -> Vec<HistoryEntry<'a>> {
    let mut surfaces: Vec<AiSurface> = Vec::new();
    for conversation in conversations {
        if !surfaces.contains(&conversation.surface) {
            surfaces.push(conversation.surface);
        }
    }
    let mut entries = Vec::with_capacity(conversations.len() + surfaces.len());
    for surface in surfaces {
        entries.push(HistoryEntry::Group(surface));
        entries.extend(
            conversations
                .iter()
                .copied()
                .filter(|conversation| conversation.surface == surface)
                .map(HistoryEntry::Conversation),
        );
    }
    entries
}

/// The surface names Settings already uses, so a group reads the same as the
/// switch that enables it.
fn surface_label(surface: AiSurface) -> String {
    let key = match surface {
        AiSurface::Global => "ai.history.group.global",
        AiSurface::Support => "settings.ai.surfaces.support",
        AiSurface::Issue => "settings.ai.surfaces.issues",
        AiSurface::Script => "settings.ai.surfaces.scripts",
        AiSurface::Terminal => "settings.ai.surfaces.terminal",
        AiSurface::Monique => "settings.ai.surfaces.monique",
        AiSurface::Naming => "settings.ai.surfaces.naming",
        AiSurface::Recent => "settings.ai.surfaces.recent",
        AiSurface::Clippy => "settings.ai.surfaces.clippy",
    };
    t!(key).to_string()
}

pub(super) fn group_header(surface: AiSurface) -> AnyElement {
    div()
        .flex()
        .min_w(px(0.0))
        .px(px(8.0))
        .pt(px(8.0))
        .pb(px(3.0))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .text_size(px(9.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ShellDeckColors::text_muted())
                .child(surface_label(surface).to_uppercase()),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{grouped_entries, HistoryEntry};
    use shelldeck_core::ai::{AiConversation, AiSurface};

    // SDTEST-1931
    #[test]
    fn sdtest_1931_history_groups_by_surface_in_order_of_latest_activity() {
        let terminal_new = AiConversation::new(AiSurface::Terminal, "Terminal local");
        let support = AiConversation::new(AiSurface::Support, "Ticket DNS");
        let terminal_old = AiConversation::new(AiSurface::Terminal, "Terminal prod");
        let sorted = vec![&terminal_new, &support, &terminal_old];

        let shape: Vec<String> = grouped_entries(&sorted)
            .iter()
            .map(|entry| match entry {
                HistoryEntry::Group(surface) => format!("{surface:?}"),
                HistoryEntry::Conversation(conversation) => conversation.context_title.clone(),
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                "Terminal",
                "Terminal local",
                "Terminal prod",
                "Support",
                "Ticket DNS"
            ]
        );
        assert!(grouped_entries(&Vec::<&AiConversation>::new()).is_empty());
    }
}
