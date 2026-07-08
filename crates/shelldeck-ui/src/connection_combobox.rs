//! Shared adabraka-ui `Combobox` for SSH connection pickers in forms.

use crate::command_palette::fuzzy_match;
use adabraka_ui::components::combobox::{Combobox, ComboboxState};
use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

pub fn connection_idx_for_id(connections: &[(Uuid, String, String)], id: Uuid) -> Option<usize> {
    connections.iter().position(|(cid, _, _)| *cid == id)
}

/// Build a searchable connection combobox (adabraka-ui — see `.agents/ui-components.md`).
pub fn build_connection_combobox<F, S>(
    connections: &[(Uuid, String, String)],
    selected_idx: usize,
    placeholder: impl Into<SharedString>,
    on_select: F,
    cx: &mut Context<S>,
) -> (Entity<ComboboxState<Uuid>>, Entity<Combobox<Uuid>>)
where
    F: Fn(&Uuid, &mut Window, &mut App) + Send + Sync + 'static,
    S: 'static,
{
    let items: Vec<Uuid> = connections.iter().map(|(id, _, _)| *id).collect();
    let mut combo_state = ComboboxState::<Uuid>::new();
    if let Some((id, _, _)) = connections.get(selected_idx) {
        combo_state.selected = vec![*id];
    }
    let state = cx.new(|_| combo_state);
    let conns = connections.to_vec();

    let combobox = cx.new(|combo_cx| {
        Combobox::new(items, &state, combo_cx)
            .placeholder(placeholder)
            .clearable(false)
            .disabled(conns.is_empty())
            .max_height(gpui::px(200.))
            .filter_fn({
                let conns = conns.clone();
                move |id, search| {
                    if search.is_empty() {
                        return true;
                    }
                    conns
                        .iter()
                        .find(|(cid, _, _)| cid == id)
                        .map(|(_, name, host)| {
                            fuzzy_match(name, search) || fuzzy_match(host, search)
                        })
                        .unwrap_or(false)
                }
            })
            .render_item({
                let conns = conns.clone();
                move |id| {
                    conns
                        .iter()
                        .find(|(cid, _, _)| cid == id)
                        .map(|(_, name, host)| format!("{name} — {host}").into())
                        .unwrap_or_else(|| "Unknown".into())
                }
            })
            .render_selected({
                let conns = conns.clone();
                move |selected| {
                    selected
                        .first()
                        .and_then(|id| {
                            conns
                                .iter()
                                .find(|(cid, _, _)| cid == id)
                                .map(|(_, name, _)| name.clone())
                        })
                        .unwrap_or_default()
                        .into()
                }
            })
            .on_select(on_select)
    });

    (state, combobox)
}
