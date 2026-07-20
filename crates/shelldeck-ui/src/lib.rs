mod ai_action_dialog;
pub mod ai_assistant;
pub mod ai_workflow;
pub mod bext_cloud_view;
pub mod brand;
pub mod command_palette;
pub mod connection_combobox;
pub mod connection_form;
pub mod dashboard;
pub mod editor_buffer;
pub mod file_editor;
pub mod fleet_view;
pub mod glyph_cache;
pub mod i18n;
pub mod icons;
pub mod issue_attachments;
pub mod jean_view;
pub mod login_form;
pub mod onboarding_view;
pub mod port_forward_form;
pub mod port_forward_view;
pub mod recent_view;
pub mod scale;
pub mod script_editor;
pub mod script_form;
pub mod server_sync_view;
pub mod settings;
pub mod sidebar;
pub mod sites_view;
pub mod status_bar;
pub mod support_view;
pub mod syntax;
pub mod template_browser;
pub mod terminal_view;
pub mod theme;
pub mod toast;
pub mod variable_prompt;
pub mod workspace;

rust_i18n::i18n!("../shelldeck-core/locales", fallback = "fr");

/// UI string lookup — keys in `shelldeck-core/locales/{fr,en}.toml`.
#[macro_export]
macro_rules! t {
    ($($all:tt)*) => {
        $crate::_rust_i18n_t!($($all)*)
    };
}

pub use i18n::{apply_ui_language, rel_time};
pub use workspace::{TrayCounters, TrayNotification, Workspace};
