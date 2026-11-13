use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use adabraka_ui::theme::use_theme;
use gpui::prelude::*;
use gpui::*;
use parking_lot::Mutex;
use shelldeck_terminal::colors::{NamedColor, TermColor};
use shelldeck_terminal::grid::{
    CellWidth, CursorShape, CursorState, MouseEncoding, MouseMode, SearchMatch, TerminalGrid,
    UnderlineStyle,
};
use shelldeck_terminal::session::{SessionState, TerminalSession};
use shelldeck_terminal::url::{detect_grid_urls, left_click_action, LeftClickAction, UrlMatch};
use tokio::sync::mpsc;
use uuid::Uuid;

use shelldeck_core::config::app_config::SessionGrouping;
use shelldeck_core::config::themes::TerminalTheme;
use shelldeck_core::tmux::TmuxAction;

use crate::glyph_cache::GlyphCache;
use crate::shortcut_reference::{
    label as shortcut_label, shortcuts_for, BindableAction, ShortcutGroup, ShortcutSurface,
};
use crate::t;
use crate::theme::ShellDeckColors;

mod block_drawing;
mod daemon;
mod empty_state;
pub(crate) use empty_state::trace_layout_for_preview;
mod layout;
mod palette;
mod projects;

use crate::view_attention::ViewAttention;
use block_drawing::paint_block_char;
use layout::{PaneId, PaneNode, PaneRect, StatusTone, TabLayout};
pub use layout::{SplitDirection, TerminalPane, TerminalTab};
use palette::{brighten_for_bold, dim_color, TerminalPalette};
use shelldeck_terminal::program_status::{ProgramHeadline, ProgramState};

const CLAUDE_CLI_COMMAND: &str = "claude --dangerously-skip-permissions";
const CODEX_CLI_COMMAND: &str = "codex --sandbox workspace-write --ask-for-approval on-request";

fn command_available(command: &str) -> bool {
    shelldeck_core::util::executable_on_path(command)
}

pub(crate) fn validate_ai_command(command: &str) -> Result<&str, &'static str> {
    let command = command.trim();
    if command.is_empty() {
        return Err("empty");
    }
    if command.lines().count() != 1 {
        return Err("multiline");
    }
    Ok(command)
}

// ---------------------------------------------------------------------------
// Terminal view types
// ---------------------------------------------------------------------------

// Actions for terminal-specific keyboard shortcuts
actions!(
    shelldeck,
    [
        CopySelection,
        PasteClipboard,
        ClearTerminal,
        ToggleSearch,
        SearchNext,
        SearchPrev,
        SplitHorizontal,
        SplitVertical,
        ClosePane,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ToggleSplitFocus,
        TogglePaneZoom,
    ]
);

/// Events emitted by the terminal view
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    TabSelected(Uuid),
    TabClosed(Uuid),
    NewTabRequested,
    TmuxUnavailable,
    /// Duplicate a connection-backed tab: the workspace should open a new SSH
    /// session for the given connection.
    DuplicateTabRequested(Uuid),
    /// The user requested a split pane. If connection_id is Some, the workspace
    /// should open an SSH session for the split; otherwise a local terminal was
    /// already spawned.
    SplitRequested {
        connection_id: Uuid,
        direction: SplitDirection,
    },
    /// Run a script by ID (emitted from the toolbar script dropdown).
    RunScriptRequested(Uuid),
    /// Toggle pin/unpin a script on the toolbar.
    TogglePinScript(Uuid),
    GenerateCommandWithAi(Uuid),
    DiagnoseWithAi(Uuid),
    SuggestNameWithAi(Uuid),
    CreateIssueFromContext(Uuid),
    StopAiCommand(Uuid),
    AiCommandFinished {
        session_id: Uuid,
        exit_code: Option<i32>,
        output: String,
    },
    /// What the terminals want from the user changed (a program reported
    /// through OSC 7501, or its tab was looked at).
    AttentionChanged,
    /// The session daemon refused input for a pane that stopped reading
    /// it (`count` chunks since the last report, typically one paste):
    /// that input never reached the program.
    InputDropped {
        count: u32,
    },
    /// The user switched what the session list groups the terminals by:
    /// the workspace keeps the choice.
    SessionGroupingChanged(SessionGrouping),
}

impl EventEmitter<TerminalEvent> for TerminalView {}

// ---------------------------------------------------------------------------
// Layout constants – centralised so they aren't scattered as magic numbers.
// ---------------------------------------------------------------------------

/// Base height of the workspace titlebar in pixels, before UI scaling.
const TITLEBAR_HEIGHT: f32 = 40.0;
/// Base height of the terminal tab bar in pixels (before UI scaling).
const TAB_BAR_HEIGHT: f32 = 38.0;
/// Base height of the header of the session list (before UI scaling).
const SESSION_LIST_HEADER_HEIGHT: f32 = 32.0;
/// Minimum / maximum per-tab width (before UI scaling). Tabs shrink toward the
/// minimum as more are opened, then the tab strip scrolls.
const MIN_TAB_WIDTH: f32 = 104.0;
const MAX_TAB_WIDTH: f32 = 220.0;
/// Height of the status bar at the bottom of the window.
const STATUS_BAR_HEIGHT: f32 = 28.0;
/// Width of the sidebar resize handle.
const SIDEBAR_HANDLE_WIDTH: f32 = 4.0;

/// Width of the session list, before UI scaling.
const SESSION_LIST_WIDTH: f32 = 232.0;

/// Window-space x where the terminal content (tab strip, toolbar, panes)
/// starts: right of the application sidebar, when one is drawn, and of the
/// view's own session list, when it is shown.
fn content_left(sidebar_width: f32, full_width: bool, session_list_width: f32) -> f32 {
    let sidebar = if full_width {
        0.0
    } else {
        sidebar_width + SIDEBAR_HANDLE_WIDTH
    };
    sidebar + session_list_width
}

/// Height of the rows above the panes, before UI scaling: the tab strip,
/// unless the session list replaces it. There is no toolbar row: its
/// actions open from a button (`render_actions_button`).
fn chrome_rows_height(session_list: bool) -> f32 {
    if session_list {
        0.0
    } else {
        TAB_BAR_HEIGHT
    }
}
/// Width of the space between two pane cards, which resizes them.
const SPLIT_DIVIDER_SIZE: f32 = layout::PANE_GAP;
/// Vertical offset from the top of the view to the pane area (tab bar).
const GRID_TOP_OFFSET: f32 = TAB_BAR_HEIGHT;
/// Scrollbar track width.
const SCROLLBAR_WIDTH: f32 = 6.0;
/// Right margin of the scrollbar from the grid edge.
const SCROLLBAR_MARGIN: f32 = 2.0;
/// Rows of output a `@terminal` mention carries. Bounded on purpose: a mention
/// identifies a session and shows what it was doing, it does not ship a
/// scrollback.
const TERMINAL_MENTION_TAIL_ROWS: usize = 60;

/// One open terminal tab, as the assistant's mention directory needs it.
#[derive(Debug, Clone)]
pub struct TerminalMentionRow {
    pub id: Uuid,
    pub title: String,
    pub state: String,
    pub cwd: Option<String>,
    pub connection_id: Option<Uuid>,
    /// Epoch milliseconds at which `tail` was captured.
    pub captured_at: i64,
    pub tail: String,
}

/// Native, non-owning workspace snapshot for a live [`TerminalView`].
///
/// It deliberately stores only state the terminal can apply without
/// reconstructing a PTY. The entity remains the authority for sessions.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TerminalWorkspaceSnapshot {
    pub tabs: Vec<Uuid>,
    pub active_tab: Option<Uuid>,
    layouts: HashMap<Uuid, TerminalPaneSnapshot>,
}

#[derive(Clone, Debug, PartialEq)]
struct TerminalPaneSnapshot {
    tree: TerminalPaneNodeSnapshot,
    focused: TerminalPaneKey,
    viewports: HashMap<Uuid, usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalPaneKey {
    Primary,
    Extra(Uuid),
}

#[derive(Clone, Debug, PartialEq)]
enum TerminalPaneNodeSnapshot {
    Leaf(TerminalPaneKey),
    Split {
        horizontal: bool,
        ratio: f32,
        first: Box<TerminalPaneNodeSnapshot>,
        second: Box<TerminalPaneNodeSnapshot>,
    },
}

/// Politique de cwd des nouveaux PTY locaux. `Unscoped` est réservé au
/// terminal général; un terminal de workspace reste `Required` même lorsque
/// sa racine n'est plus disponible, afin de ne jamais retomber sur HOME.
#[derive(Clone, Debug)]
enum LocalCwdScope {
    Unscoped,
    Required {
        intended: std::path::PathBuf,
        authority: Option<Arc<AuthorizedLocalRoot>>,
    },
}

pub(crate) use daemon::RestorablePanes;

/// Session daemon panes that no session shows yet (app restart), or the
/// fact that the daemon belongs to another version of the app.
/// Blocking: call it from a background thread, never the GPUI thread.
pub(crate) fn restorable_daemon_panes() -> RestorablePanes {
    daemon::restorable_panes()
}

pub(crate) struct PreparedLocalTerminal {
    session: TerminalSession,
    authority: AuthorizedLocalRoot,
}

pub struct TerminalView {
    pub pane: TerminalPane,
    pub tabs: Vec<TerminalTab>,
    pub font_size: f32,
    pub font_family: String,
    /// User-configured shell for new local terminals (`[terminal]
    /// default_shell` in `shelldeck.toml`, pushed by the workspace like the
    /// font settings). `None` → the platform default resolved by
    /// `shelldeck-terminal` ($SHELL → /bin/bash on Unix; PowerShell →
    /// %COMSPEC% → cmd.exe on Windows).
    default_shell: Option<String>,
    /// Portée de checkout exigée par chaque nouveau PTY local.
    default_cwd: LocalCwdScope,
    /// `general.session_daemon`: new unscoped local tabs and splits run in
    /// panes owned by the session daemon instead of an in-process PTY.
    persistent_sessions: bool,
    empty_state_width: f32,
    pub focus_handle: FocusHandle,
    _refresh_task: Option<gpui::Task<()>>,
    /// Last known grid dimensions so we can detect when a resize is needed.
    last_grid_rows: u16,
    last_grid_cols: u16,
    /// Last known secondary pane grid dimensions (may differ from primary).
    last_secondary_rows: u16,
    last_secondary_cols: u16,
    /// Set when a session is added so the next render focuses the grid (once).
    needs_focus: bool,
    /// Pre-resolved glyph cache – rebuilt only when font/size changes.
    glyph_cache: Option<Arc<GlyphCache>>,
    /// Timestamp of the last mouse-down for multi-click detection.
    last_click_time: Option<std::time::Instant>,
    /// Number of rapid consecutive clicks (1=single, 2=double, 3=triple).
    click_count: u8,
    /// Position of the last click (for multi-click proximity check).
    last_click_pos: Option<(usize, usize)>,
    /// Whether the left mouse button is held for drag-selection.
    selecting: bool,
    /// Search state
    search_visible: bool,
    search_query: String,
    search_matches: Vec<SearchMatch>,
    search_current_idx: Option<usize>,
    search_case_sensitive: bool,
    search_regex: bool,
    /// Detected URLs in the focused pane's visible grid. Recomputed only when
    /// the focused grid changes (dirty) or the focused session changes, not on
    /// every repaint (e.g. cursor-blink frames keep the cached result).
    detected_urls: Vec<UrlMatch>,
    /// Consume the matching release locally, even if the modifier was released first.
    url_click_active: bool,
    /// Session id the cached `detected_urls` was computed for.
    last_url_session: Option<Uuid>,
    /// Index of the URL currently hovered by the mouse.
    _hovered_url: Option<usize>,
    /// Context menu state (position + whether visible).
    context_menu: Option<ContextMenuState>,
    /// Right-click context menu state for a terminal tab.
    tab_context_menu: Option<TabMenuState>,
    /// Pane layout (tree of splits) for the *active* tab.
    layout: TabLayout,
    /// Stored pane layouts for inactive tabs, keyed by tab ID.
    stored_layouts: HashMap<Uuid, TabLayout>,
    /// Dynamic pixel offsets for the active grid (set during render).
    grid_x_offset: f32,
    grid_y_offset: f32,
    /// Window-space right and bottom edges of the focused grid, recorded
    /// with its origin when it is painted. 0 until the first paint.
    grid_right: f32,
    grid_bottom: f32,
    /// Size of the pane layer as it was last laid out. The cards are placed
    /// from it so their margins are the same on every side; until the first
    /// layout the size is worked out from the window.
    pane_layer_size: Option<(f32, f32)>,
    /// Window-space origin of the pane layer, recorded with its size.
    pane_layer_origin: Option<(f32, f32)>,
    /// Whether the terminal actions are shown, over the pane area.
    actions_open: bool,
    /// Sidebar width in pixels (updated dynamically from workspace).
    sidebar_width: f32,
    /// The focused Terminal presentation renders no sidebar beside the view:
    /// the content then starts at the window's left edge.
    full_width: bool,
    /// The user wants the session list instead of the tab strip.
    session_list_visible: bool,
    /// Width the session list takes this frame, in pixels: 0 when it is not
    /// drawn. Set at the start of each render, read by the geometry.
    session_list_width: f32,
    /// What the session list groups the tabs by.
    session_grouping: SessionGrouping,
    /// Folder each tab works in, as last seen by the refresh loop. A tab
    /// whose folder is not known has no entry.
    tab_folders: HashMap<Uuid, String>,
    /// Projects of the local folders, filled off the UI thread.
    project_index: projects::ProjectIndex,
    /// Projects whose tabs are hidden in the session list.
    collapsed_projects: HashSet<projects::ProjectKey>,
    /// Tabs in the order they were last in front, most recent first.
    recent_tabs: Vec<Uuid>,
    /// Home directory of this machine, read once.
    home_dir: Option<String>,
    /// Whether the workspace is currently showing the application menu row
    /// above the content area (updated dynamically from the workspace).
    menu_bar_visible: bool,
    /// Whether a split divider is being dragged.
    split_dragging: bool,
    /// The divider currently being dragged: (tree path to its split, is_horizontal).
    active_divider: Option<(Vec<bool>, bool)>,
    /// Whether the scrollbar thumb is being dragged.
    scrollbar_dragging: bool,
    /// Active terminal color palette (set from theme).
    palette: TerminalPalette,
    /// User-preferred cursor shape (overrides the grid's DECSCUSR shape when set).
    cursor_style_override: Option<CursorShape>,
    /// Script dropdown state for toolbar integration.
    script_dropdown_open: bool,
    /// Favorite scripts for toolbar dropdown.
    favorite_scripts: Vec<(Uuid, String, String)>,
    /// Recently run scripts for toolbar dropdown.
    recent_scripts: Vec<(Uuid, String, String)>,
    /// Scripts pinned directly to the toolbar for one-click execution.
    pinned_scripts: Vec<PinnedScript>,
    /// Current blink phase: true = cursor visible, false = cursor hidden.
    cursor_blink_on: bool,
    /// Async task that toggles `cursor_blink_on` every 530ms.
    cursor_blink_timer: Option<gpui::Task<()>>,
    /// User preference (from config) that gates cursor blinking entirely.
    /// When `false`, the cursor stays steady regardless of the program's
    /// DECSCUSR blink request.
    cursor_blink_enabled: bool,
    /// Local AI CLIs discovered in PATH when the view is created. Their launch
    /// actions stay hidden when the corresponding executable is unavailable.
    claude_available: bool,
    codex_available: bool,
    ai_actions_enabled: bool,
    ai_naming_enabled: bool,
    ai_running_command: Option<Uuid>,
    observed_command_sequences: HashMap<Uuid, u64>,
    /// Configured scrollback buffer size (lines). Applied to live grids and to
    /// newly added sessions.
    configured_scrollback: usize,
    /// Whether the terminal grid currently has focus (tracked for hollow cursor).
    has_focus: bool,
    /// Sender handed to each session's reader thread; pinged on new output so
    /// the refresh task wakes and repaints (event-driven, not polled). A clone
    /// is kept here so the channel never closes while the view is alive.
    output_tx: mpsc::UnboundedSender<()>,
    /// Receiver for the output signal, moved into the refresh task on start.
    output_rx: Option<mpsc::UnboundedReceiver<()>>,
}

struct TerminalToolbarTooltip {
    label: SharedString,
}

impl Render for TerminalToolbarTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let font_family = use_theme().tokens.font_family.clone();
        div()
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(4.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .shadow_md()
            .text_size(px(11.0))
            .font_family(font_family)
            .text_color(ShellDeckColors::text_primary())
            .whitespace_nowrap()
            .child(self.label.clone())
    }
}

/// A script pinned to the terminal toolbar for one-click execution.
#[derive(Debug, Clone)]
pub struct PinnedScript {
    pub id: Uuid,
    pub name: String,
    pub badge: String,
    pub badge_color: (u8, u8, u8),
}

#[derive(Debug, Clone)]
struct ContextMenuState {
    position: Point<Pixels>,
    /// URL under the right-click, if any.
    url: Option<String>,
}

/// State for a right-click context menu on a terminal tab.
#[derive(Debug, Clone)]
struct TabMenuState {
    /// Window-relative position of the click.
    position: Point<Pixels>,
    /// The tab the menu was opened on.
    tab_id: Uuid,
}

impl TerminalView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (output_tx, output_rx) = mpsc::unbounded_channel::<()>();
        let claude_available = command_available("claude");
        let codex_available = command_available("codex");
        Self {
            pane: TerminalPane {
                sessions: Vec::new(),
                active_index: 0,
            },
            tabs: Vec::new(),
            font_size: 14.0,
            font_family: "JetBrains Mono".to_string(),
            default_shell: None,
            default_cwd: LocalCwdScope::Unscoped,
            persistent_sessions: false,
            empty_state_width: 0.0,
            focus_handle: cx.focus_handle(),
            _refresh_task: None,
            last_grid_rows: 0,
            last_grid_cols: 0,
            last_secondary_rows: 0,
            last_secondary_cols: 0,
            needs_focus: false,
            glyph_cache: None,
            last_click_time: None,
            click_count: 0,
            last_click_pos: None,
            selecting: false,
            search_visible: false,
            search_query: String::new(),
            search_matches: Vec::new(),
            search_current_idx: None,
            search_case_sensitive: false,
            search_regex: false,
            detected_urls: Vec::new(),
            url_click_active: false,
            last_url_session: None,
            _hovered_url: None,
            context_menu: None,
            tab_context_menu: None,
            layout: TabLayout::single(),
            stored_layouts: HashMap::new(),
            grid_x_offset: 260.0 + SIDEBAR_HANDLE_WIDTH,
            grid_y_offset: GRID_TOP_OFFSET,
            grid_right: 0.0,
            grid_bottom: 0.0,
            pane_layer_size: None,
            pane_layer_origin: None,
            actions_open: false,
            sidebar_width: 260.0,
            full_width: false,
            session_list_visible: false,
            session_list_width: 0.0,
            session_grouping: SessionGrouping::Machine,
            tab_folders: HashMap::new(),
            project_index: projects::ProjectIndex::default(),
            collapsed_projects: HashSet::new(),
            recent_tabs: Vec::new(),
            home_dir: shelldeck_core::util::home_dir()
                .and_then(|home| home.to_str().map(str::to_string)),
            menu_bar_visible: true,
            split_dragging: false,
            active_divider: None,
            scrollbar_dragging: false,
            palette: TerminalPalette::default(),
            cursor_style_override: None,
            script_dropdown_open: false,
            favorite_scripts: Vec::new(),
            recent_scripts: Vec::new(),
            pinned_scripts: Vec::new(),
            cursor_blink_on: true,
            cursor_blink_timer: None,
            cursor_blink_enabled: true,
            claude_available,
            codex_available,
            ai_actions_enabled: false,
            ai_naming_enabled: false,
            ai_running_command: None,
            observed_command_sequences: HashMap::new(),
            configured_scrollback: 10_000,
            has_focus: false,
            output_tx,
            output_rx: Some(output_rx),
        }
    }

    /// Update favorite and recent scripts for the toolbar dropdown.
    pub fn set_scripts(
        &mut self,
        favorites: Vec<(Uuid, String, shelldeck_core::models::script::ScriptLanguage)>,
        recent: Vec<(Uuid, String, shelldeck_core::models::script::ScriptLanguage)>,
    ) {
        self.favorite_scripts = favorites
            .into_iter()
            .map(|(id, name, lang)| (id, name, lang.badge().to_string()))
            .collect();
        self.recent_scripts = recent
            .into_iter()
            .map(|(id, name, lang)| (id, name, lang.badge().to_string()))
            .collect();
    }

    /// Update pinned scripts for the toolbar buttons.
    pub fn set_pinned_scripts(&mut self, pinned: Vec<PinnedScript>) {
        self.pinned_scripts = pinned;
    }

    /// Update the sidebar width used for coordinate mapping.
    pub fn set_sidebar_width(&mut self, width: f32) {
        self.sidebar_width = width;
    }

    /// Whether the view is drawn without a sidebar beside it (focused
    /// Terminal presentation). The sidebar width is kept, so leaving the
    /// presentation restores the previous geometry.
    pub fn set_full_width(&mut self, full_width: bool) {
        self.full_width = full_width;
    }

    /// Show the terminals as a list grouped by machine, in place of the tab
    /// strip (never both: one navigation at a time).
    pub fn set_session_list_visible(&mut self, visible: bool) {
        if self.session_list_visible != visible {
            self.session_list_visible = visible;
            // The panes change size: resize them at the next render.
            self.last_grid_rows = 0;
            self.last_grid_cols = 0;
        }
    }

    pub fn session_list_visible(&self) -> bool {
        self.session_list_visible
    }

    /// Group the session list by machine or by project.
    pub fn set_session_grouping(&mut self, grouping: SessionGrouping) {
        self.session_grouping = grouping;
    }

    pub fn session_grouping(&self) -> SessionGrouping {
        self.session_grouping
    }

    /// Where each tab is, for the grouping by project. Reads what the
    /// refresh loop and the lookups recorded: no filesystem work here.
    fn tab_places(&self) -> Vec<projects::TabPlace> {
        self.tabs
            .iter()
            .map(|tab| {
                let folder = self.tab_folders.get(&tab.id);
                let root = match (folder, tab.connection_id) {
                    (Some(folder), Some(_)) => projects::remote_project_root(folder),
                    (Some(folder), None) => self.project_index.root(folder).map(str::to_string),
                    (None, _) => None,
                };
                projects::TabPlace {
                    connection_id: tab.connection_id,
                    host: tab.host.clone(),
                    root,
                }
            })
            .collect()
    }

    /// Record the folder each tab works in and, for a folder of this
    /// machine seen for the first time or entered again, look for its
    /// project on a background thread. `true` when a folder changed.
    fn follow_tab_folders(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        let mut lookups = Vec::new();
        for (session, tab) in self.pane.sessions.iter().zip(&self.tabs) {
            let remote = tab.connection_id.is_some();
            let folder = {
                let grid = session.grid.lock();
                projects::tab_folder(
                    grid.working_directory.as_deref(),
                    &grid.title,
                    session.initial_cwd().and_then(|cwd| cwd.to_str()),
                    // The home of another machine is not this one's.
                    self.home_dir.as_deref().filter(|_| !remote),
                )
            };
            if self.tab_folders.get(&tab.id) == folder.as_ref() {
                continue;
            }
            changed = true;
            match folder {
                Some(folder) => {
                    // A remote path is never looked for on this disk.
                    if !remote && self.project_index.request(&folder) {
                        lookups.push(folder.clone());
                    }
                    self.tab_folders.insert(tab.id, folder);
                }
                None => {
                    self.tab_folders.remove(&tab.id);
                }
            }
        }
        for folder in lookups {
            let home = self.home_dir.clone();
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let looked_up = folder.clone();
                let root = cx
                    .background_executor()
                    .spawn(async move {
                        projects::local_project_root(
                            &looked_up,
                            home.as_deref(),
                            projects::has_git_entry,
                        )
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.project_index.resolve(&folder, root) {
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        changed
    }

    /// Select the most recently used tab of the next (`forward`) or the
    /// previous project. Returns the tab selected, `None` when every tab
    /// is in the same project.
    pub fn select_adjacent_project(&mut self, forward: bool) -> Option<Uuid> {
        let groups = projects::project_groups(&self.tab_places());
        let recent: Vec<usize> = self
            .recent_tabs
            .iter()
            .filter_map(|id| self.tabs.iter().position(|tab| tab.id == *id))
            .collect();
        let index =
            projects::adjacent_project_tab(&groups, self.pane.active_index, &recent, forward)?;
        let id = self.tabs.get(index)?.id;
        // The tab arrived at is shown in the list, whatever was folded.
        if let Some(group) = groups.iter().find(|group| group.tabs.contains(&index)) {
            self.collapsed_projects.remove(&group.key);
        }
        self.select_tab(id);
        self.needs_focus = true;
        Some(id)
    }

    /// `id` is the tab in front: the most recently used one.
    fn mark_tab_recent(&mut self, id: Uuid) {
        self.recent_tabs.retain(|tab| *tab != id);
        self.recent_tabs.insert(0, id);
    }

    /// The session list is drawn: asked for, and there are terminals to list.
    fn session_list_shown(&self) -> bool {
        self.session_list_visible && !self.pane.sessions.is_empty()
    }

    /// Show the focused pane of a split tab alone, or bring the split back.
    pub fn toggle_pane_zoom(&mut self) {
        if !self.layout.is_split() {
            return;
        }
        self.layout.zoomed = !self.layout.zoomed;
        self.last_grid_rows = 0;
        self.last_grid_cols = 0;
    }

    pub fn full_width(&self) -> bool {
        self.full_width
    }

    pub fn sidebar_width(&self) -> f32 {
        self.sidebar_width
    }

    /// The most urgent report among the panes of a tab (OSC 7501).
    fn tab_headline(&self, tab_id: Uuid) -> Option<ProgramHeadline> {
        let active = self
            .tabs
            .get(self.pane.active_index)
            .is_some_and(|tab| tab.id == tab_id);
        let splits = if active {
            Some(&self.layout)
        } else {
            self.stored_layouts.get(&tab_id)
        };
        let primary = self.pane.sessions.iter().find(|s| s.id == tab_id);
        layout::most_urgent(
            primary
                .into_iter()
                .chain(splits.into_iter().flat_map(|layout| layout.extra.values()))
                .map(|session| session.grid.lock().program_status.headline()),
        )
    }

    /// What the terminals want from a user who is looking elsewhere: the
    /// most pressing tab.
    pub fn attention(&self) -> ViewAttention {
        self.tabs
            .iter()
            .map(TerminalTab::attention)
            .max()
            .unwrap_or_default()
    }

    /// The active tab is in front of the user: its finished work is seen.
    fn mark_active_tab_seen(&mut self, cx: &mut Context<Self>) {
        let before = self.attention();
        if let Some(tab) = self.tabs.get_mut(self.pane.active_index) {
            tab.status_seen = true;
        }
        if self.attention() != before {
            cx.emit(TerminalEvent::AttentionChanged);
        }
    }

    /// Give the active pane keyboard focus at the next render.
    pub fn request_focus(&mut self) {
        self.needs_focus = true;
    }

    /// Window-space x of the view's left edge: just right of the sidebar and
    /// its resize handle, or the window edge when no sidebar is drawn.
    fn content_left(&self) -> f32 {
        content_left(self.sidebar_width, self.full_width, self.session_list_width)
    }

    /// Menu visibility is retained with the settings snapshot; the menus now
    /// share the titlebar and do not reserve a separate content row.
    pub fn set_menu_bar_visible(&mut self, visible: bool) {
        self.menu_bar_visible = visible;
    }

    /// Apply a terminal color theme to the renderer.
    pub fn set_terminal_theme(&mut self, theme: &TerminalTheme) {
        self.palette = TerminalPalette::from_theme(theme);
    }

    /// Update the base font size (invalidates glyph cache).
    pub fn set_font_size(&mut self, size: f32) {
        self.font_size = size.clamp(8.0, 36.0);
        self.glyph_cache = None;
    }

    /// Update the font family (invalidates glyph cache).
    pub fn set_font_family(&mut self, family: String) {
        self.font_family = family;
        self.glyph_cache = None;
    }

    /// Update the user-configured shell used for *new* local terminals.
    /// Blank values count as unset; already-running sessions keep the shell
    /// they were spawned with.
    pub fn set_default_shell(&mut self, shell: Option<String>) {
        self.default_shell = shell.filter(|s| !s.trim().is_empty());
    }

    /// Opt-in session daemon backend (`general.session_daemon`). Applies to
    /// *new* unscoped local terminals; running sessions keep their backend.
    pub fn set_persistent_sessions(&mut self, enabled: bool) {
        self.persistent_sessions = enabled;
    }

    /// Whether new unscoped local terminals of this view are session
    /// daemon panes, and so whether panes may be offered back to it.
    pub fn restores_daemon_panes(&self) -> bool {
        self.persistent_sessions && matches!(self.default_cwd, LocalCwdScope::Unscoped)
    }

    /// Rebuild one tab around session daemon panes that an earlier run of
    /// the app left running: the saved split tree and ratios, one session
    /// per pane. Returns the id of the new tab, which becomes the active
    /// one.
    ///
    /// Nothing here waits for the daemon: each session is created in its
    /// "connecting" phase and attaches on its own thread, keeping the
    /// pane's size until the view has been laid out. A pane that turns out
    /// to be gone is reported inside its session.
    pub fn restore_daemon_tab(
        &mut self,
        title: Option<&str>,
        saved: &shelldeck_core::config::workspace_state::DaemonTabLayout,
        cx: &mut Context<Self>,
    ) -> Option<Uuid> {
        if !self.restores_daemon_panes() {
            return None;
        }
        let (rows, cols) = self.grid_size();
        let session_title = title.unwrap_or("Terminal");
        let mut sessions = HashMap::new();
        for pane in saved.tree.panes() {
            match daemon::attach(pane, session_title, rows, cols, self.configured_scrollback) {
                Ok(session) => {
                    sessions.insert(pane, session);
                }
                Err(error) => tracing::warn!(%pane, "pane not restored: {error}"),
            }
        }
        // The leftmost pane takes the tab's primary slot.
        let order: Vec<Uuid> = saved
            .tree
            .panes()
            .into_iter()
            .filter(|pane| sessions.contains_key(pane))
            .collect();
        let primary = sessions.remove(order.first()?)?;
        let tab_id = primary.id;
        self.add_session(primary);
        let extras = order[1..]
            .iter()
            .filter_map(|pane| sessions.remove(pane).map(|session| (*pane, session)))
            .collect();
        self.install_daemon_splits(tab_id, order[0], extras, saved);
        if let (Some(title), Some(tab)) = (title, self.tabs.iter_mut().find(|t| t.id == tab_id)) {
            tab.title = title.to_string();
            tab.named = saved.named;
        }
        self.ensure_refresh_running(cx);
        Some(tab_id)
    }

    /// Show the session daemon pane `ask.pane` beside the pane that shows
    /// `ask.beside`, in whichever tab that is (`shelldeck-sessiond split`).
    /// Neither the active tab nor the focused pane changes: the request
    /// comes from a program, often an agent working in the background,
    /// and the user keeps typing where they were. `Err` says why nothing
    /// was shown; the pane itself runs either way.
    fn adopt_daemon_split(&mut self, ask: &daemon::SplitAsk) -> Result<(), String> {
        if !self.restores_daemon_panes() {
            return Err("persistent sessions are off".into());
        }
        let active_tab = self.tabs.get(self.pane.active_index).map(|tab| tab.id);
        if self.find_daemon_pane(ask.pane).is_some() {
            return Ok(());
        }
        let Some((tab, leaf)) = self.find_daemon_pane(ask.beside) else {
            return Err("no tab of the application shows that pane".into());
        };
        let (rows, cols) = self.grid_size();
        let session = daemon::attach(ask.pane, "Terminal", rows, cols, self.configured_scrollback)?;
        session
            .grid
            .lock()
            .set_max_scrollback(self.configured_scrollback);
        session.set_output_notifier(self.output_tx.clone());
        let direction = if ask.down {
            SplitDirection::Vertical
        } else {
            SplitDirection::Horizontal
        };
        let layout = if Some(tab) == active_tab {
            &mut self.layout
        } else {
            self.stored_layouts
                .entry(tab)
                .or_insert_with(TabLayout::single)
        };
        let new_id = Uuid::new_v4();
        if !layout.split_leaf_at(leaf, direction, new_id, 1.0 - ask.ratio) {
            return Err("that pane is no longer part of its tab".into());
        }
        layout.extra.insert(new_id, session);
        layout.zoomed = false;
        // Size the panes of the tab again on the next render.
        self.last_grid_rows = 0;
        self.last_grid_cols = 0;
        Ok(())
    }

    /// Where the session daemon pane `pane` is shown: its tab and its
    /// place in that tab's layout.
    fn find_daemon_pane(&self, pane: Uuid) -> Option<(Uuid, PaneId)> {
        let shows = |session: &TerminalSession| session.daemon_pane() == Some(pane);
        let active_tab = self.tabs.get(self.pane.active_index).map(|tab| tab.id);
        self.pane.sessions.iter().find_map(|primary| {
            if shows(primary) {
                return Some((primary.id, PaneId::Primary));
            }
            let layout = if Some(primary.id) == active_tab {
                &self.layout
            } else {
                self.stored_layouts.get(&primary.id)?
            };
            layout
                .extra
                .iter()
                .find(|(_, session)| shows(session))
                .map(|(leaf, _)| (primary.id, PaneId::Extra(*leaf)))
        })
    }

    /// Do what `shelldeck-sessiond focus|rename|zoom|unzoom` asked for the
    /// session daemon pane `pane`. Only `focus` changes the active tab.
    /// `Err` says why nothing was done.
    fn act_on_daemon_pane(
        &mut self,
        pane: Uuid,
        action: &daemon::PaneAction,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.restores_daemon_panes() {
            return Err("persistent sessions are off".into());
        }
        let Some((tab, leaf)) = self.find_daemon_pane(pane) else {
            return Err("no tab of the application shows that pane".into());
        };
        let active = self.tabs.get(self.pane.active_index).map(|tab| tab.id) == Some(tab);
        match action {
            daemon::PaneAction::Focus => {
                if !active {
                    self.select_tab(tab);
                    cx.emit(TerminalEvent::TabSelected(tab));
                }
                self.focus_pane(leaf);
                if self.layout.zoomed {
                    // The pane asked for is the one shown alone.
                    self.last_grid_rows = 0;
                    self.last_grid_cols = 0;
                }
            }
            // The name is the tab's, whichever pane of it was named: a
            // pane inside a split shows what its program calls itself.
            daemon::PaneAction::Rename(name) => {
                if let Some(session) = self.pane.sessions.iter_mut().find(|s| s.id == tab) {
                    session.title = name.clone();
                }
                if let Some(entry) = self.tabs.iter_mut().find(|entry| entry.id == tab) {
                    entry.title = name.clone();
                    entry.named = true;
                }
            }
            daemon::PaneAction::Zoom | daemon::PaneAction::Unzoom => {
                let zoom = *action == daemon::PaneAction::Zoom;
                let layout = if active {
                    &mut self.layout
                } else {
                    self.stored_layouts
                        .get_mut(&tab)
                        .ok_or("that pane is no longer part of its tab")?
                };
                if zoom {
                    if !layout.is_split() {
                        return Err("that pane is alone in its tab".into());
                    }
                    layout.focused = leaf;
                }
                layout.zoomed = zoom;
                if active {
                    self.last_grid_rows = 0;
                    self.last_grid_cols = 0;
                }
            }
        }
        Ok(())
    }

    /// Select a restored tab and give keyboard focus back to the pane that
    /// had it (selecting a tab focuses its primary pane otherwise).
    pub fn activate_restored_tab(&mut self, tab_id: Uuid, focused_pane: Option<Uuid>) {
        if !self.tabs.iter().any(|tab| tab.id == tab_id) {
            return;
        }
        self.select_tab(tab_id);
        let Some(pane) = focused_pane else { return };
        let leaf = self.layout.leaves().into_iter().find(|leaf| {
            self.session_for(*leaf)
                .is_some_and(|session| session.daemon_pane() == Some(pane))
        });
        if let Some(leaf) = leaf {
            self.layout.focused = leaf;
        }
    }

    /// Give the tab `tab_id`, whose primary session shows the daemon pane
    /// `primary_pane`, the splits of `saved`: one split pane per entry of
    /// `extras` (daemon pane id, session), placed and sized as saved, and
    /// the saved focus. Panes of the tree without a session are dropped
    /// (their split collapses). Used for tabs rebuilt around local daemon
    /// panes at startup and for SSH tabs rebuilt around the panes left on
    /// their host. Returns `false` when there is no such tab.
    pub(crate) fn install_daemon_splits(
        &mut self,
        tab_id: Uuid,
        primary_pane: Uuid,
        extras: Vec<(Uuid, TerminalSession)>,
        saved: &shelldeck_core::config::workspace_state::DaemonTabLayout,
    ) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return false;
        };
        let scrollback = self.configured_scrollback;
        let notifier = self.output_tx.clone();
        let layout = if index == self.pane.active_index {
            &mut self.layout
        } else {
            self.stored_layouts
                .entry(tab_id)
                .or_insert_with(TabLayout::single)
        };
        let mut leaves = HashMap::from([(primary_pane, PaneId::Primary)]);
        for (pane, session) in extras {
            session.grid.lock().set_max_scrollback(scrollback);
            session.set_output_notifier(notifier.clone());
            let id = Uuid::new_v4();
            layout.extra.insert(id, session);
            leaves.insert(pane, PaneId::Extra(id));
        }
        if let Some(tree) = layout::pane_tree_from_daemon(&saved.tree, &leaves) {
            layout.tree = tree;
        }
        layout.focused = saved
            .focused
            .and_then(|pane| leaves.get(&pane).copied())
            .unwrap_or(PaneId::Primary);
        if let Some(tab) = self.tabs.get_mut(index) {
            tab.named = tab.named || saved.named;
        }
        true
    }

    /// For each tab, in tab order, its layout by pane id of its SSH
    /// host's session daemon, or `None` for a local tab or an SSH tab
    /// whose primary pane is not a daemon pane (tmux, plain shell). Saved
    /// so the tab comes back with its splits when the host is connected
    /// again. Read-only.
    pub fn remote_tab_layouts(
        &self,
    ) -> Vec<Option<shelldeck_core::config::workspace_state::DaemonTabLayout>> {
        self.tab_layouts_by_pane(true)
    }

    /// For each tab, in tab order, its layout by session daemon pane id,
    /// or `None` when its primary pane is not a daemon pane. Leaves that
    /// are not live daemon panes (in-process, ended, not spawned yet) are
    /// left out. Read-only.
    pub fn daemon_tab_layouts(
        &self,
    ) -> Vec<Option<shelldeck_core::config::workspace_state::DaemonTabLayout>> {
        self.tab_layouts_by_pane(false)
    }

    /// `remote`: the SSH tabs (panes of their host's daemon), else the
    /// local ones (panes of this machine's daemon).
    fn tab_layouts_by_pane(
        &self,
        remote: bool,
    ) -> Vec<Option<shelldeck_core::config::workspace_state::DaemonTabLayout>> {
        fn live_pane(session: &TerminalSession) -> Option<Uuid> {
            use shelldeck_terminal::attached::AttachedPhase;
            let status = session.attached_status()?;
            match status.phase() {
                AttachedPhase::Ended { .. } | AttachedPhase::Unavailable => None,
                // `OtherVersion` included: the pane runs under a daemon
                // this app cannot speak to, and its place in the layout
                // is kept for the version that can.
                _ => status.pane(),
            }
        }
        let single = TabLayout::single();
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                // A tab of an SSH connection shows panes of that host's
                // daemon: they are matched against that host when it is
                // connected again, never against the local daemon.
                if tab.connection_id.is_some() != remote {
                    return None;
                }
                let primary = self.pane.sessions.get(index)?;
                live_pane(primary)?;
                let layout = if index == self.pane.active_index {
                    &self.layout
                } else {
                    self.stored_layouts.get(&tab.id).unwrap_or(&single)
                };
                let pane_of = |id: PaneId| match id {
                    PaneId::Primary => live_pane(primary),
                    PaneId::Extra(id) => layout.extra.get(&id).and_then(live_pane),
                };
                Some(shelldeck_core::config::workspace_state::DaemonTabLayout {
                    tree: layout::daemon_tree_of(&layout.tree, &pane_of)?,
                    focused: pane_of(layout.focused),
                    named: tab.named,
                })
            })
            .collect()
    }

    /// Set the canonical checkout root for every future local tab, split, or
    /// CLI-created terminal owned by this view.
    pub(crate) fn set_default_cwd(&mut self, cwd: &std::path::Path) -> Result<(), String> {
        let intended = cwd.to_path_buf();
        self.default_cwd = LocalCwdScope::Required {
            intended: intended.clone(),
            authority: None,
        };
        let authority = AuthorizedLocalRoot::capture(cwd)?;
        self.default_cwd = LocalCwdScope::Required {
            intended,
            authority: Some(Arc::new(authority)),
        };
        Ok(())
    }

    /// Installe une racine déjà canonique et autorisée sans relire le système
    /// de fichiers. Le lanceur s'en sert avant d'exposer une nouvelle surface:
    /// les actions utilisateur ne peuvent ainsi jamais retomber sur le cwd du
    /// processus. Si le dossier disparaît ensuite, la création du PTY échoue
    /// fermée avec cette racine au lieu d'utiliser un autre répertoire.
    pub(crate) fn install_authorized_default_cwd(&mut self, root: &AuthorizedLocalRoot) {
        debug_assert!(root.path().is_absolute());
        self.default_cwd = LocalCwdScope::Required {
            intended: root.path().to_path_buf(),
            authority: Some(Arc::new(root.clone())),
        };
    }

    pub(crate) fn required_cwd_unavailable_message(&self) -> Option<String> {
        let LocalCwdScope::Required {
            intended,
            authority: None,
        } = &self.default_cwd
        else {
            return None;
        };
        Some(
            t!(
                "terminal.empty.workspace_folder_unavailable",
                path = intended.display().to_string().as_str()
            )
            .to_string(),
        )
    }

    #[cfg(test)]
    pub(crate) fn runtime_config_probe(&self) -> (f32, bool, f32, &str, Option<&str>, bool, usize) {
        (
            self.sidebar_width,
            self.menu_bar_visible,
            self.font_size,
            &self.font_family,
            self.default_shell.as_deref(),
            self.cursor_blink_enabled,
            self.configured_scrollback,
        )
    }

    fn spawn_configured_local(
        &mut self,
        rows: u16,
        cols: u16,
    ) -> shelldeck_terminal::Result<TerminalSession> {
        let (authority_cwd, canonical_cwd, authority) = match &self.default_cwd {
            LocalCwdScope::Unscoped => {
                if self.persistent_sessions {
                    // Returns at once: the session connects on its own
                    // thread and shows a notice meanwhile. Workspace-scoped
                    // terminals stay in-process: their directory authority
                    // is a held descriptor the daemon cannot honour
                    // (design doc, section 15).
                    match daemon::spawn(
                        self.default_shell.as_deref(),
                        rows,
                        cols,
                        self.configured_scrollback,
                    ) {
                        Ok(session) => return Ok(session),
                        Err(error) => tracing::warn!(
                            "Session daemon unavailable, using an in-process terminal: {error}"
                        ),
                    }
                }
                return TerminalSession::spawn_local(self.default_shell.as_deref(), rows, cols);
            }
            LocalCwdScope::Required {
                authority: Some(authority),
                ..
            } => {
                if authority.revalidate().is_err() {
                    if let LocalCwdScope::Required { authority, .. } = &mut self.default_cwd {
                        *authority = None;
                    }
                    return Err(shelldeck_terminal::TerminalError::Pty(
                        "authorized terminal working directory changed".into(),
                    ));
                }
                (
                    authority.command_path(),
                    authority.path().to_path_buf(),
                    authority.clone(),
                )
            }
            LocalCwdScope::Required {
                authority: None, ..
            } => {
                return Err(shelldeck_terminal::TerminalError::Pty(
                    "required workspace terminal directory is unavailable".into(),
                ));
            }
        };
        // `portable-pty` peut retomber silencieusement sur le répertoire du
        // processus lorsque `cwd` disparaît. Refuser ici ferme cette course
        // pour tous les onglets, splits et lanceurs CLI, puis rend l'état
        // indisponible visible au prochain rendu.
        if !authority_cwd.is_dir() {
            if let LocalCwdScope::Required { authority, .. } = &mut self.default_cwd {
                *authority = None;
            }
            return Err(shelldeck_terminal::TerminalError::Pty(
                "authorized terminal working directory is unavailable".into(),
            ));
        }
        let session = TerminalSession::spawn_local_at_authorized(
            self.default_shell.as_deref(),
            rows,
            cols,
            &authority_cwd,
            &canonical_cwd,
        )?;
        if authority.revalidate().is_err() {
            drop(session);
            if let LocalCwdScope::Required { authority, .. } = &mut self.default_cwd {
                *authority = None;
            }
            return Err(shelldeck_terminal::TerminalError::Pty(
                "authorized terminal working directory changed".into(),
            ));
        }
        Ok(session)
    }

    /// Update the cursor style preference.
    ///
    /// Pass `"default"` (or any unrecognized value) to clear the override and
    /// let the terminal application control the shape via DECSCUSR.
    pub fn set_cursor_style(&mut self, style: &str) {
        self.cursor_style_override = match style {
            "underline" => Some(CursorShape::Underline),
            "bar" => Some(CursorShape::Bar),
            "block" => Some(CursorShape::Block),
            _ => None,
        };
    }

    /// Apply the configured scrollback size to every live session grid and
    /// remember it for sessions created later.
    pub fn set_scrollback_lines(&mut self, lines: usize) {
        let lines = lines.max(1);
        self.configured_scrollback = lines;
        for session in &self.pane.sessions {
            session.grid.lock().set_max_scrollback(lines);
        }
        for session in self.layout.extra.values() {
            session.grid.lock().set_max_scrollback(lines);
        }
        for layout in self.stored_layouts.values() {
            for session in layout.extra.values() {
                session.grid.lock().set_max_scrollback(lines);
            }
        }
    }

    /// Apply the user's cursor-blink preference (from config). When disabled,
    /// the cursor is forced steady; when enabled it resumes blinking if the
    /// terminal currently wants it.
    pub fn set_cursor_blink(&mut self, enabled: bool) {
        if self.cursor_blink_enabled == enabled {
            return;
        }
        self.cursor_blink_enabled = enabled;
        if !enabled {
            self.stop_cursor_blink();
        }
    }

    // ------------------------------------------------------------------
    // Cursor blink timer
    // ------------------------------------------------------------------

    /// Start (or restart) the cursor blink timer.
    ///
    /// The cursor is made visible immediately and then toggled every 530 ms.
    /// Calling this while a timer is already running cancels the old one.
    /// No-op when the user has disabled blinking via config.
    fn start_cursor_blink(&mut self, cx: &mut Context<Self>) {
        if !self.cursor_blink_enabled {
            self.cursor_blink_on = true;
            return;
        }
        self.cursor_blink_on = true;
        let entity = cx.entity().downgrade();
        self.cursor_blink_timer = Some(cx.spawn(
            async move |_this: WeakEntity<Self>, cx: &mut AsyncApp| loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(530))
                    .await;
                let Ok(_) = entity.update(cx, |this, cx| {
                    // Only toggle + repaint when the cursor is actually on screen.
                    // Full-screen apps that hide the cursor (e.g. htop via DECTCEM)
                    // otherwise cost a full-grid repaint twice a second for nothing.
                    let visible = this
                        .active_session()
                        .map(|s| s.grid.lock().cursor.visible)
                        .unwrap_or(false);
                    if visible {
                        this.cursor_blink_on = !this.cursor_blink_on;
                        cx.notify();
                    }
                }) else {
                    break;
                };
            },
        ));
    }

    /// Stop the blink timer and ensure the cursor is visible (steady).
    fn stop_cursor_blink(&mut self) {
        self.cursor_blink_timer = None;
        self.cursor_blink_on = true;
    }

    /// Reset the blink cycle so the cursor stays visible right after input,
    /// then resumes blinking after the interval.
    fn reset_cursor_blink(&mut self, cx: &mut Context<Self>) {
        // Only restart if the grid says blinking is enabled
        let grid_blink = self
            .active_session()
            .map(|s| s.grid.lock().cursor.blink)
            .unwrap_or(false);
        if grid_blink && self.has_focus {
            self.start_cursor_blink(cx);
        } else {
            self.stop_cursor_blink();
        }
    }

    /// Determine the effective cursor shape: the user override wins if set,
    /// otherwise the grid's shape (set by DECSCUSR) is used.
    fn effective_cursor_shape(&self, grid_shape: CursorShape) -> CursorShape {
        self.cursor_style_override.unwrap_or(grid_shape)
    }

    pub fn add_session(&mut self, session: TerminalSession) {
        self.add_session_with_connection(session, None);
    }

    pub fn add_session_with_connection(
        &mut self,
        session: TerminalSession,
        connection_id: Option<Uuid>,
    ) {
        // Apply the configured scrollback size to the freshly-spawned grid
        // (sessions are spawned with the engine default of 10k lines).
        session
            .grid
            .lock()
            .set_max_scrollback(self.configured_scrollback);

        // Wire the session's reader thread to wake the UI on output.
        session.set_output_notifier(self.output_tx.clone());

        // Save the current tab's pane layout before switching away
        if let Some(current_tab) = self.tabs.get(self.pane.active_index) {
            let current_id = current_tab.id;
            let layout = std::mem::replace(&mut self.layout, TabLayout::single());
            self.stored_layouts.insert(current_id, layout);
        }

        let tab = TerminalTab {
            id: session.id,
            title: session.title.clone(),
            is_active: true,
            state: session.state.clone(),
            zoom_level: 1.0,
            connection_id,
            program_title: String::new(),
            named: false,
            status: None,
            status_seen: false,
            // A connection tab opens under the connection's name.
            host: connection_id.map(|_| session.title.clone()),
        };

        // Deactivate other tabs
        for t in &mut self.tabs {
            t.is_active = false;
        }

        self.mark_tab_recent(tab.id);
        self.tabs.push(tab);
        self.pane.active_index = self.pane.sessions.len();
        self.pane.sessions.push(session);

        // Request focus on next render
        self.needs_focus = true;
    }

    /// Scrollback rows configured for the sessions of this view.
    pub(crate) fn configured_scrollback(&self) -> usize {
        self.configured_scrollback
    }

    /// Whether a session with this id is shown, in any tab or split.
    pub(crate) fn has_session(&self, session_id: Uuid) -> bool {
        let extras = std::iter::once(&self.layout)
            .chain(self.stored_layouts.values())
            .flat_map(|layout| layout.extra.values());
        self.pane
            .sessions
            .iter()
            .chain(extras)
            .any(|session| session.id == session_id)
    }

    /// Put `session` in the place of the session `old_id` (a tab's primary
    /// pane or a split), keeping the tab, its title and the layout. Used
    /// when the backend of a tab is only known after it was opened (an SSH
    /// tab that turns out to be a session daemon pane). The replaced
    /// session is dropped, not terminated. `false`: no such session (the
    /// tab was closed meanwhile); `session` is dropped.
    ///
    /// The new session takes the id of the one it replaces. A tab is
    /// found by the id of its primary session and a split by its key in
    /// the layout (tab dot, AI naming, mentions, snapshots, `select_tab`):
    /// with another id the handed-over tab would be missed by all of them.
    pub(crate) fn replace_session(&mut self, old_id: Uuid, mut session: TerminalSession) -> bool {
        session.id = old_id;
        session
            .grid
            .lock()
            .set_max_scrollback(self.configured_scrollback);
        session.set_output_notifier(self.output_tx.clone());
        let extras = std::iter::once(&mut self.layout)
            .chain(self.stored_layouts.values_mut())
            .flat_map(|layout| layout.extra.values_mut());
        let Some(slot) = self
            .pane
            .sessions
            .iter_mut()
            .chain(extras)
            .find(|slot| slot.id == old_id)
        else {
            return false;
        };
        *slot = session;
        // Size the new session on the next render.
        self.last_grid_rows = 0;
        self.last_grid_cols = 0;
        true
    }

    pub fn close_tab(&mut self, id: Uuid) {
        if let Some(pos) = self.tabs.iter().position(|t| t.id == id) {
            if self.ai_running_command == Some(id) {
                self.ai_running_command = None;
            }
            // Drop the tab's entire pane layout (all its split sessions).
            let was_active = pos == self.pane.active_index;
            // A session daemon pane outlives its dropped session: closing
            // the tab is what ends it (no-op for PTY and SSH sessions).
            self.pane.sessions[pos].terminate();
            let closing_layout = if was_active {
                Some(&self.layout)
            } else {
                self.stored_layouts.get(&id)
            };
            for session in closing_layout
                .into_iter()
                .flat_map(|layout| layout.extra.values())
            {
                session.terminate();
            }
            if was_active {
                self.layout = TabLayout::single();
            } else {
                self.stored_layouts.remove(&id);
            }

            self.tabs.remove(pos);
            self.pane.sessions.remove(pos);
            self.tab_folders.remove(&id);
            self.recent_tabs.retain(|tab| *tab != id);

            if self.pane.active_index >= self.pane.sessions.len() && !self.pane.sessions.is_empty()
            {
                self.pane.active_index = self.pane.sessions.len() - 1;
            }

            // Restore the new active tab's layout if it had one stored.
            if was_active && !self.pane.sessions.is_empty() {
                if let Some(tab) = self.tabs.get(self.pane.active_index) {
                    self.layout = self
                        .stored_layouts
                        .remove(&tab.id)
                        .unwrap_or_else(TabLayout::single);
                }
            }

            if let Some(tab) = self.tabs.get_mut(self.pane.active_index) {
                tab.is_active = true;
                let front = tab.id;
                self.mark_tab_recent(front);
            }

            // Stop the refresh task when all sessions are closed (drop cancels the task)
            if self.pane.sessions.is_empty() {
                self.layout = TabLayout::single();
                self.stored_layouts.clear();
                self._refresh_task = None;
            }
        }
    }

    /// Duplicate a tab. Connection-backed tabs ask the workspace to open a new
    /// SSH session for the same connection; local tabs spawn a fresh shell.
    pub fn duplicate_tab(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let connection_id = self
            .tabs
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.connection_id);
        if let Some(connection_id) = connection_id {
            cx.emit(TerminalEvent::DuplicateTabRequested(connection_id));
        } else {
            self.spawn_local_terminal(cx);
            cx.emit(TerminalEvent::NewTabRequested);
        }
    }

    /// Close every tab positioned to the right of the given tab.
    pub fn close_tabs_to_right(&mut self, id: Uuid) {
        if let Some(pos) = self.tabs.iter().position(|t| t.id == id) {
            let ids: Vec<Uuid> = self.tabs.iter().skip(pos + 1).map(|t| t.id).collect();
            for tab_id in ids {
                self.close_tab(tab_id);
            }
        }
    }

    /// Close every tab positioned to the left of the given tab.
    pub fn close_tabs_to_left(&mut self, id: Uuid) {
        if let Some(pos) = self.tabs.iter().position(|t| t.id == id) {
            let ids: Vec<Uuid> = self.tabs.iter().take(pos).map(|t| t.id).collect();
            for tab_id in ids {
                self.close_tab(tab_id);
            }
        }
    }

    /// Whether the given tab has any tabs to its right.
    fn tab_has_right(&self, id: Uuid) -> bool {
        self.tabs
            .iter()
            .position(|t| t.id == id)
            .is_some_and(|pos| pos + 1 < self.tabs.len())
    }

    /// Whether the given tab has any tabs to its left.
    fn tab_has_left(&self, id: Uuid) -> bool {
        self.tabs
            .iter()
            .position(|t| t.id == id)
            .is_some_and(|pos| pos > 0)
    }

    /// Find a tab that belongs to the given connection, if any.
    pub fn find_tab_for_connection(&self, connection_id: Uuid) -> Option<Uuid> {
        self.tabs
            .iter()
            .find(|t| t.connection_id == Some(connection_id))
            .map(|t| t.id)
    }

    pub fn select_tab(&mut self, id: Uuid) {
        if self
            .tabs
            .get(self.pane.active_index)
            .is_some_and(|active| active.id == id)
        {
            return;
        }
        if self.tabs.iter().any(|tab| tab.id == id) {
            self.mark_tab_recent(id);
        }
        // Save the current tab's pane layout before switching away.
        if let Some(current_tab) = self.tabs.get(self.pane.active_index) {
            let current_id = current_tab.id;
            if current_id != id {
                let layout = std::mem::replace(&mut self.layout, TabLayout::single());
                self.stored_layouts.insert(current_id, layout);
            }
        }

        for (i, tab) in self.tabs.iter_mut().enumerate() {
            tab.is_active = tab.id == id;
            if tab.is_active {
                self.pane.active_index = i;
            }
        }

        // Restore the new tab's layout (or a fresh single pane), focusing primary.
        self.layout = self
            .stored_layouts
            .remove(&id)
            .unwrap_or_else(TabLayout::single);
        self.layout.focused = PaneId::Primary;

        // Reset secondary dimensions so resize_if_needed picks up the new layout.
        self.last_secondary_rows = 0;
        self.last_secondary_cols = 0;
    }

    /// Cycle keyboard focus to the next pane in the active tab's layout.
    pub fn toggle_split_focus(&mut self) {
        let leaves = self.layout.leaves();
        if leaves.len() <= 1 {
            return;
        }
        let cur = leaves
            .iter()
            .position(|&p| p == self.layout.focused)
            .unwrap_or(0);
        self.layout.focused = leaves[(cur + 1) % leaves.len()];
    }

    /// Move focus to a specific pane (used by click-to-focus on a passive pane).
    fn focus_pane(&mut self, id: PaneId) {
        if self.layout.leaves().contains(&id) {
            self.layout.focused = id;
        }
    }

    /// Look up the session backing a pane id within the active tab.
    fn session_for(&self, id: PaneId) -> Option<&TerminalSession> {
        match id {
            PaneId::Primary => self.pane.sessions.get(self.pane.active_index),
            PaneId::Extra(uuid) => self.layout.extra.get(&uuid),
        }
    }

    /// Switch to the next tab (wraps around).
    pub fn next_tab(&mut self) {
        if self.tabs.len() <= 1 {
            return;
        }
        let next = (self.pane.active_index + 1) % self.tabs.len();
        let id = self.tabs[next].id;
        self.select_tab(id);
    }

    /// Switch to the previous tab (wraps around).
    pub fn prev_tab(&mut self) {
        if self.tabs.len() <= 1 {
            return;
        }
        let prev = if self.pane.active_index == 0 {
            self.tabs.len() - 1
        } else {
            self.pane.active_index - 1
        };
        let id = self.tabs[prev].id;
        self.select_tab(id);
    }

    pub fn active_session(&self) -> Option<&TerminalSession> {
        self.session_for(self.layout.focused)
    }

    fn session_by_id(&self, id: Uuid) -> Option<&TerminalSession> {
        self.pane
            .sessions
            .iter()
            .find(|session| session.id == id)
            .or_else(|| self.layout.extra.values().find(|session| session.id == id))
            .or_else(|| {
                self.stored_layouts
                    .values()
                    .flat_map(|layout| layout.extra.values())
                    .find(|session| session.id == id)
            })
    }

    pub fn set_ai_actions_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.ai_actions_enabled = enabled;
        cx.notify();
    }

    pub fn set_ai_naming_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.ai_naming_enabled = enabled;
        cx.notify();
    }

    pub fn apply_ai_name(
        &mut self,
        expected_session: Uuid,
        name: String,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let Some(index) = self
            .pane
            .sessions
            .iter()
            .position(|session| session.id == expected_session)
        else {
            return Err("session_changed");
        };
        self.pane.sessions[index].title = name.clone();
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == expected_session) {
            tab.title = name;
            tab.named = true;
        }
        cx.notify();
        Ok(())
    }

    /// Insert a reviewed AI command without submitting it.
    pub fn insert_ai_command(
        &self,
        expected_session: Uuid,
        command: &str,
    ) -> Result<(), &'static str> {
        let command = validate_ai_command(command)?;
        let Some(session) = self.active_session() else {
            return Err("no_session");
        };
        if session.id != expected_session {
            return Err("session_changed");
        }
        session.write_input(command.as_bytes());
        Ok(())
    }

    /// Submit a separately-confirmed AI command to the exact active session.
    pub fn execute_ai_command(
        &mut self,
        expected_session: Uuid,
        command: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let command = validate_ai_command(command)?;
        let Some(session) = self.active_session() else {
            return Err("no_session");
        };
        if session.id != expected_session {
            return Err("session_changed");
        }
        let sequence = session.grid.lock().prompt_mark_sequence;
        session.write_tracked_command(command);
        self.observed_command_sequences
            .insert(expected_session, sequence);
        self.ai_running_command = Some(expected_session);
        cx.notify();
        Ok(())
    }

    pub fn stop_ai_command(
        &mut self,
        expected_session: Uuid,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        if self.ai_running_command != Some(expected_session) {
            return Err("action_changed");
        }
        let Some(session) = self.active_session() else {
            return Err("no_session");
        };
        if session.id != expected_session {
            return Err("session_changed");
        }
        session.write_input(b"\x03");
        self.ai_running_command = None;
        cx.notify();
        Ok(())
    }

    /// One row per open tab for the assistant's `@` picker.
    ///
    /// The output tail is a *snapshot*, taken here, with its capture time — the
    /// assistant sends what it showed the user, not a re-read at send time, and
    /// says when it was taken so a stale tail is legible as one. The grid lock
    /// is taken once per tab and released immediately.
    pub fn mention_sessions(&self) -> Vec<TerminalMentionRow> {
        let captured_at = chrono::Utc::now().timestamp_millis();
        self.tabs
            .iter()
            .map(|tab| {
                let session = self
                    .pane
                    .sessions
                    .iter()
                    .find(|session| session.id == tab.id);
                let (cwd, tail) = match session {
                    Some(session) => {
                        let grid = session.grid.lock();
                        let cwd = grid.working_directory.clone();
                        let tail = grid
                            .visible_rows()
                            .iter()
                            .rev()
                            .take(TERMINAL_MENTION_TAIL_ROWS)
                            .rev()
                            .map(|row| {
                                row.iter()
                                    .map(|cell| cell.c)
                                    .collect::<String>()
                                    .trim_end()
                                    .to_string()
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        (cwd, tail)
                    }
                    None => (None, String::new()),
                };
                TerminalMentionRow {
                    id: tab.id,
                    title: tab.title.clone(),
                    state: format!("{:?}", tab.state),
                    cwd,
                    connection_id: tab.connection_id,
                    captured_at,
                    tail,
                }
            })
            .collect()
    }

    pub fn ai_context_data(&self) -> serde_json::Value {
        let Some(session) = self.active_session() else {
            return serde_json::json!({ "terminal": null });
        };
        let grid = session.grid.lock();
        let selection = grid.selected_text();
        let cwd = grid.working_directory.clone();
        let rows = grid.visible_rows();
        let output = rows
            .iter()
            .rev()
            .take(120)
            .rev()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.c)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        serde_json::json!({
            "title": session.title,
            "state": format!("{:?}", session.state),
            "session_id": session.id,
            "cwd": cwd,
            "selection": selection,
            "visible_output": output,
        })
    }

    /// End every session daemon pane shown by this view (sign-out). Unlike
    /// [`close_all_sessions`](Self::close_all_sessions), which only
    /// detaches so the panes survive an app quit, this stops their
    /// processes: nothing may be offered back to the next account. No-op
    /// for PTY and SSH sessions; returns at once.
    pub fn end_daemon_panes(&self) {
        let extras = std::iter::once(&self.layout)
            .chain(self.stored_layouts.values())
            .flat_map(|layout| layout.extra.values());
        for session in self.pane.sessions.iter().chain(extras) {
            session.terminate();
        }
    }

    /// Close all terminal sessions for graceful shutdown.
    pub fn close_all_sessions(&mut self) {
        tracing::info!("Closing {} terminal sessions", self.pane.sessions.len());
        self.layout = TabLayout::single();
        self.stored_layouts.clear();
        self.tabs.clear();
        self.tab_folders.clear();
        self.recent_tabs.clear();
        self.pane.sessions.clear();
        self.pane.active_index = 0;
        self._refresh_task = None;
    }

    /// Return the number of open terminal tabs.
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Snapshot the open tabs for session persistence.
    ///
    /// Returns one entry per tab as `(title, connection_id)`. A `connection_id`
    /// of `Some` marks an SSH tab (to reconnect on restore); `None` marks a
    /// local shell tab. Read-only — does not mutate any state.
    pub fn session_states(&self) -> Vec<(String, Option<Uuid>)> {
        self.tabs
            .iter()
            .map(|t| (t.title.clone(), t.connection_id))
            .collect()
    }

    /// Index of the currently active tab (for restoring focus).
    pub fn active_tab_index(&self) -> usize {
        self.pane.active_index
    }

    /// Capture tab order, active tab, native split ratios/focus and viewport
    /// offsets without moving or cloning any live terminal session.
    pub(crate) fn capture_workspace_snapshot(&self) -> TerminalWorkspaceSnapshot {
        let mut layouts = HashMap::new();
        for tab in &self.tabs {
            let layout = if self
                .tabs
                .get(self.pane.active_index)
                .is_some_and(|active| active.id == tab.id)
            {
                &self.layout
            } else {
                self.stored_layouts.get(&tab.id).unwrap_or(&self.layout)
            };
            let mut viewports = HashMap::new();
            if let Some(session) = self.session_by_id(tab.id) {
                viewports.insert(session.id, session.grid.lock().scroll_info().2);
            }
            for session in layout.extra.values() {
                viewports.insert(session.id, session.grid.lock().scroll_info().2);
            }
            layouts.insert(
                tab.id,
                TerminalPaneSnapshot {
                    tree: snapshot_pane_node(&layout.tree),
                    focused: snapshot_pane_key(layout.focused),
                    viewports,
                },
            );
        }
        TerminalWorkspaceSnapshot {
            tabs: self.tabs.iter().map(|tab| tab.id).collect(),
            active_tab: self.tabs.get(self.pane.active_index).map(|tab| tab.id),
            layouts,
        }
    }

    /// Reapply a snapshot to sessions still owned by this entity. Unknown or
    /// closed tabs are ignored; no PTY is ever recreated as a side effect.
    pub(crate) fn apply_workspace_snapshot(&mut self, snapshot: &TerminalWorkspaceSnapshot) {
        if let Some(active) = snapshot
            .active_tab
            .filter(|id| self.tabs.iter().any(|tab| tab.id == *id))
        {
            self.select_tab(active);
        }
        for (tab_id, pane) in &snapshot.layouts {
            let layout = if self
                .tabs
                .get(self.pane.active_index)
                .is_some_and(|active| active.id == *tab_id)
            {
                Some(&mut self.layout)
            } else {
                self.stored_layouts.get_mut(tab_id)
            };
            let Some(layout) = layout else { continue };
            apply_pane_node_snapshot(&mut layout.tree, &pane.tree);
            let focused = restore_pane_key(pane.focused);
            if layout.leaves().contains(&focused) {
                layout.focused = focused;
            }
            for session in layout.extra.values() {
                if let Some(offset) = pane.viewports.get(&session.id) {
                    session.grid.lock().set_scroll_offset(*offset);
                }
            }
        }
        for session in &self.pane.sessions {
            if let Some(offset) = snapshot
                .layouts
                .values()
                .find_map(|pane| pane.viewports.get(&session.id))
            {
                session.grid.lock().set_scroll_offset(*offset);
            }
        }
    }

    /// Return the last computed grid dimensions, or a default if unknown.
    pub fn grid_size(&self) -> (u16, u16) {
        if self.last_grid_rows > 0 {
            (self.last_grid_rows, self.last_grid_cols)
        } else {
            (24, 80)
        }
    }

    /// Convert a GPUI KeyDownEvent into the byte sequence expected by a terminal.
    ///
    /// `app_cursor` indicates whether application cursor keys mode (DECCKM) is
    /// active on the grid. When true, arrow keys and Home/End emit SS3
    /// sequences instead of CSI sequences.
    fn keystroke_to_bytes(event: &KeyDownEvent, app_cursor: bool) -> Option<Vec<u8>> {
        let keystroke = &event.keystroke;
        let key = keystroke.key.as_str();
        let mods = &keystroke.modifiers;

        // Ctrl+key combos: letter & 0x1f produces the control character.
        // Skip when Shift is held so Ctrl+Shift+C/V reach the Copy/Paste actions.
        if mods.control && !mods.alt && !mods.shift && key.len() == 1 {
            let ch = key
                .chars()
                .next()
                .expect("key.len() == 1 guarantees a char");
            if ch.is_ascii_alphabetic() {
                return Some(vec![(ch.to_ascii_lowercase() as u8) & 0x1f]);
            }
        }

        // Compute xterm modifier code for modified special keys.
        // Shift=2, Alt=3, Shift+Alt=4, Ctrl=5, Shift+Ctrl=6, Alt+Ctrl=7,
        // Shift+Alt+Ctrl=8.
        let modifier_code = || -> Option<u8> {
            let val = 1
                + if mods.shift { 1 } else { 0 }
                + if mods.alt { 2 } else { 0 }
                + if mods.control { 4 } else { 0 };
            if val > 1 {
                Some(val)
            } else {
                None
            }
        };

        // ---- Function keys F1..F24 ----
        if let Some(fnum) = key.strip_prefix('f').and_then(|s| s.parse::<u8>().ok()) {
            if (1..=24).contains(&fnum) {
                return Some(Self::function_key_bytes(fnum, modifier_code()));
            }
        }

        match key {
            "enter" => Some(b"\r".to_vec()),
            "backspace" => Some(vec![0x7f]),
            "tab" => Some(b"\t".to_vec()),
            "escape" => Some(vec![0x1b]),

            // Arrow keys: SS3 in application cursor mode, CSI otherwise.
            // With modifiers always use CSI form: \x1b[1;{mod}{final}
            "up" | "down" | "right" | "left" => {
                let final_byte = match key {
                    "up" => b'A',
                    "down" => b'B',
                    "right" => b'C',
                    "left" => b'D',
                    _ => unreachable!(),
                };
                if let Some(m) = modifier_code() {
                    Some(format!("\x1b[1;{}{}", m, final_byte as char).into_bytes())
                } else if app_cursor {
                    Some(vec![0x1b, b'O', final_byte])
                } else {
                    Some(vec![0x1b, b'[', final_byte])
                }
            }

            // Home / End: SS3 in application cursor mode, CSI otherwise.
            "home" => {
                if let Some(m) = modifier_code() {
                    Some(format!("\x1b[1;{}H", m).into_bytes())
                } else if app_cursor {
                    Some(b"\x1bOH".to_vec())
                } else {
                    Some(b"\x1b[H".to_vec())
                }
            }
            "end" => {
                if let Some(m) = modifier_code() {
                    Some(format!("\x1b[1;{}F", m).into_bytes())
                } else if app_cursor {
                    Some(b"\x1bOF".to_vec())
                } else {
                    Some(b"\x1b[F".to_vec())
                }
            }

            "insert" => Some(b"\x1b[2~".to_vec()),
            "delete" => Some(b"\x1b[3~".to_vec()),
            "pageup" => Some(b"\x1b[5~".to_vec()),
            "pagedown" => Some(b"\x1b[6~".to_vec()),
            "space" => {
                // Alt+Space sends ESC followed by space
                if mods.alt {
                    Some(b"\x1b ".to_vec())
                } else {
                    Some(b" ".to_vec())
                }
            }
            _ => {
                // Alt+key: send ESC prefix before the character.
                if mods.alt && !mods.control {
                    if let Some(ref kc) = keystroke.key_char {
                        let mut bytes = vec![0x1b];
                        bytes.extend_from_slice(kc.as_bytes());
                        return Some(bytes);
                    } else if key.len() == 1 {
                        let mut bytes = vec![0x1b];
                        bytes.extend_from_slice(key.as_bytes());
                        return Some(bytes);
                    }
                }

                // Use key_char for typed characters (handles shift etc.)
                if let Some(ref kc) = keystroke.key_char {
                    Some(kc.as_bytes().to_vec())
                } else if key.len() == 1 {
                    Some(key.as_bytes().to_vec())
                } else {
                    None
                }
            }
        }
    }

    /// Build the escape sequence for a function key F1..F24, optionally with
    /// an xterm modifier code.
    fn function_key_bytes(fnum: u8, modifier: Option<u8>) -> Vec<u8> {
        // F1-F4 use SS3 finals P/Q/R/S (no modifier) or CSI 1;mod P/Q/R/S
        // F5+ use CSI code ~ format
        match fnum {
            1..=4 => {
                let final_ch = match fnum {
                    1 => 'P',
                    2 => 'Q',
                    3 => 'R',
                    4 => 'S',
                    _ => unreachable!(),
                };
                if let Some(m) = modifier {
                    format!("\x1b[1;{}{}", m, final_ch).into_bytes()
                } else {
                    format!("\x1bO{}", final_ch).into_bytes()
                }
            }
            5..=24 => {
                let code = match fnum {
                    5 => 15,
                    6 => 17,
                    7 => 18,
                    8 => 19,
                    9 => 20,
                    10 => 21,
                    11 => 23,
                    12 => 24,
                    13 => 25,
                    14 => 26,
                    15 => 28,
                    16 => 29,
                    17 => 31,
                    18 => 32,
                    19 => 33,
                    20 => 34,
                    21 => 42,
                    22 => 43,
                    23 => 44,
                    24 => 45,
                    _ => unreachable!(),
                };
                if let Some(m) = modifier {
                    format!("\x1b[{};{}~", code, m).into_bytes()
                } else {
                    format!("\x1b[{}~", code).into_bytes()
                }
            }
            _ => Vec::new(),
        }
    }

    /// Compute terminal grid dimensions (rows, cols) from the window viewport.
    /// Current UI scale factor, derived from the window rem size set by the
    /// workspace from the "App Font Size" setting. 1.0 at the default size.
    fn ui_scale(window: &Window) -> f32 {
        (window.rem_size().to_f64() as f32 / crate::scale::REM_BASE).clamp(0.6, 2.0)
    }

    /// Height of the rows above the panes (tab strip, toolbar) in absolute
    /// pixels at the current UI scale. This is the terminal grid's top
    /// offset within the content area.
    fn chrome_top_offset(&self, window: &Window) -> f32 {
        chrome_rows_height(self.session_list_shown()) * Self::ui_scale(window)
    }

    /// Height of a pane header in absolute pixels: 0 in a tab that is not
    /// split, where the tab already names the only pane.
    fn pane_header_height(&self, window: &Window) -> f32 {
        if self.layout.is_split() {
            layout::PANE_HEADER_HEIGHT * Self::ui_scale(window)
        } else {
            0.0
        }
    }

    /// Terminal cell size in pixels (width, height) at the effective font size.
    fn cell_size(&self) -> (f32, f32) {
        let fs = self.effective_font_size();
        (fs * 0.6, fs * 1.4)
    }

    /// The full terminal content rectangle (in the grid offset convention:
    /// x = right of the sidebar, y = below the scaled tab bar + toolbar).
    fn content_area(&self, window: &Window) -> PaneRect {
        let viewport = window.viewport_size();
        let x = self.content_left();
        let y = self.chrome_top_offset(window);
        if let Some((w, h)) = self.pane_layer_size {
            return PaneRect { x, y, w, h };
        }
        let w = (viewport.width.to_f64() as f32 - x - SIDEBAR_HANDLE_WIDTH).max(1.0);
        let h = (viewport.height.to_f64() as f32
            - TITLEBAR_HEIGHT * Self::ui_scale(window)
            - self.chrome_top_offset(window)
            - STATUS_BAR_HEIGHT)
            .max(1.0);
        PaneRect { x, y, w, h }
    }

    /// The surface the tab strip, the toolbar and the pane cards sit on: the
    /// chrome colour, a shade toward the text colour so a card stands out
    /// on it in a light theme as in a dark one. Painted over the view's
    /// own `bg_sidebar` background.
    fn canvas_tint() -> Hsla {
        ShellDeckColors::text_primary().opacity(0.06)
    }

    /// The rectangle the pane cards are laid out in: the content area
    /// without the margin kept around the cards.
    fn panes_area(&self, window: &Window) -> PaneRect {
        layout::inset_rect(self.content_area(window), layout::PANE_MARGIN)
    }

    /// Convert a pixel rect into grid (rows, cols).
    fn rect_to_grid(&self, rect: PaneRect) -> (u16, u16) {
        let (cw, ch) = self.cell_size();
        let cols = (rect.w / cw).floor() as u16;
        let rows = (rect.h / ch).floor() as u16;
        (rows.max(2), cols.max(10))
    }

    /// Recompute every pane's grid size and resize its session to match.
    fn resize_if_needed(&mut self, window: &Window) {
        let area = self.panes_area(window);
        let header = self.pane_header_height(window);
        let (leaves, _) = self.layout.compute(area, SPLIT_DIVIDER_SIZE);

        // Track the focused pane's size for spawn defaults / `grid_size()`.
        if let Some((_, rect)) = leaves.iter().find(|(id, _)| *id == self.layout.focused) {
            let (rows, cols) = self.rect_to_grid(layout::grid_rect(*rect, header));
            self.last_grid_rows = rows;
            self.last_grid_cols = cols;
        }

        for (id, rect) in leaves {
            let (rows, cols) = self.rect_to_grid(layout::grid_rect(rect, header));
            if let Some(session) = self.session_for(id) {
                session.resize(rows, cols);
            }
        }
    }

    pub fn spawn_local_terminal(&mut self, cx: &mut Context<Self>) {
        let (rows, cols) = if self.last_grid_rows > 0 {
            (self.last_grid_rows, self.last_grid_cols)
        } else {
            (24, 80)
        };
        match self.spawn_configured_local(rows, cols) {
            Ok(session) => {
                self.add_session(session);
                tracing::info!("Spawned new local terminal");
            }
            Err(e) => {
                tracing::error!("Failed to spawn terminal: {}", e);
            }
        }

        self.ensure_refresh_running(cx);
    }

    /// Starts the PTY while keeping it detached from terminal/UI state. The
    /// workspace catalog can then durably commit; dropping this value on a
    /// failed catalog write closes the PTY and reaps its child.
    pub(crate) fn prepare_authorized_local_terminal(
        &mut self,
        root: &AuthorizedLocalRoot,
    ) -> Result<PreparedLocalTerminal, String> {
        root.revalidate()?;
        let (rows, cols) = if self.last_grid_rows > 0 {
            (self.last_grid_rows, self.last_grid_cols)
        } else {
            (24, 80)
        };
        let authority_cwd = root.command_path();
        if !authority_cwd.is_dir() {
            return Err("authorized terminal working directory is unavailable".into());
        }
        let session = TerminalSession::spawn_local_at_authorized(
            self.default_shell.as_deref(),
            rows,
            cols,
            &authority_cwd,
            root.path(),
        )
        .map_err(|error| error.to_string())?;
        root.revalidate()?;
        Ok(PreparedLocalTerminal {
            session,
            authority: root.clone(),
        })
    }

    pub(crate) fn commit_prepared_local_terminal(
        &mut self,
        prepared: PreparedLocalTerminal,
        cx: &mut Context<Self>,
    ) -> Uuid {
        let PreparedLocalTerminal { session, authority } = prepared;
        self.install_authorized_default_cwd(&authority);
        let id = session.id;
        self.add_session(session);
        self.ensure_refresh_running(cx);
        id
    }

    /// Run a local CLI in the active terminal, creating one when necessary.
    fn launch_cli(&mut self, command: &str, label: &str, cx: &mut Context<Self>) {
        let spawned_terminal = self.active_session().is_none();
        if spawned_terminal {
            self.spawn_local_terminal(cx);
        }

        if let Some(session) = self.active_session() {
            let mut input = command.as_bytes().to_vec();
            input.push(b'\n');
            session.write_input(&input);
        }

        if spawned_terminal {
            cx.emit(TerminalEvent::NewTabRequested);
        }
        tracing::info!("Launched {} in the active terminal", label);
        cx.notify();
    }

    /// Start Claude Code in the current terminal, or in a new local terminal.
    pub fn launch_claude(&mut self, cx: &mut Context<Self>) {
        self.launch_cli(CLAUDE_CLI_COMMAND, "Claude Code", cx);
    }

    /// Start Codex with workspace writes and approvals requested when needed.
    pub fn launch_codex(&mut self, cx: &mut Context<Self>) {
        self.launch_cli(CODEX_CLI_COMMAND, "Codex", cx);
    }

    /// Start the periodic refresh loop if it is not already running.
    ///
    /// Repaints only when a grid is actually dirty (new data arrived). The poll
    /// interval adapts: ~60 Hz while output is flowing, backing off to ~25 Hz
    /// after roughly half a second of quiet so an idle terminal costs almost
    /// nothing. Any new output is still picked up within one idle interval.
    pub fn ensure_refresh_running(&mut self, cx: &mut Context<Self>) {
        if self._refresh_task.is_some() {
            return;
        }
        let mut rx = match self.output_rx.take() {
            Some(rx) => rx,
            None => {
                // The previous loop was stopped (the last tab was closed)
                // and its receiver went with it. Without a new channel no
                // loop would ever run again and the next terminals would
                // repaint only on input events: output of a running
                // command, and everything a session daemon pane prints,
                // would stay invisible until a key press or a mouse move.
                let (output_tx, output_rx) = mpsc::unbounded_channel::<()>();
                self.output_tx = output_tx;
                let extras = std::iter::once(&self.layout)
                    .chain(self.stored_layouts.values())
                    .flat_map(|layout| layout.extra.values());
                for session in self.pane.sessions.iter().chain(extras) {
                    session.set_output_notifier(self.output_tx.clone());
                }
                output_rx
            }
        };
        self._refresh_task = Some(cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                loop {
                    // Block until a reader thread signals new output, or wake every
                    // 250 ms as a safety net. Idle terminals cost ~4 cheap wake-ups
                    // a second instead of polling at 60-120 Hz.
                    let _ = rx
                        .recv()
                        .with_timeout(
                            std::time::Duration::from_millis(250),
                            cx.background_executor(),
                        )
                        .await;
                    // Coalesce a burst of output into a single repaint: a screen
                    // update (e.g. htop) arrives as several PTY chunks over a few
                    // milliseconds, so wait one frame and drain them all, repainting
                    // once instead of once per chunk. Caps the repaint rate at ~60 Hz
                    // under continuous output while staying fully responsive.
                    while rx.try_recv().is_ok() {}
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(16))
                        .await;
                    while rx.try_recv().is_ok() {}

                    let result = this.update(cx, |this, cx| {
                        let mut any_dirty = false;
                        let mut any_sync = false;
                        // What the pane commands asked of the application
                        // (`shelldeck-sessiond split`, `focus`, `rename`,
                        // `zoom`).
                        if this.restores_daemon_panes() {
                            for (id, ask) in daemon::take_requests() {
                                let result = match &ask {
                                    daemon::PaneAsk::Split(split) => this.adopt_daemon_split(split),
                                    daemon::PaneAsk::Act { pane, action } => {
                                        this.act_on_daemon_pane(*pane, action, cx)
                                    }
                                };
                                if let Err(reason) = &result {
                                    tracing::info!("pane request {ask:?} not done: {reason}");
                                }
                                daemon::answer_request(id, result);
                                any_dirty = true;
                            }
                        }
                        for s in this.pane.sessions.iter() {
                            let g = s.grid.lock();
                            if g.dirty {
                                any_dirty = true;
                            }
                            if g.synchronized_output() {
                                any_sync = true;
                            }
                        }
                        // Session daemon panes: the tab follows the phase
                        // its session reports (ended, lost, unavailable).
                        for (index, s) in this.pane.sessions.iter_mut().enumerate() {
                            let Some(phase) = s.attached_status().map(|status| status.phase())
                            else {
                                continue;
                            };
                            let state = layout::attached_tab_state(
                                &phase,
                                &t!("terminal.session_daemon.ended"),
                                &t!("terminal.session_daemon.unavailable"),
                                &t!("terminal.session_daemon.other_version_tab"),
                            );
                            if s.state != state {
                                s.state = state.clone();
                                if let Some(tab) = this.tabs.get_mut(index) {
                                    tab.state = state;
                                }
                                any_dirty = true;
                            }
                        }
                        // Follow the title each program sets (OSC 0 / OSC 2).
                        for s in this.pane.sessions.iter() {
                            let g = s.grid.lock();
                            if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == s.id) {
                                if tab.program_title != g.title {
                                    tab.program_title.clone_from(&g.title);
                                    any_dirty = true;
                                }
                            }
                        }
                        // Follow the folder each tab works in (projects).
                        if this.follow_tab_folders(cx) {
                            any_dirty = true;
                        }
                        // Follow what each program says it is doing (OSC 7501).
                        let before = this.attention();
                        for index in 0..this.tabs.len() {
                            let headline = this.tab_headline(this.tabs[index].id);
                            if this.tabs[index].set_status(headline) {
                                any_dirty = true;
                            }
                        }
                        if this.attention() != before {
                            cx.emit(TerminalEvent::AttentionChanged);
                        }
                        // Input the session daemon refused (pane busy):
                        // reported once per wake, across tabs and splits.
                        let dropped: u32 = this
                            .pane
                            .sessions
                            .iter()
                            .chain(this.layout.extra.values())
                            .filter_map(|s| s.attached_status())
                            .map(|status| status.take_dropped_input())
                            .sum();
                        if dropped > 0 {
                            cx.emit(TerminalEvent::InputDropped { count: dropped });
                        }
                        // Extra (split) panes of the active tab.
                        for session in this.layout.extra.values() {
                            let g = session.grid.lock();
                            if g.dirty {
                                any_dirty = true;
                            }
                            if g.synchronized_output() {
                                any_sync = true;
                            }
                        }
                        if let Some(session_id) = this.ai_running_command {
                            let completion = this.session_by_id(session_id).and_then(|session| {
                                let grid = session.grid.lock();
                                let exit_code = match grid.prompt_mark {
                                    Some(
                                        shelldeck_terminal::grid::PromptMark::CommandFinished(code),
                                    ) => code,
                                    _ => return None,
                                };
                                let sequence = grid.prompt_mark_sequence;
                                let output = grid.command_output(120, 32_000);
                                Some((sequence, exit_code, output))
                            });
                            if let Some((sequence, exit_code, output)) = completion {
                                let observed = this
                                    .observed_command_sequences
                                    .entry(session_id)
                                    .or_default();
                                if sequence > *observed {
                                    *observed = sequence;
                                    this.ai_running_command = None;
                                    cx.emit(TerminalEvent::AiCommandFinished {
                                        session_id,
                                        exit_code,
                                        output,
                                    });
                                }
                            }
                        }
                        // Clear dirty flags on stored (background) tabs' split
                        // sessions so they don't accumulate stale state, without
                        // triggering a repaint for background tabs.
                        for layout in this.stored_layouts.values() {
                            for session in layout.extra.values() {
                                session.grid.lock().dirty = false;
                            }
                        }
                        // Handle OSC 52 clipboard requests from any visible session.
                        for session in this.pane.sessions.iter() {
                            if let Some((_sel, text)) = session.grid.lock().clipboard_request.take()
                            {
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                            }
                        }
                        for session in this.layout.extra.values() {
                            if let Some((_sel, text)) = session.grid.lock().clipboard_request.take()
                            {
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                            }
                        }
                        // Suppress repaint while synchronized output is active
                        // (batching updates to prevent flicker). When the app turns
                        // sync off, dirty is set and any_sync cleared.
                        let should_refresh = any_dirty && !any_sync;
                        if should_refresh {
                            cx.notify();
                        }
                        should_refresh
                    });
                    match result {
                        Ok(true) => {
                            // The grid is mutated by the PTY/SSH reader outside
                            // GPUI's entity update cycle. Explicitly schedule a
                            // frame as well as invalidating this entity; otherwise
                            // some platforms only paint after the next input event.
                            if cx.refresh().is_err() {
                                break;
                            }
                        }
                        Ok(false) => {}
                        Err(_) => break,
                    }
                }
            },
        ));
    }

    /// Ensure the glyph cache is populated for the current font + zoom level.
    /// Terminal cells stay at the historical `1.4` multiplier — the editor
    /// gets its own airy multiplier via `EditorConfig.line_height`.
    fn ensure_glyph_cache(&mut self, window: &Window) {
        if self.glyph_cache.is_none() {
            let fs = self.effective_font_size();
            self.glyph_cache = Some(Arc::new(GlyphCache::build(
                window.text_system(),
                &self.font_family,
                fs,
                1.4,
            )));
        }
    }

    /// A session daemon pane that is not attached yet (or again) is neither
    /// running nor ended.
    fn tab_pending(&self, tab_id: Uuid) -> bool {
        self.session_by_id(tab_id).is_some_and(|session| {
            session.attached_status().is_some_and(|status| {
                matches!(
                    status.phase(),
                    shelldeck_terminal::attached::AttachedPhase::Connecting
                        | shelldeck_terminal::attached::AttachedPhase::Reconnecting
                )
            })
        })
    }

    /// Colour and size (before UI scaling) of a tab's dot. A running session
    /// shows what its program reports (OSC 7501); a report that asks for the
    /// user also makes the dot larger.
    fn tab_dot(tab: &TerminalTab, pending: bool) -> (Hsla, f32) {
        let tone = tab.status_tone();
        let color = match &tab.state {
            SessionState::Running if pending => ShellDeckColors::warning(),
            SessionState::Running => match tone {
                StatusTone::NeedsYou => ShellDeckColors::warning(),
                StatusTone::Failed => ShellDeckColors::error(),
                StatusTone::Working | StatusTone::Finished => ShellDeckColors::primary(),
                StatusTone::Plain => ShellDeckColors::success(),
            },
            SessionState::Exited(0) => ShellDeckColors::text_muted(),
            SessionState::Exited(_) => ShellDeckColors::warning(),
            SessionState::Error(_) => ShellDeckColors::error(),
        };
        let size = match tone {
            StatusTone::NeedsYou | StatusTone::Failed | StatusTone::Finished => 8.0,
            StatusTone::Working | StatusTone::Plain => 6.0,
        };
        (color, size)
    }

    /// One terminal in the session list: its dot, its name, a mark when it
    /// is split, and its close button.
    fn render_session_row(
        &self,
        index: usize,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let s = |v: f32| px(v * scale);
        let tab = self.tabs.get(index)?;
        let tab_id = tab.id;
        let (dot_color, dot_size) = Self::tab_dot(tab, self.tab_pending(tab_id));
        let split = if tab.is_active {
            self.layout.is_split()
        } else {
            self.stored_layouts
                .get(&tab_id)
                .is_some_and(|layout| layout.is_split())
        };
        let mut row = div()
            .id(ElementId::from(SharedString::from(format!(
                "session-row-{tab_id}"
            ))))
            .flex()
            .items_center()
            .gap(s(8.0))
            .h(s(28.0))
            .mx(s(6.0))
            .pl(s(10.0))
            .pr(s(4.0))
            .rounded(s(6.0))
            .cursor_pointer()
            .overflow_hidden()
            .on_click(cx.listener(move |this, _event: &ClickEvent, window, cx| {
                this.select_tab(tab_id);
                this.needs_focus = true;
                this.focus_handle.focus(window);
                cx.emit(TerminalEvent::TabSelected(tab_id));
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    this.context_menu = None;
                    this.tab_context_menu = Some(TabMenuState {
                        position: event.position,
                        tab_id,
                    });
                    cx.notify();
                }),
            );
        row = if tab.is_active {
            row.bg(ShellDeckColors::primary().opacity(0.14))
                .text_color(ShellDeckColors::text_primary())
        } else {
            row.text_color(ShellDeckColors::text_muted())
                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
        };
        // The dot keeps one slot whatever its size, so titles align.
        row = row.child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .w(s(8.0))
                .child(
                    div()
                        .w(s(dot_size))
                        .h(s(dot_size))
                        .rounded_full()
                        .bg(dot_color),
                ),
        );
        row = row.child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(s(12.0))
                .child(tab.display_title().to_string()),
        );
        if split {
            row = row.child(
                svg()
                    .path("icons/lucide/columns-2.svg")
                    .size(s(12.0))
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::text_muted()),
            );
        }
        row = row.child(
            div()
                .id(ElementId::from(SharedString::from(format!(
                    "session-close-{tab_id}"
                ))))
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .w(s(20.0))
                .h(s(20.0))
                .rounded(s(4.0))
                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                .child(
                    svg()
                        .path("icons/lucide/x.svg")
                        .size(s(11.0))
                        .text_color(ShellDeckColors::text_muted()),
                )
                .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                    cx.stop_propagation();
                    this.close_tab(tab_id);
                    cx.emit(TerminalEvent::TabClosed(tab_id));
                    cx.notify();
                })),
        );
        if let Some(status) = &tab.status {
            let label: SharedString = layout::status_tooltip(status).into();
            row = row.tooltip(move |_, cx| {
                cx.new(|_| TerminalToolbarTooltip {
                    label: label.clone(),
                })
                .into()
            });
        }
        Some(row)
    }

    /// The button of the session list header that switches between the
    /// grouping by machine and by project. It shows the grouping it leads to.
    fn render_grouping_button(&self, scale: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let s = |v: f32| px(v * scale);
        let (next, icon, label) = match self.session_grouping {
            SessionGrouping::Machine => (
                SessionGrouping::Project,
                "icons/lucide/folder.svg",
                t!("terminal.sessions.group_by_project"),
            ),
            SessionGrouping::Project => (
                SessionGrouping::Machine,
                "icons/lucide/server.svg",
                t!("terminal.sessions.group_by_machine"),
            ),
        };
        let label: SharedString = label.to_string().into();
        div()
            .id("session-list-grouping")
            .flex()
            .items_center()
            .justify_center()
            .w(s(26.0))
            .h(s(26.0))
            .rounded(s(6.0))
            .cursor_pointer()
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            .child(
                svg()
                    .path(icon)
                    .size(s(14.0))
                    .text_color(ShellDeckColors::text_muted()),
            )
            .tooltip(move |_, cx| {
                cx.new(|_| TerminalToolbarTooltip {
                    label: label.clone(),
                })
                .into()
            })
            .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                this.session_grouping = next;
                cx.emit(TerminalEvent::SessionGroupingChanged(next));
                cx.notify();
            }))
    }

    /// The header of one project in the session list: the folder name, the
    /// path that holds it (after the machine, for a project on a server),
    /// and how many terminals work there. A click folds its terminals.
    fn render_project_header(
        &self,
        group: &projects::ProjectGroup,
        collapsed: bool,
        scale: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let s = |v: f32| px(v * scale);
        let (id, name, place) = match &group.key {
            projects::ProjectKey::None => (
                "session-project-none".to_string(),
                t!("terminal.sessions.no_project").to_string(),
                None,
            ),
            projects::ProjectKey::Folder {
                connection_id,
                root,
            } => {
                let home = self.home_dir.as_deref().filter(|_| connection_id.is_none());
                let (name, parent) = projects::project_label(root, home);
                let machine = connection_id.map(|_| {
                    group
                        .host
                        .clone()
                        .unwrap_or_else(|| t!("terminal.sessions.remote").to_string())
                });
                let place = match (machine, parent) {
                    (Some(machine), Some(parent)) => Some(format!("{machine} · {parent}")),
                    (Some(machine), None) => Some(machine),
                    (None, parent) => parent,
                };
                let owner = connection_id.map(|id| id.to_string()).unwrap_or_default();
                (format!("session-project-{owner}-{root}"), name, place)
            }
        };
        let chevron = if collapsed {
            "icons/lucide/chevron-right.svg"
        } else {
            "icons/lucide/chevron-down.svg"
        };
        let key = group.key.clone();
        let mut header = div()
            .id(ElementId::from(SharedString::from(id)))
            .flex()
            .items_center()
            .gap(s(6.0))
            .h(s(26.0))
            .mx(s(6.0))
            .pl(s(6.0))
            .pr(s(8.0))
            .mt(s(4.0))
            .rounded(s(6.0))
            .cursor_pointer()
            .overflow_hidden()
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                if !this.collapsed_projects.remove(&key) {
                    this.collapsed_projects.insert(key.clone());
                }
                cx.notify();
            }))
            .child(
                svg()
                    .path(chevron)
                    .size(s(12.0))
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::text_muted()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(s(120.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(s(12.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(name),
            );
        // The path takes what is left and is cut first: the name and the
        // count stay readable.
        header = header.child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(s(11.0))
                .text_color(ShellDeckColors::text_muted())
                .child(place.unwrap_or_default()),
        );
        header.child(
            div()
                .flex_shrink_0()
                .text_size(s(11.0))
                .text_color(ShellDeckColors::text_muted())
                .child(group.tabs.len().to_string()),
        )
    }

    /// The terminals as a list grouped by machine or by project: what the
    /// tab strip shows, with room for names and for many sessions. It
    /// replaces the strip while it is shown.
    fn render_session_list(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = Self::ui_scale(window);
        let s = |v: f32| px(v * scale);

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .flex_shrink_0()
            .h(s(SESSION_LIST_HEADER_HEIGHT))
            .pl(s(12.0))
            .pr(s(6.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .text_size(s(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("terminal.sessions.title").to_string()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(s(2.0))
                    .child(self.render_grouping_button(scale, cx))
                    .child(self.render_actions_button(window, cx))
                    .child(
                        div()
                            .id("session-list-new")
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(s(26.0))
                            .h(s(26.0))
                            .rounded(s(6.0))
                            .cursor_pointer()
                            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                            .child(
                                svg()
                                    .path("icons/lucide/plus.svg")
                                    .size(s(14.0))
                                    .text_color(ShellDeckColors::text_muted()),
                            )
                            .on_click(cx.listener(|this, _event: &ClickEvent, window, cx| {
                                this.spawn_local_terminal(cx);
                                this.focus_handle.focus(window);
                                cx.emit(TerminalEvent::NewTabRequested);
                                cx.notify();
                            })),
                    ),
            );

        let mut rows = div()
            .id("terminal-session-rows")
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .py(s(4.0));

        match self.session_grouping {
            SessionGrouping::Machine => {
                for group in layout::session_groups(&self.tabs) {
                    let (icon, name) = match (&group.connection_id, &group.host) {
                        (None, _) => (
                            "icons/lucide/terminal.svg",
                            t!("terminal.sessions.this_machine").to_string(),
                        ),
                        (Some(_), Some(host)) => ("icons/lucide/server.svg", host.clone()),
                        (Some(_), None) => (
                            "icons/lucide/server.svg",
                            t!("terminal.sessions.remote").to_string(),
                        ),
                    };
                    rows = rows.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(s(6.0))
                            .h(s(26.0))
                            .px(s(12.0))
                            .mt(s(4.0))
                            .overflow_hidden()
                            .child(
                                svg()
                                    .path(icon)
                                    .size(s(12.0))
                                    .flex_shrink_0()
                                    .text_color(ShellDeckColors::text_muted()),
                            )
                            .child(
                                div()
                                    .min_w(px(0.0))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_size(s(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(name),
                            ),
                    );
                    for index in group.tabs {
                        rows = rows.children(self.render_session_row(index, scale, cx));
                    }
                }
            }
            SessionGrouping::Project => {
                for group in projects::project_groups(&self.tab_places()) {
                    let collapsed = self.collapsed_projects.contains(&group.key);
                    rows = rows.child(self.render_project_header(&group, collapsed, scale, cx));
                    if collapsed {
                        continue;
                    }
                    for index in group.tabs {
                        rows = rows.children(self.render_session_row(index, scale, cx));
                    }
                }
            }
        }

        div()
            .id("terminal-session-list")
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(px(self.session_list_width))
            .h_full()
            .bg(ShellDeckColors::bg_sidebar())
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .overflow_hidden()
            .child(header)
            .child(rows)
    }

    /// The header of one pane of a split tab: what runs there (a name and
    /// its folder), and on the focused pane what can be done with it.
    fn render_pane_header(
        &self,
        id: PaneId,
        focused: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let scale = Self::ui_scale(window);
        let s = |v: f32| px(v * scale);
        let (title, cwd, dot) = match self.session_for(id) {
            Some(session) => {
                let grid = session.grid.lock();
                let program = grid.title.trim();
                let title = if program.is_empty() {
                    session.title.clone()
                } else {
                    program.to_string()
                };
                // A dot only for a status the program reported.
                let dot = match grid.program_status.headline().map(|h| h.state) {
                    Some(ProgramState::Blocked) => Some(ShellDeckColors::warning()),
                    Some(ProgramState::Error) => Some(ShellDeckColors::error()),
                    Some(ProgramState::Working) => Some(ShellDeckColors::primary()),
                    Some(ProgramState::Done) => Some(ShellDeckColors::success()),
                    Some(ProgramState::Idle) | None => None,
                };
                (title, grid.working_directory.clone(), dot)
            }
            None => (String::new(), None, None),
        };
        let home = shelldeck_core::util::home_dir();
        let home = home.as_deref().and_then(|home| home.to_str());
        let (name, folder) = layout::title_parts(&title, cwd.as_deref(), home);
        let focus = self.focus_handle.clone();
        // The header is drawn on the card, which has the terminal theme's
        // background: its ink comes from that theme, not from the app's,
        // or a dark terminal in a light app would hide it.
        let foreground = self.palette.foreground_color();
        let quiet = foreground.opacity(0.6);
        let ink = if focused { foreground } else { quiet };
        // No background of its own: the card shows through, and its border
        // tells the focused pane apart.
        let mut header = div()
            .id(ElementId::from(SharedString::from(format!(
                "pane-header-{id:?}"
            ))))
            .flex()
            .items_center()
            .flex_shrink_0()
            .gap(s(8.0))
            .w_full()
            .h(px(self.pane_header_height(window)))
            .pl(s(12.0))
            .pr(s(8.0))
            .overflow_hidden()
            .text_color(ink)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                    this.focus_pane(id);
                    this.needs_focus = true;
                    focus.focus(window);
                    cx.notify();
                }),
            )
            .child(
                svg()
                    .path("icons/lucide/terminal.svg")
                    .size(s(13.0))
                    .flex_shrink_0()
                    .text_color(ink),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .max_w(relative(0.6))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(s(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(name),
            );
        if let Some(folder) = folder {
            header = header.child(
                div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(s(12.0))
                    .text_color(quiet)
                    .child(folder),
            );
        }
        if let Some(dot) = dot {
            header = header.child(
                div()
                    .w(s(7.0))
                    .h(s(7.0))
                    .rounded_full()
                    .flex_shrink_0()
                    .bg(dot),
            );
        }
        header = header.child(div().flex_1().min_w(px(0.0)));
        if !focused {
            return header.cursor_pointer();
        }

        let zoomed = self.layout.is_zoomed();
        let button = |name: &'static str, icon: &'static str, label: String| {
            let label: SharedString = label.into();
            div()
                .id(name)
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .w(s(22.0))
                .h(s(22.0))
                .rounded(s(6.0))
                .cursor_pointer()
                .hover(move |el| el.bg(foreground.opacity(0.12)))
                .tooltip(move |_, cx| {
                    cx.new(|_| TerminalToolbarTooltip {
                        label: label.clone(),
                    })
                    .into()
                })
                .child(svg().path(icon).size(s(13.0)).text_color(quiet))
        };
        let with_keys = |label: String, action: BindableAction| match shortcut_label(action) {
            Some(keys) => format!("{label} ({keys})"),
            None => label,
        };
        header
            .child(
                button(
                    "pane-split-right",
                    "icons/lucide/columns-2.svg",
                    with_keys(
                        t!("terminal.toolbar.split_h").to_string(),
                        BindableAction::SplitHorizontal,
                    ),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.split_horizontal(cx);
                    cx.notify();
                })),
            )
            .child(
                button(
                    "pane-split-down",
                    "icons/lucide/rows-2.svg",
                    with_keys(
                        t!("terminal.toolbar.split_v").to_string(),
                        BindableAction::SplitVertical,
                    ),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.split_vertical(cx);
                    cx.notify();
                })),
            )
            .child(
                button(
                    "pane-zoom",
                    if zoomed {
                        "icons/lucide/minimize-2.svg"
                    } else {
                        "icons/lucide/maximize-2.svg"
                    },
                    with_keys(
                        if zoomed {
                            t!("terminal.pane.restore").to_string()
                        } else {
                            t!("terminal.pane.maximize").to_string()
                        },
                        BindableAction::MaximizePane,
                    ),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.toggle_pane_zoom();
                    cx.notify();
                })),
            )
            .child(
                button(
                    "pane-close",
                    "icons/lucide/x.svg",
                    t!("terminal.toolbar.close_split").to_string(),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.close_split();
                    cx.notify();
                })),
            )
    }

    fn render_tab_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Everything in the tab bar scales with the UI size. We render in
        // absolute pixels multiplied by `scale` (rather than rems) so the
        // grid-offset math, which is also absolute, stays perfectly in step.
        let scale = Self::ui_scale(window);
        let s = |v: f32| px(v * scale);

        // Compute a per-tab width that shrinks toward MIN_TAB_WIDTH as more tabs
        // open; once at the minimum, the strip scrolls (overflow_x_scroll).
        let n = self.tabs.len().max(1) as f32;
        let content_w = (window.viewport_size().width.to_f64() as f32) - self.content_left();
        // New-tab button, agent launchers, actions button and paddings.
        let launchers = self.claude_available as u8 + self.codex_available as u8;
        let reserve = (92.0 + 32.0 * f32::from(launchers)) * scale;
        let usable = (content_w - reserve).max(MIN_TAB_WIDTH * scale);
        let tab_w = (usable / n).clamp(MIN_TAB_WIDTH * scale, MAX_TAB_WIDTH * scale);

        let mut tab_bar = div()
            .flex()
            .items_center()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .pl(s(8.0))
            .gap(s(4.0))
            .id("terminal-tab-bar")
            .overflow_x_scroll();

        for tab in &self.tabs {
            let tab_id = tab.id;
            let is_active = tab.is_active;
            let pending = self.tab_pending(tab_id);
            let (state_color, dot_size) = Self::tab_dot(tab, pending);
            let status_tooltip: Option<SharedString> = tab
                .status
                .as_ref()
                .map(|status| layout::status_tooltip(status).into());

            // Tab outer container — fixed (computed) width so tabs shrink as
            // more open, then the strip scrolls. Visual styling only.
            let group_name = SharedString::from(format!("tab-group-{}", tab_id));
            let mut tab_el = div()
                .group(group_name.clone())
                .flex()
                .items_center()
                .flex_shrink_0()
                .w(px(tab_w))
                .h(s(TAB_BAR_HEIGHT - 10.0))
                .rounded(s(8.0))
                .border_1();

            // A tab is a pill; the active one is a small card like the
            // panes below it. The border is always there so a tab does not
            // move when it becomes active.
            if is_active {
                tab_el = tab_el
                    .bg(ShellDeckColors::terminal_bg())
                    .border_color(ShellDeckColors::border())
                    .shadow_sm()
                    .text_color(ShellDeckColors::text_primary());
            } else {
                tab_el = tab_el
                    .border_color(gpui::transparent_black())
                    .text_color(ShellDeckColors::text_muted())
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()));
            }

            // Coarse char cap; the flex layout + overflow clip handles the rest.
            let max_chars = 28;
            let (name, folder) = layout::title_parts(tab.display_title(), None, None);
            let name = if name.chars().count() > max_chars {
                let truncated: String = name.chars().take(max_chars).collect();
                format!("{}\u{2026}", truncated) // ellipsis
            } else {
                name
            };

            // Clickable content area (dot + title) — selects the tab. Grows to
            // fill the tab and clips its title when the tab is narrow.
            let mut tab_content = div()
                .id(ElementId::from(SharedString::from(format!(
                    "tab-{}",
                    tab_id
                ))))
                .flex()
                .flex_1()
                .min_w(px(0.0))
                .items_center()
                .gap(s(6.0))
                .px(s(10.0))
                .py(s(5.0))
                .overflow_hidden()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                    this.select_tab(tab_id);
                    cx.emit(TerminalEvent::TabSelected(tab_id));
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                        this.context_menu = None;
                        this.tab_context_menu = Some(TabMenuState {
                            position: event.position,
                            tab_id,
                        });
                        cx.notify();
                    }),
                )
                // Status dot
                .child(
                    div()
                        .w(s(dot_size))
                        .h(s(dot_size))
                        .rounded_full()
                        .bg(state_color)
                        .flex_shrink_0(),
                );
            if let Some(label) = status_tooltip {
                tab_content = tab_content.tooltip(move |_, cx| {
                    cx.new(|_| TerminalToolbarTooltip {
                        label: label.clone(),
                    })
                    .into()
                });
            }

            // Split indicator — small icon next to the tab's status dot
            let tab_has_split = if is_active {
                self.layout.is_split()
            } else {
                self.stored_layouts
                    .get(&tab_id)
                    .map(|l| l.is_split())
                    .unwrap_or(false)
            };
            if tab_has_split {
                tab_content = tab_content.child(
                    div()
                        .text_size(s(10.0))
                        .text_color(ShellDeckColors::primary())
                        .flex_shrink_0()
                        .child("\u{2ABF}"), // ⫿ vertical line with horizontal stroke
                );
            }

            // Title — the name, then its folder in a quieter colour. The
            // folder gives way first when the tab is narrow.
            let mut title = div()
                .flex()
                .flex_1()
                .min_w(px(0.0))
                .items_center()
                .gap(s(6.0))
                .overflow_hidden()
                .text_size(s(12.0))
                .whitespace_nowrap()
                .child(
                    div()
                        .flex_shrink_0()
                        .max_w_full()
                        .overflow_hidden()
                        .font_weight(FontWeight::MEDIUM)
                        .child(name),
                );
            if let Some(folder) = folder {
                title = title.child(
                    div()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .text_color(ShellDeckColors::text_muted())
                        .child(folder),
                );
            }
            let tab_content = tab_content.child(title);

            tab_el = tab_el.child(tab_content);

            // Close button — fixed, never shrinks away.
            let close_btn = div()
                .id(ElementId::from(SharedString::from(format!(
                    "close-tab-{}",
                    tab_id
                ))))
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .w(s(16.0))
                .h(s(16.0))
                .mr(s(4.0))
                .rounded(s(4.0))
                .text_size(s(10.0))
                .text_color(ShellDeckColors::text_muted())
                .cursor_pointer()
                .hover(|el| {
                    el.bg(ShellDeckColors::badge_bg())
                        .text_color(ShellDeckColors::error())
                })
                .child(
                    svg()
                        .path("icons/lucide/x.svg")
                        .size(s(10.0))
                        .text_color(ShellDeckColors::text_muted()),
                )
                .on_click(cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                    this.close_tab(tab_id);
                    cx.emit(TerminalEvent::TabClosed(tab_id));
                    cx.notify();
                }));

            if is_active {
                // Always show close on active tab
                tab_el = tab_el.child(close_btn);
            } else {
                // Show close only on group hover for inactive tabs
                tab_el = tab_el.child(
                    div()
                        .opacity(0.0)
                        .group_hover(group_name, |style| style.opacity(1.0))
                        .child(close_btn),
                );
            }

            tab_bar = tab_bar.child(tab_el);
        }

        // New tab button — fixed, stays visible past the scrolling strip.
        tab_bar = tab_bar.child(
            div()
                .id("new-tab-btn")
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .w(s(28.0))
                .h(s(28.0))
                .ml(s(4.0))
                .cursor_pointer()
                .rounded(s(6.0))
                .text_size(s(14.0))
                .text_color(ShellDeckColors::text_muted())
                .hover(|el| {
                    el.bg(ShellDeckColors::hover_bg())
                        .text_color(ShellDeckColors::text_primary())
                })
                .child(
                    svg()
                        .path("icons/lucide/plus.svg")
                        .size(s(14.0))
                        .text_color(ShellDeckColors::text_muted()),
                )
                .on_click(cx.listener(|this, _event: &ClickEvent, window, cx| {
                    this.spawn_local_terminal(cx);
                    this.focus_handle.focus(window);
                    cx.emit(TerminalEvent::NewTabRequested);
                    cx.notify();
                })),
        );

        // New tab running an agent, beside the new-tab button.
        let launcher = |id: &'static str, label: String, accent: Hsla, logo: AnyElement| {
            let label: SharedString = label.into();
            div()
                .id(id)
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .w(s(28.0))
                .h(s(28.0))
                .rounded(s(6.0))
                .cursor_pointer()
                .hover(move |el| el.bg(accent.opacity(0.16)))
                .tooltip(move |_, cx| {
                    cx.new(|_| TerminalToolbarTooltip {
                        label: label.clone(),
                    })
                    .into()
                })
                .child(logo)
        };
        if self.claude_available {
            tab_bar = tab_bar.child(
                launcher(
                    "tab-launch-claude",
                    t!("terminal.toolbar.claude").to_string(),
                    Self::claude_orange(),
                    Self::claude_logo(16.0 * scale).into_any_element(),
                )
                .on_click(cx.listener(|this, _, _, cx| this.launch_claude(cx))),
            );
        }
        if self.codex_available {
            tab_bar = tab_bar.child(
                launcher(
                    "tab-launch-codex",
                    t!("terminal.toolbar.codex").to_string(),
                    Self::codex_green(),
                    Self::codex_logo(16.0 * scale).into_any_element(),
                )
                .on_click(cx.listener(|this, _, _, cx| this.launch_codex(cx))),
            );
        }

        // The strip scrolls; the actions button stays at the right edge.
        div()
            .flex()
            .items_center()
            .flex_shrink_0()
            .w_full()
            .h(s(TAB_BAR_HEIGHT))
            .bg(Self::canvas_tint())
            .gap(s(6.0))
            .pr(s(8.0))
            .child(tab_bar)
            .child(self.render_actions_button(window, cx))
    }

    /// The button that shows or hides the terminal actions. It sits at the
    /// end of the tab strip, or in the header of the session list.
    fn render_actions_button(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = Self::ui_scale(window);
        let s = |v: f32| px(v * scale);
        let label: SharedString = t!("terminal.toolbar.more").to_string().into();
        let open = self.actions_open;
        div()
            .id("terminal-actions-btn")
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .w(s(28.0))
            .h(s(28.0))
            .rounded(s(6.0))
            .cursor_pointer()
            .when(open, |el| el.bg(ShellDeckColors::selected_bg()))
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            .tooltip(move |_, cx| {
                cx.new(|_| TerminalToolbarTooltip {
                    label: label.clone(),
                })
                .into()
            })
            .child(
                svg()
                    .path("icons/lucide/ellipsis.svg")
                    .size(s(14.0))
                    .text_color(if open {
                        ShellDeckColors::text_primary()
                    } else {
                        ShellDeckColors::text_muted()
                    }),
            )
            .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                this.actions_open = !this.actions_open;
                cx.notify();
            }))
    }

    /// The terminal actions (agents, search, clipboard, splits, zoom,
    /// scripts, clear): a floating card, shown from `render_actions_button`.
    fn render_toolbar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Scale the whole toolbar with the UI size: shadow `px` with a
        // scale-multiplying version so every dimension below tracks the setting.
        // The toolbar has no mouse-coordinate math, so this is safe.
        let scale = Self::ui_scale(window);
        let px = |v: f32| gpui::px(v * scale);
        let available_width = window.viewport_size().width.to_f64() as f32 - self.content_left();
        // Always icons with tooltips: the actions are a small floating card.
        let compact = true;
        let show_pinned_scripts = available_width >= 1400.0 * scale;

        let zoom = self
            .tabs
            .get(self.pane.active_index)
            .map(|t| t.zoom_level)
            .unwrap_or(1.0);
        let zoom_pct = format!("{}%", (zoom * 100.0).round() as u32);

        let has_selection = self
            .active_session()
            .is_some_and(|s| s.grid.lock().selected_text().is_some());

        let lbl_search = t!("terminal.toolbar.search");
        let lbl_copy = t!("terminal.toolbar.copy");
        let lbl_paste = t!("terminal.toolbar.paste");
        let lbl_claude = t!("terminal.toolbar.claude");
        let lbl_codex = t!("terminal.toolbar.codex");
        let lbl_rotate = t!("terminal.toolbar.rotate");
        let lbl_close_split = t!("terminal.toolbar.close_split");
        let lbl_split_h = t!("terminal.toolbar.split_h");
        let lbl_split_v = t!("terminal.toolbar.split_v");
        let lbl_scripts = t!("terminal.toolbar.scripts");
        let lbl_clear = t!("terminal.toolbar.clear");
        let lbl_favorites = t!("terminal.toolbar.favorites");
        let lbl_recent = t!("terminal.toolbar.recent");
        let lbl_ai_command = t!("terminal.toolbar.ai_command");
        let lbl_ai_diagnose = t!("terminal.toolbar.ai_diagnose");
        let lbl_ai_name = t!("terminal.toolbar.ai_name");
        let lbl_ai_issue = t!("terminal.toolbar.ai_issue");
        let lbl_ai_stop = t!("terminal.toolbar.ai_stop");

        // The keys shown are the ones bound, which the user may have changed.
        let shortcut_tooltip = |label: &str, action: BindableAction| match shortcut_label(action) {
            Some(keys) => format!("{label} ({keys})"),
            None => label.to_string(),
        };

        let toolbar_btn = |id: &str, label: &str, tooltip_label: String| {
            let tooltip_label: SharedString = tooltip_label.into();
            div()
                .id(ElementId::from(SharedString::from(id.to_string())))
                .flex()
                .items_center()
                .px(px(8.0))
                .py(px(3.0))
                .flex_shrink_0()
                .rounded(px(4.0))
                .cursor_pointer()
                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ShellDeckColors::text_primary())
                        .child(label.to_string()),
                )
                .tooltip(move |_, cx| {
                    cx.new(|_| TerminalToolbarTooltip {
                        label: tooltip_label.clone(),
                    })
                    .into()
                })
        };

        let toolbar_icon = |id: &str, icon: &str, tooltip_label: String| {
            let tooltip_label: SharedString = tooltip_label.into();
            div()
                .id(ElementId::from(SharedString::from(id.to_string())))
                .flex()
                .items_center()
                .justify_center()
                .w(px(28.0))
                .h(px(24.0))
                .rounded(px(4.0))
                .text_color(ShellDeckColors::text_muted())
                .cursor_pointer()
                .hover(|el| {
                    el.bg(ShellDeckColors::hover_bg())
                        .text_color(ShellDeckColors::text_primary())
                })
                .child(
                    svg()
                        .path(format!("icons/lucide/{icon}.svg"))
                        .text_color(ShellDeckColors::text_muted())
                        .size(px(14.0)),
                )
                .tooltip(move |_, cx| {
                    cx.new(|_| TerminalToolbarTooltip {
                        label: tooltip_label.clone(),
                    })
                    .into()
                })
        };

        let ai_toolbar_icon = |id: &str, icon: &str, tooltip_label: String| {
            let tooltip_label: SharedString = tooltip_label.into();
            div()
                .id(ElementId::from(SharedString::from(id.to_string())))
                .flex()
                .items_center()
                .justify_center()
                .w(px(28.0))
                .h(px(24.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(ShellDeckColors::primary().opacity(0.35))
                .bg(ShellDeckColors::primary().opacity(0.10))
                .cursor_pointer()
                .hover(|el| el.bg(ShellDeckColors::primary().opacity(0.18)))
                .child(
                    svg()
                        .path(format!("icons/lucide/{icon}.svg"))
                        .text_color(ShellDeckColors::primary())
                        .size(px(14.0)),
                )
                .tooltip(move |_, cx| {
                    cx.new(|_| TerminalToolbarTooltip {
                        label: tooltip_label.clone(),
                    })
                    .into()
                })
        };

        // Branded CLI launchers need a compact logo + label treatment that the
        // standard one-line Button component does not support.
        let cli_button = |id: &'static str, label: String, accent: Hsla, logo: AnyElement| {
            let tooltip_label: SharedString = label.clone().into();
            let mut button = div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .px(if compact { px(5.0) } else { px(7.0) })
                .py(px(3.0))
                .flex_shrink_0()
                .rounded(px(5.0))
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(accent)
                .bg(accent.opacity(0.12))
                .cursor_pointer()
                .hover(move |el| el.bg(accent.opacity(0.2)))
                .child(logo);
            if compact {
                button = button.tooltip(move |_, cx| {
                    cx.new(|_| TerminalToolbarTooltip {
                        label: tooltip_label.clone(),
                    })
                    .into()
                });
            } else {
                button = button.child(label);
            }
            button
        };

        let mut toolbar = div()
            .flex()
            .items_center()
            .h(px(36.0))
            .px(px(8.0))
            .gap(px(6.0));

        if self.claude_available {
            let button = cli_button(
                "tb-claude",
                lbl_claude.to_string(),
                Self::claude_orange(),
                Self::claude_logo(18.0).into_any_element(),
            )
            .on_click(cx.listener(|this, _, _, cx| this.launch_claude(cx)));
            toolbar = toolbar.child(button);
        }
        if self.codex_available {
            let button = cli_button(
                "tb-codex",
                lbl_codex.to_string(),
                Self::codex_green(),
                Self::codex_logo(18.0).into_any_element(),
            )
            .on_click(cx.listener(|this, _, _, cx| this.launch_codex(cx)));
            toolbar = toolbar.child(button);
        }
        if self.claude_available || self.codex_available {
            toolbar = toolbar.child(
                div()
                    .w(px(1.0))
                    .h(px(16.0))
                    .mx(px(6.0))
                    .bg(ShellDeckColors::border()),
            );
        }

        if self.ai_actions_enabled {
            if let Some(session) = self.active_session() {
                let session_id = session.id;
                if self.ai_running_command == Some(session_id) {
                    toolbar = toolbar.child(
                        ai_toolbar_icon("tb-ai-stop", "square", lbl_ai_stop.to_string())
                            .border_color(ShellDeckColors::error().opacity(0.45))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(TerminalEvent::StopAiCommand(session_id));
                            })),
                    );
                }
                toolbar = toolbar.child(
                    ai_toolbar_icon("tb-ai-command", "sparkles", lbl_ai_command.to_string())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(TerminalEvent::GenerateCommandWithAi(session_id));
                        })),
                );

                let has_diagnostic_context = {
                    let grid = session.grid.lock();
                    grid.selected_text().is_some()
                        || grid
                            .visible_rows()
                            .iter()
                            .any(|row| row.iter().any(|cell| !cell.c.is_whitespace()))
                };
                let mut diagnose = ai_toolbar_icon(
                    "tb-ai-diagnose",
                    "circle-alert",
                    lbl_ai_diagnose.to_string(),
                );
                if has_diagnostic_context {
                    diagnose = diagnose.on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(TerminalEvent::DiagnoseWithAi(session_id));
                    }));
                } else {
                    diagnose = diagnose.opacity(0.42).cursor_default();
                }
                let mut create_issue =
                    ai_toolbar_icon("tb-ai-issue", "file-plus-2", lbl_ai_issue.to_string());
                if has_diagnostic_context {
                    create_issue = create_issue.on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(TerminalEvent::CreateIssueFromContext(session_id));
                    }));
                } else {
                    create_issue = create_issue.opacity(0.42).cursor_default();
                }
                toolbar = toolbar.child(diagnose).child(create_issue).child(
                    div()
                        .w(px(1.0))
                        .h(px(16.0))
                        .mx(px(6.0))
                        .bg(ShellDeckColors::border()),
                );
            }
        }

        if self.ai_naming_enabled {
            if let Some(session) = self.active_session() {
                let session_id = session.id;
                toolbar = toolbar.child(
                    ai_toolbar_icon("tb-ai-name", "wand-sparkles", lbl_ai_name.to_string())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(TerminalEvent::SuggestNameWithAi(session_id));
                        })),
                );
            }
        }

        // Left group: icon-only at compact widths, labels otherwise.
        if compact {
            toolbar = toolbar.child(
                toolbar_icon(
                    "tb-search",
                    "search",
                    shortcut_tooltip(lbl_search.as_ref(), BindableAction::Search),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.toggle_search();
                    cx.notify();
                })),
            );

            let mut copy = toolbar_icon(
                "tb-copy",
                "copy",
                shortcut_tooltip(lbl_copy.as_ref(), BindableAction::Copy),
            );
            if !has_selection {
                copy = copy.opacity(0.45).cursor_default();
            } else {
                copy = copy.on_click(cx.listener(|this, _, _, cx| {
                    this.copy_selection(cx);
                    cx.notify();
                }));
            }
            toolbar = toolbar.child(copy);
            toolbar = toolbar.child(
                toolbar_icon(
                    "tb-paste",
                    "clipboard-paste",
                    shortcut_tooltip(lbl_paste.as_ref(), BindableAction::Paste),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.paste_clipboard(cx);
                    cx.notify();
                })),
            );
        } else {
            toolbar = toolbar
                .child(
                    toolbar_btn(
                        "tb-search",
                        lbl_search.as_ref(),
                        shortcut_tooltip(lbl_search.as_ref(), BindableAction::Search),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_search();
                        cx.notify();
                    })),
                )
                .child({
                    let mut btn = toolbar_btn(
                        "tb-copy",
                        lbl_copy.as_ref(),
                        shortcut_tooltip(lbl_copy.as_ref(), BindableAction::Copy),
                    );
                    if !has_selection {
                        btn = btn.opacity(0.45).cursor_default();
                    } else {
                        btn = btn.on_click(cx.listener(|this, _, _, cx| {
                            this.copy_selection(cx);
                            cx.notify();
                        }));
                    }
                    btn
                })
                .child(
                    toolbar_btn(
                        "tb-paste",
                        lbl_paste.as_ref(),
                        shortcut_tooltip(lbl_paste.as_ref(), BindableAction::Paste),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.paste_clipboard(cx);
                        cx.notify();
                    })),
                );
        }

        // Separator
        toolbar = toolbar.child(
            div()
                .w(px(1.0))
                .h(px(16.0))
                .mx(px(6.0))
                .bg(ShellDeckColors::border()),
        );

        // Managed server tabs keep their complete layout inside tmux.
        if self
            .active_session()
            .is_some_and(|session| session.is_tmux_managed())
        {
            for (id, label, hint, action) in [
                (
                    "tb-tmux-split-h",
                    lbl_split_h.to_string(),
                    t!("terminal.tmux.split_h").to_string(),
                    TmuxAction::SplitHorizontal,
                ),
                (
                    "tb-tmux-split-v",
                    lbl_split_v.to_string(),
                    t!("terminal.tmux.split_v").to_string(),
                    TmuxAction::SplitVertical,
                ),
            ] {
                toolbar = toolbar.child(toolbar_btn(id, &label, hint).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.tmux_action(action, cx);
                        this.focus_handle.focus(window);
                    },
                )));
            }
            for (id, icon, hint, action) in [
                (
                    "tb-tmux-next",
                    "arrow-right",
                    t!("terminal.tmux.next").to_string(),
                    TmuxAction::NextPane,
                ),
                (
                    "tb-tmux-layout",
                    "refresh-cw",
                    t!("terminal.tmux.layout").to_string(),
                    TmuxAction::NextLayout,
                ),
                (
                    "tb-tmux-zoom",
                    "maximize-2",
                    t!("terminal.tmux.zoom").to_string(),
                    TmuxAction::ToggleZoom,
                ),
            ] {
                toolbar = toolbar.child(toolbar_icon(id, icon, hint).on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.tmux_action(action, cx);
                        this.focus_handle.focus(window);
                    },
                )));
            }
        } else if self.layout.is_split() {
            toolbar = toolbar
                .child(
                    toolbar_btn(
                        "tb-rotate-split",
                        lbl_rotate.as_ref(),
                        lbl_rotate.to_string(),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_split_direction();
                        this.resize_if_needed(window);
                        cx.notify();
                    })),
                )
                .child(
                    toolbar_btn(
                        "tb-close-split",
                        lbl_close_split.as_ref(),
                        lbl_close_split.to_string(),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close_split();
                        cx.notify();
                    })),
                );
        } else if !self.layout.is_split() {
            toolbar = toolbar.child(
                toolbar_btn(
                    "tb-split-h",
                    lbl_split_h.as_ref(),
                    shortcut_tooltip(lbl_split_h.as_ref(), BindableAction::SplitHorizontal),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.split_horizontal(cx);
                })),
            );
            toolbar = toolbar.child(
                toolbar_btn(
                    "tb-split-v",
                    lbl_split_v.as_ref(),
                    shortcut_tooltip(lbl_split_v.as_ref(), BindableAction::SplitVertical),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.split_vertical(cx);
                })),
            );
        }

        // Separator
        toolbar = toolbar.child(
            div()
                .w(px(1.0))
                .h(px(16.0))
                .mx(px(6.0))
                .bg(ShellDeckColors::border()),
        );
        if compact {
            toolbar = toolbar.child(div().flex_grow());
        }

        // Right group: zoom controls
        toolbar = toolbar
            .child(
                toolbar_icon(
                    "tb-zoom-out",
                    "minus",
                    shortcut_tooltip(t!("shortcuts.zoom").as_ref(), BindableAction::ZoomOut),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.zoom_out();
                    this.resize_if_needed(window);
                    cx.notify();
                })),
            )
            .child(
                div()
                    .id("tb-zoom-level")
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(42.0))
                    .h(px(24.0))
                    .rounded(px(4.0))
                    .text_size(px(11.0))
                    .text_color(if (zoom - 1.0).abs() < 0.01 {
                        ShellDeckColors::text_muted()
                    } else {
                        ShellDeckColors::primary()
                    })
                    .cursor_pointer()
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                    .child(zoom_pct)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.zoom_reset();
                        this.resize_if_needed(window);
                        cx.notify();
                    })),
            )
            .child(
                toolbar_icon(
                    "tb-zoom-in",
                    "plus",
                    shortcut_tooltip(t!("shortcuts.zoom").as_ref(), BindableAction::ZoomIn),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.zoom_in();
                    this.resize_if_needed(window);
                    cx.notify();
                })),
            );

        // Separator before scripts
        let has_scripts = !self.favorite_scripts.is_empty() || !self.recent_scripts.is_empty();
        if has_scripts {
            toolbar = toolbar.child(
                div()
                    .w(px(1.0))
                    .h(px(16.0))
                    .mx(px(6.0))
                    .bg(ShellDeckColors::border()),
            );

            // Scripts dropdown button + panel
            let is_open = self.script_dropdown_open;
            let scripts_button = if compact {
                toolbar_icon("tb-scripts", "scroll-text", lbl_scripts.to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.script_dropdown_open = !this.script_dropdown_open;
                        cx.notify();
                    }))
                    .into_any_element()
            } else {
                toolbar_btn("tb-scripts", lbl_scripts.as_ref(), lbl_scripts.to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.script_dropdown_open = !this.script_dropdown_open;
                        cx.notify();
                    }))
                    .into_any_element()
            };
            let mut scripts_wrapper = div().relative().child(scripts_button);

            if is_open {
                let mut dropdown = div()
                    .id("scripts-dropdown")
                    .absolute()
                    .top(px(30.0))
                    .left_0()
                    .w(px(220.0))
                    .bg(ShellDeckColors::bg_surface())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .rounded(px(8.0))
                    .shadow_lg()
                    .py(px(4.0))
                    .flex()
                    .flex_col()
                    .overflow_y_scroll();

                // Favorites section
                if !self.favorite_scripts.is_empty() {
                    dropdown = dropdown.child(
                        div()
                            .px(px(10.0))
                            .py(px(4.0))
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(lbl_favorites.to_string()),
                    );
                    for (id, name, lang_badge) in &self.favorite_scripts {
                        let script_id = *id;
                        let name = name.clone();
                        let badge = lang_badge.clone();
                        let is_pinned = self.pinned_scripts.iter().any(|p| p.id == script_id);
                        dropdown = dropdown.child(
                            div()
                                .id(ElementId::from(SharedString::from(format!(
                                    "fav-{}",
                                    script_id
                                ))))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .px(px(10.0))
                                .py(px(5.0))
                                .cursor_pointer()
                                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.script_dropdown_open = false;
                                    cx.emit(TerminalEvent::RunScriptRequested(script_id));
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .text_size(px(9.0))
                                        .px(px(3.0))
                                        .py(px(1.0))
                                        .rounded(px(3.0))
                                        .bg(ShellDeckColors::primary().opacity(0.15))
                                        .text_color(ShellDeckColors::primary())
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(badge),
                                )
                                .child(
                                    div()
                                        .flex_grow()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(name),
                                )
                                // Pin/unpin toggle
                                .child(
                                    div()
                                        .id(ElementId::from(SharedString::from(format!(
                                            "pin-fav-{}",
                                            script_id
                                        ))))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .w(px(16.0))
                                        .h(px(16.0))
                                        .cursor_pointer()
                                        .text_color(if is_pinned {
                                            ShellDeckColors::primary()
                                        } else {
                                            ShellDeckColors::text_muted()
                                        })
                                        .hover(|el| el.text_color(ShellDeckColors::primary()))
                                        .on_click(cx.listener(
                                            move |_this, _: &ClickEvent, _, cx| {
                                                cx.emit(TerminalEvent::TogglePinScript(script_id));
                                            },
                                        ))
                                        .child(
                                            svg()
                                                .path(if is_pinned {
                                                    "icons/lucide/pin.svg"
                                                } else {
                                                    "images/pin-outline.svg"
                                                })
                                                .size(px(11.0)),
                                        ),
                                ),
                        );
                    }
                }

                // Separator between sections
                if !self.favorite_scripts.is_empty() && !self.recent_scripts.is_empty() {
                    dropdown = dropdown.child(
                        div()
                            .h(px(1.0))
                            .mx(px(8.0))
                            .my(px(4.0))
                            .bg(ShellDeckColors::border()),
                    );
                }

                // Recent section
                if !self.recent_scripts.is_empty() {
                    dropdown = dropdown.child(
                        div()
                            .px(px(10.0))
                            .py(px(4.0))
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(lbl_recent.to_string()),
                    );
                    for (id, name, lang_badge) in &self.recent_scripts {
                        let script_id = *id;
                        let name = name.clone();
                        let badge = lang_badge.clone();
                        dropdown = dropdown.child(
                            div()
                                .id(ElementId::from(SharedString::from(format!(
                                    "rec-{}",
                                    script_id
                                ))))
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .px(px(10.0))
                                .py(px(5.0))
                                .cursor_pointer()
                                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.script_dropdown_open = false;
                                    cx.emit(TerminalEvent::RunScriptRequested(script_id));
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .text_size(px(9.0))
                                        .px(px(3.0))
                                        .py(px(1.0))
                                        .rounded(px(3.0))
                                        .bg(ShellDeckColors::primary().opacity(0.15))
                                        .text_color(ShellDeckColors::primary())
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(badge),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(name),
                                ),
                        );
                    }
                }

                scripts_wrapper = scripts_wrapper.child(dropdown);
            }

            toolbar = toolbar.child(scripts_wrapper);
        }

        // Pinned script buttons
        if show_pinned_scripts && !self.pinned_scripts.is_empty() {
            // Separator before pinned scripts
            toolbar = toolbar.child(
                div()
                    .w(px(1.0))
                    .h(px(16.0))
                    .mx(px(4.0))
                    .bg(ShellDeckColors::border()),
            );

            let max_visible = 6;
            let visible_count = self.pinned_scripts.len().min(max_visible);
            for pinned in self.pinned_scripts.iter().take(max_visible) {
                let script_id = pinned.id;
                let badge_text = pinned.badge.clone();
                let script_name = pinned.name.clone();
                let (r, g, b) = pinned.badge_color;
                let badge_color =
                    rgba(((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF);
                let badge_bg_color =
                    rgba(((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0x33);

                toolbar = toolbar.child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "pinned-{}",
                            script_id
                        ))))
                        .flex()
                        .items_center()
                        .gap(px(3.0))
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .cursor_pointer()
                        .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(move |_this, _, _, cx| {
                            cx.emit(TerminalEvent::RunScriptRequested(script_id));
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |_this, _, _, cx| {
                                cx.emit(TerminalEvent::TogglePinScript(script_id));
                            }),
                        )
                        .child(
                            div()
                                .text_size(px(8.0))
                                .px(px(3.0))
                                .py(px(1.0))
                                .rounded(px(3.0))
                                .bg(badge_bg_color)
                                .text_color(badge_color)
                                .font_weight(FontWeight::BOLD)
                                .child(badge_text),
                        )
                        .child(
                            div()
                                .max_w(px(80.0))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(script_name),
                        ),
                );
            }

            // Overflow indicator
            if self.pinned_scripts.len() > max_visible {
                let overflow = self.pinned_scripts.len() - max_visible;
                toolbar = toolbar.child(
                    div()
                        .id("pinned-overflow")
                        .flex()
                        .items_center()
                        .justify_center()
                        .px(px(4.0))
                        .py(px(2.0))
                        .rounded(px(4.0))
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .cursor_pointer()
                        .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.script_dropdown_open = !this.script_dropdown_open;
                            cx.notify();
                        }))
                        .child(format!("+{}", overflow)),
                );
            }

            let _ = visible_count;
        }

        if compact {
            toolbar = toolbar.child(
                toolbar_icon(
                    "tb-clear",
                    "trash-2",
                    shortcut_tooltip(lbl_clear.as_ref(), BindableAction::ClearTerminal),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(session) = this.active_session() {
                        let mut grid = session.grid.lock();
                        grid.erase_display(2);
                        grid.cursor_to(0, 0);
                    }
                    cx.notify();
                })),
            );
        } else {
            toolbar = toolbar.child(div().flex_grow());
            toolbar = toolbar.child(
                toolbar_btn(
                    "tb-clear",
                    lbl_clear.as_ref(),
                    shortcut_tooltip(lbl_clear.as_ref(), BindableAction::ClearTerminal),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(session) = this.active_session() {
                        let mut grid = session.grid.lock();
                        grid.erase_display(2);
                        grid.cursor_to(0, 0);
                    }
                    cx.notify();
                })),
            );
        }

        toolbar
    }

    // -----------------------------------------------------------------------
    // Terminal grid – direct glyph painting via canvas
    // -----------------------------------------------------------------------

    fn render_terminal_grid(
        &self,
        mouse_mode: MouseMode,
        mouse_encoding: MouseEncoding,
        cursor: CursorState,
        cache: Arc<GlyphCache>,
        grid: Arc<Mutex<TerminalGrid>>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let cell_height_f = self.font_size * 1.4;

        // -- event handler closures --
        let handle = cx.entity().downgrade();
        let focus = self.focus_handle.clone();

        // Left mouse down: selection or terminal mouse mode
        let h_down = handle.clone();
        let mouse_down_handler = move |event: &MouseDownEvent,
                                       window: &mut Window,
                                       cx: &mut App| {
            focus.focus(window);
            let url = h_down.upgrade().and_then(|view| {
                view.update(cx, |this, _| {
                    this.url_click_active = false;
                    this.pixel_to_cell_zero(event.position)
                        .and_then(|(col, row)| {
                            this.detected_urls
                                .iter()
                                .find(|url| url.contains(col, row))
                                .map(|url| url.url.clone())
                        })
                })
            });
            let action = left_click_action(
                mouse_mode,
                event.modifiers.control || event.modifiers.secondary(),
                url.is_some(),
            );
            // Local link activation takes priority over tmux/TUI mouse reporting.
            if action == LeftClickAction::OpenUrl {
                if let Some(view) = h_down.upgrade() {
                    view.update(cx, |this, _| this.url_click_active = true);
                }
                if let Some(url) = url {
                    let _ = open::that_detached(url);
                }
                return;
            }
            if action == LeftClickAction::ReportMouse {
                if let Some(view) = h_down.upgrade() {
                    view.update(cx, |this, _cx| {
                        if let Some((col, row)) = this.pixel_to_cell(event.position) {
                            let bytes = Self::encode_mouse(mouse_encoding, 0, col, row, true);
                            if let Some(s) = this.active_session() {
                                s.write_input(&bytes);
                            }
                        }
                    });
                }
            } else {
                // Check for scrollbar click first
                if let Some(view) = h_down.upgrade() {
                    let is_scrollbar = view.read(cx).is_in_scrollbar_area(event.position, window);
                    if is_scrollbar {
                        view.update(cx, |this, cx| {
                            this.scrollbar_dragging = true;
                            this.scrollbar_scroll_to_y(event.position.y, window);
                            cx.notify();
                        });
                        return;
                    }
                }

                // Selection mode: handle single/double/triple click
                if let Some(view) = h_down.upgrade() {
                    view.update(cx, |this, cx| {
                        // Reset blink on click so cursor stays visible
                        this.reset_cursor_blink(cx);
                        // Dismiss context menu on any click
                        if this.context_menu.is_some() {
                            this.context_menu = None;
                            cx.notify();
                            return;
                        }
                        if let Some((col, row)) = this.pixel_to_cell_zero(event.position) {
                            let now = std::time::Instant::now();
                            let multi_click_threshold = std::time::Duration::from_millis(400);

                            // Detect multi-click
                            let is_multi = this
                                .last_click_time
                                .is_some_and(|t| now.duration_since(t) < multi_click_threshold)
                                && this.last_click_pos == Some((col, row));

                            if is_multi {
                                this.click_count = (this.click_count % 3) + 1;
                            } else {
                                this.click_count = 1;
                            }
                            this.last_click_time = Some(now);
                            this.last_click_pos = Some((col, row));

                            if let Some(session) = this.active_session() {
                                let mut grid = session.grid.lock();
                                let alt_held = event.modifiers.alt;
                                match this.click_count {
                                    1 => {
                                        grid.clear_selection();
                                        if alt_held {
                                            grid.start_block_selection(col, row);
                                        } else {
                                            grid.start_selection(col, row);
                                        }
                                    }
                                    2 => grid.start_word_selection(col, row),
                                    3 => grid.start_line_selection(col, row),
                                    _ => {}
                                }
                            }
                            this.selecting = true;
                            cx.notify();
                        }
                    });
                }
            }
        };

        // Left mouse up
        let h_up = handle.clone();
        let mouse_up_handler = move |event: &MouseUpEvent, _window: &mut Window, cx: &mut App| {
            if let Some(view) = h_up.upgrade() {
                if view.update(cx, |this, _| std::mem::take(&mut this.url_click_active)) {
                    return;
                }
            }
            if mouse_mode != MouseMode::None {
                if let Some(view) = h_up.upgrade() {
                    view.update(cx, |this, _cx| {
                        if let Some((col, row)) = this.pixel_to_cell(event.position) {
                            let bytes = Self::encode_mouse(mouse_encoding, 0, col, row, false);
                            if let Some(s) = this.active_session() {
                                s.write_input(&bytes);
                            }
                        }
                    });
                }
            } else {
                // End scrollbar drag or selection drag
                if let Some(view) = h_up.upgrade() {
                    view.update(cx, |this, _cx| {
                        this.scrollbar_dragging = false;
                        this.selecting = false;
                        if let Some(session) = this.active_session() {
                            session.grid.lock().end_selection();
                        }
                    });
                }
            }
        };

        // Mouse move: selection drag or terminal mouse reporting
        let h_move = handle.clone();
        let mouse_move_handler =
            move |event: &MouseMoveEvent, window: &mut Window, cx: &mut App| {
                if h_move
                    .upgrade()
                    .is_some_and(|view| view.read(cx).url_click_active)
                {
                    return;
                }
                if mouse_mode != MouseMode::None {
                    let should_report = match mouse_mode {
                        MouseMode::AnyMotion => true,
                        MouseMode::ButtonTracking => event.pressed_button.is_some(),
                        _ => false,
                    };
                    if should_report {
                        if let Some(view) = h_move.upgrade() {
                            view.update(cx, |this, _cx| {
                                if let Some((col, row)) = this.pixel_to_cell(event.position) {
                                    let button = match event.pressed_button {
                                        Some(MouseButton::Left) => 32u8,
                                        Some(MouseButton::Middle) => 33u8,
                                        Some(MouseButton::Right) => 34u8,
                                        _ => 35u8,
                                    };
                                    let bytes =
                                        Self::encode_mouse(mouse_encoding, button, col, row, true);
                                    if let Some(s) = this.active_session() {
                                        s.write_input(&bytes);
                                    }
                                }
                            });
                        }
                    }
                } else {
                    // Scrollbar drag or selection drag
                    if let Some(view) = h_move.upgrade() {
                        view.update(cx, |this, cx| {
                            if this.scrollbar_dragging {
                                this.scrollbar_scroll_to_y(event.position.y, window);
                                cx.notify();
                            } else if this.selecting {
                                if let Some((col, row)) = this.pixel_to_cell_zero(event.position) {
                                    if let Some(session) = this.active_session() {
                                        session.grid.lock().update_selection(col, row);
                                    }
                                    cx.notify();
                                }
                            }
                        });
                    }
                }
            };

        // Scroll wheel: scrollback or terminal mouse reporting
        let h_scroll = handle.clone();
        let scroll_handler = move |event: &ScrollWheelEvent, _window: &mut Window, cx: &mut App| {
            if mouse_mode != MouseMode::None {
                if let Some(view) = h_scroll.upgrade() {
                    view.update(cx, |this, _cx| {
                        if let Some((col, row)) = this.pixel_to_cell(event.position) {
                            let delta_y = match event.delta {
                                ScrollDelta::Lines(pt) => pt.y,
                                ScrollDelta::Pixels(pt) => pt.y / px(cell_height_f),
                            };
                            // Wheel up (delta_y > 0) → button 64, wheel down → 65.
                            let button = if delta_y > 0.0 { 64u8 } else { 65u8 };
                            let bytes = Self::encode_mouse(mouse_encoding, button, col, row, true);
                            if let Some(s) = this.active_session() {
                                s.write_input(&bytes);
                            }
                        }
                    });
                }
            } else {
                // Scrollback navigation
                if let Some(view) = h_scroll.upgrade() {
                    view.update(cx, |this, cx| {
                        let delta_y = match event.delta {
                            ScrollDelta::Lines(pt) => pt.y,
                            ScrollDelta::Pixels(pt) => pt.y / px(cell_height_f),
                        };
                        let lines = delta_y.abs().ceil() as usize;
                        if let Some(session) = this.active_session() {
                            let mut grid = session.grid.lock();
                            // Wheel up (delta_y > 0) scrolls back into history.
                            if delta_y > 0.0 {
                                grid.scroll_view_up(lines);
                            } else {
                                grid.scroll_view_down(lines);
                            }
                        }
                        cx.notify();
                    });
                }
            }
        };

        let focus2 = self.focus_handle.clone();
        let focus3 = self.focus_handle.clone();
        let h_down2 = cx.entity().downgrade();
        let h_down3 = cx.entity().downgrade();
        let h_up2 = cx.entity().downgrade();
        let h_up3 = cx.entity().downgrade();

        // -- build the grid element --
        let grid_el = div()
            .id("terminal-grid")
            .track_focus(&self.focus_handle)
            .on_mouse_down(MouseButton::Left, mouse_down_handler)
            .on_mouse_down(
                MouseButton::Right,
                move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                    focus2.focus(window);
                    if mouse_mode != MouseMode::None {
                        if let Some(view) = h_down2.upgrade() {
                            view.update(cx, |this, _cx| {
                                if let Some((col, row)) = this.pixel_to_cell(event.position) {
                                    let bytes =
                                        Self::encode_mouse(mouse_encoding, 2, col, row, true);
                                    if let Some(s) = this.active_session() {
                                        s.write_input(&bytes);
                                    }
                                }
                            });
                        }
                    } else {
                        // Show context menu
                        if let Some(view) = h_down2.upgrade() {
                            view.update(cx, |this, cx| {
                                // Check if right-clicking on a URL
                                let url = this.pixel_to_cell_zero(event.position).and_then(
                                    |(col, row)| {
                                        this.detected_urls
                                            .iter()
                                            .find(|u| u.contains(col, row))
                                            .map(|u| u.url.clone())
                                    },
                                );
                                this.context_menu = Some(ContextMenuState {
                                    position: event.position,
                                    url,
                                });
                                cx.notify();
                            });
                        }
                    }
                },
            )
            .on_mouse_down(
                MouseButton::Middle,
                move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                    focus3.focus(window);
                    if mouse_mode != MouseMode::None {
                        if let Some(view) = h_down3.upgrade() {
                            view.update(cx, |this, _cx| {
                                if let Some((col, row)) = this.pixel_to_cell(event.position) {
                                    let bytes =
                                        Self::encode_mouse(mouse_encoding, 1, col, row, true);
                                    if let Some(s) = this.active_session() {
                                        s.write_input(&bytes);
                                    }
                                }
                            });
                        }
                    }
                },
            )
            .on_mouse_up(MouseButton::Left, mouse_up_handler)
            .on_mouse_up(
                MouseButton::Right,
                move |event: &MouseUpEvent, _window: &mut Window, cx: &mut App| {
                    if mouse_mode != MouseMode::None {
                        if let Some(view) = h_up2.upgrade() {
                            view.update(cx, |this, _cx| {
                                if let Some((col, row)) = this.pixel_to_cell(event.position) {
                                    let bytes =
                                        Self::encode_mouse(mouse_encoding, 2, col, row, false);
                                    if let Some(s) = this.active_session() {
                                        s.write_input(&bytes);
                                    }
                                }
                            });
                        }
                    }
                },
            )
            .on_mouse_up(
                MouseButton::Middle,
                move |event: &MouseUpEvent, _window: &mut Window, cx: &mut App| {
                    if mouse_mode != MouseMode::None {
                        if let Some(view) = h_up3.upgrade() {
                            view.update(cx, |this, _cx| {
                                if let Some((col, row)) = this.pixel_to_cell(event.position) {
                                    let bytes =
                                        Self::encode_mouse(mouse_encoding, 1, col, row, false);
                                    if let Some(s) = this.active_session() {
                                        s.write_input(&bytes);
                                    }
                                }
                            });
                        }
                    }
                },
            )
            .on_mouse_move(mouse_move_handler)
            .on_scroll_wheel(scroll_handler)
            .on_key_down(move |event: &KeyDownEvent, _window, cx| {
                if let Some(view) = handle.upgrade() {
                    view.update(cx, |this, cx| {
                        // When search is visible, intercept keystrokes for the search bar
                        if this.search_visible {
                            let key = event.keystroke.key.as_str();
                            match key {
                                "escape" => {
                                    this.toggle_search();
                                    cx.notify();
                                }
                                "enter" => {
                                    if event.keystroke.modifiers.shift {
                                        this.search_prev();
                                    } else {
                                        this.search_next();
                                    }
                                    cx.notify();
                                }
                                "backspace" => {
                                    this.search_query.pop();
                                    this.update_search();
                                    cx.notify();
                                }
                                _ => {
                                    // Type into search query
                                    if let Some(ref kc) = event.keystroke.key_char {
                                        if !event.keystroke.modifiers.control
                                            && !event.keystroke.modifiers.alt
                                        {
                                            this.search_query.push_str(kc);
                                            this.update_search();
                                            cx.notify();
                                        }
                                    } else if key.len() == 1
                                        && !event.keystroke.modifiers.control
                                        && !event.keystroke.modifiers.alt
                                    {
                                        this.search_query.push_str(key);
                                        this.update_search();
                                        cx.notify();
                                    }
                                }
                            }
                            return;
                        }

                        // Normal terminal input
                        let app_cursor = this
                            .active_session()
                            .map(|s| s.grid.lock().application_cursor_keys())
                            .unwrap_or(false);
                        if let Some(bytes) = TerminalView::keystroke_to_bytes(event, app_cursor) {
                            // Clear selection on typing
                            if let Some(session) = this.active_session() {
                                session.grid.lock().clear_selection();
                            }
                            if let Some(s) = this.active_session() {
                                s.write_input(&bytes);
                            }
                            // Reset blink so cursor stays visible during typing
                            this.reset_cursor_blink(cx);
                        }
                    });
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &CopySelection, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.copy_selection(cx);
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &PasteClipboard, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.paste_clipboard(cx);
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &ToggleSearch, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.toggle_search();
                            cx.notify();
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &SearchNext, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.search_next();
                            cx.notify();
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &SearchPrev, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.search_prev();
                            cx.notify();
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &ClearTerminal, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, _cx| {
                            if let Some(session) = this.active_session() {
                                let mut grid = session.grid.lock();
                                grid.erase_display(2);
                                grid.cursor_to(0, 0);
                            }
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &SplitHorizontal, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.split_horizontal(cx);
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &SplitVertical, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.split_vertical(cx);
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &TogglePaneZoom, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.toggle_pane_zoom();
                            cx.notify();
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &ZoomIn, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.zoom_in();
                            this.resize_if_needed(_window);
                            cx.notify();
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &ZoomOut, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.zoom_out();
                            this.resize_if_needed(_window);
                            cx.notify();
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &ZoomReset, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.zoom_reset();
                            this.resize_if_needed(_window);
                            cx.notify();
                        });
                    }
                }
            })
            .on_action({
                let h = cx.entity().downgrade();
                move |_: &ToggleSplitFocus, _window: &mut Window, cx: &mut App| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            if this.active_session().is_some_and(|s| s.is_tmux_managed()) {
                                this.tmux_action(TmuxAction::NextPane, cx);
                            } else {
                                this.toggle_split_focus();
                            }
                            cx.notify();
                        });
                    }
                }
            })
            .size_full()
            .bg(self.palette.background_color())
            .p(px(4.0))
            .overflow_hidden()
            // Direct glyph-painting canvas
            .child(Self::create_grid_canvas(
                cache,
                grid,
                cursor,
                self.search_matches.clone(),
                self.search_current_idx,
                self.detected_urls.clone(),
                self.palette.clone(),
                self.has_focus,
                self.cursor_blink_on,
                Some(cx.entity().downgrade()),
            ));

        grid_el
    }

    /// Create a grid canvas element that paints cells, cursor, search matches,
    /// URL underlines and scrollbar.  Shared by both the active and passive
    /// split panes.
    #[allow(clippy::too_many_arguments)]
    fn create_grid_canvas(
        cache: Arc<GlyphCache>,
        grid: Arc<Mutex<TerminalGrid>>,
        cursor: CursorState,
        search_matches: Vec<SearchMatch>,
        search_current: Option<usize>,
        url_matches: Vec<UrlMatch>,
        palette: TerminalPalette,
        has_focus: bool,
        cursor_blink_on: bool,
        capture: Option<WeakEntity<Self>>,
    ) -> impl IntoElement {
        canvas(
            move |bounds, _window, cx| {
                // The focused grid records its real painted origin so mouse-to-cell
                // mapping uses the exact pixel position of cell (0,0) — correct at
                // any UI scale, zoom, or chrome layout, with no analytical guessing.
                if let Some(weak) = &capture {
                    if let Some(view) = weak.upgrade() {
                        let origin = (
                            bounds.origin.x.to_f64() as f32,
                            bounds.origin.y.to_f64() as f32,
                        );
                        view.update(cx, |this, _| {
                            this.grid_x_offset = origin.0;
                            this.grid_y_offset = origin.1;
                            this.grid_right = origin.0 + bounds.size.width.to_f64() as f32;
                            this.grid_bottom = origin.1 + bounds.size.height.to_f64() as f32;
                        });
                    }
                }
                (
                    cache,
                    grid,
                    cursor,
                    search_matches,
                    search_current,
                    url_matches,
                    palette,
                    has_focus,
                    cursor_blink_on,
                )
            },
            move |bounds,
                  (
                cache,
                grid,
                cursor,
                search_matches,
                search_current,
                url_matches,
                palette,
                has_focus,
                cursor_blink_on,
            ),
                  window,
                  _cx| {
                let cell_w = cache.cell_width;
                let cell_h = cache.cell_height;
                let baseline = cache.baseline_y;
                let fs = cache.font_size;

                let grid = grid.lock();
                let visible = grid.visible_rows();

                let sel_color = palette.selection;
                let search_color = palette.search_match;
                let search_current_color = palette.search_current;

                for (ri, row) in visible.iter().enumerate() {
                    let y = bounds.origin.y + cell_h * ri as f32;

                    // Repaint the default background as one strip per row. GPUI
                    // tooltips trigger an isolated frame, so relying on retained
                    // canvas pixels would make full-screen TUIs disappear briefly.
                    window.paint_quad(fill(
                        Bounds::new(point(bounds.origin.x, y), size(bounds.size.width, cell_h)),
                        palette.background_color(),
                    ));

                    for (ci, cell) in row.iter().enumerate() {
                        let x = bounds.origin.x + cell_w * ci as f32;

                        let inverse = cell.attrs.inverse;
                        let (mut fg_t, bg_t) = if inverse {
                            (cell.bg, cell.fg)
                        } else {
                            (cell.fg, cell.bg)
                        };

                        if cell.attrs.bold {
                            fg_t = brighten_for_bold(fg_t);
                        }

                        let eff_fg = if cell.attrs.hidden {
                            bg_t
                        } else if cell.attrs.dim {
                            dim_color(fg_t)
                        } else {
                            fg_t
                        };

                        // Background: always paint for every cell (including spacers)
                        if bg_t != TermColor::Default || inverse {
                            window.paint_quad(fill(
                                Bounds::new(point(x, y), size(cell_w, cell_h)),
                                palette.resolve(&bg_t, inverse),
                            ));
                        }

                        if grid.is_selected(ci, ri) {
                            window.paint_quad(fill(
                                Bounds::new(point(x, y), size(cell_w, cell_h)),
                                sel_color,
                            ));
                        }

                        // Skip glyph rendering for Spacer cells -- the Wide cell's
                        // glyph already covers both columns.
                        if cell.wide == CellWidth::Spacer {
                            continue;
                        }

                        let fg_color = palette.resolve(&eff_fg, !inverse);

                        // Determine the rendering width: wide chars span 2 cell widths.
                        let glyph_w = if cell.wide == CellWidth::Wide {
                            cell_w * 2.0
                        } else {
                            cell_w
                        };

                        let ch = cell.c;
                        if ch != ' ' && ch != '\0' {
                            if paint_block_char(ch, x, y, glyph_w, cell_h, fg_color, window) {
                                // Handled by procedural block/box-drawing renderer
                            } else {
                                let f = match (cell.attrs.bold, cell.attrs.italic) {
                                    (true, true) => cache.base_font.clone().bold().italic(),
                                    (true, false) => cache.base_font.clone().bold(),
                                    (false, true) => cache.base_font.clone().italic(),
                                    _ => cache.base_font.clone(),
                                };
                                // Build the string: base char + combining chars
                                let mut char_str = ch.to_string();
                                for &comb in &cell.combining {
                                    char_str.push(comb);
                                }
                                let s: SharedString = char_str.into();
                                let blen = s.len();
                                let shaped = window.text_system().shape_line(
                                    s,
                                    fs,
                                    &[TextRun {
                                        len: blen,
                                        font: f,
                                        color: fg_color,
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    }],
                                    None,
                                );
                                let _ = shaped.paint(point(x, y), cell_h, window, _cx);
                            }
                        }

                        // Underline color: use dedicated underline_color if set,
                        // otherwise fall back to the foreground color.
                        let ul_color = if let Some(ref uc) = cell.attrs.underline_color {
                            palette.resolve(uc, true)
                        } else {
                            fg_color
                        };

                        // Styled underline rendering
                        let underline_y = y + cell_h - px(2.0);
                        let gw_f32 = glyph_w.to_f64() as f32;
                        match cell.attrs.underline {
                            UnderlineStyle::None => {}
                            UnderlineStyle::Single => {
                                window.paint_quad(fill(
                                    Bounds::new(point(x, underline_y), size(glyph_w, px(1.0))),
                                    ul_color,
                                ));
                            }
                            UnderlineStyle::Double => {
                                // Two parallel lines 2px apart
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(x, underline_y - px(1.0)),
                                        size(glyph_w, px(1.0)),
                                    ),
                                    ul_color,
                                ));
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(x, underline_y + px(1.0)),
                                        size(glyph_w, px(1.0)),
                                    ),
                                    ul_color,
                                ));
                            }
                            UnderlineStyle::Curly => {
                                // Wavy/zigzag underline: small quads alternating y position
                                let wave_period = 4.0_f32;
                                let wave_height = 2.0_f32;
                                let mut xoff = 0.0_f32;
                                let mut segment_idx = 0;
                                while xoff < gw_f32 {
                                    let seg_w = wave_period.min(gw_f32 - xoff);
                                    let y_shift = if segment_idx % 2 == 0 {
                                        0.0
                                    } else {
                                        wave_height
                                    };
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(x + px(xoff), underline_y + px(y_shift)),
                                            size(px(seg_w), px(1.0)),
                                        ),
                                        ul_color,
                                    ));
                                    xoff += wave_period;
                                    segment_idx += 1;
                                }
                            }
                            UnderlineStyle::Dotted => {
                                // Dots: 1px on, 1px off
                                let mut xoff = 0.0_f32;
                                while xoff < gw_f32 {
                                    let dot_w = 1.0_f32.min(gw_f32 - xoff);
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(x + px(xoff), underline_y),
                                            size(px(dot_w), px(1.0)),
                                        ),
                                        ul_color,
                                    ));
                                    xoff += 2.0;
                                }
                            }
                            UnderlineStyle::Dashed => {
                                // Dashes: 3px on, 2px off
                                let mut xoff = 0.0_f32;
                                while xoff < gw_f32 {
                                    let dash_w = 3.0_f32.min(gw_f32 - xoff);
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(x + px(xoff), underline_y),
                                            size(px(dash_w), px(1.0)),
                                        ),
                                        ul_color,
                                    ));
                                    xoff += 5.0;
                                }
                            }
                        }

                        // Overline: drawn at the top of the cell
                        if cell.attrs.overline {
                            window.paint_quad(fill(
                                Bounds::new(point(x, y + px(1.0)), size(glyph_w, px(1.0))),
                                fg_color,
                            ));
                        }

                        // Strikethrough: horizontal line at vertical center
                        if cell.attrs.strikethrough {
                            window.paint_quad(fill(
                                Bounds::new(point(x, y + cell_h / 2.0), size(glyph_w, px(1.0))),
                                fg_color,
                            ));
                        }
                    }
                }

                // URL underlines
                let url_underline_color = hsla(0.58, 0.6, 0.6, 0.6);
                for url in &url_matches {
                    let y = bounds.origin.y + cell_h * url.row as f32;
                    let underline_y = y + cell_h - px(1.0);
                    for offset in 0..url.len {
                        let col = url.col + offset;
                        let x = bounds.origin.x + cell_w * col as f32;
                        if offset % 2 == 0 {
                            window.paint_quad(fill(
                                Bounds::new(point(x, underline_y), size(cell_w, px(1.0))),
                                url_underline_color,
                            ));
                        }
                    }
                }

                // Search match highlights
                for (mi, m) in search_matches.iter().enumerate() {
                    let is_current = search_current == Some(mi);
                    let color = if is_current {
                        search_current_color
                    } else {
                        search_color
                    };
                    for offset in 0..m.len {
                        let col = m.col + offset;
                        let y = bounds.origin.y + cell_h * m.row as f32;
                        let x = bounds.origin.x + cell_w * col as f32;
                        window.paint_quad(fill(
                            Bounds::new(point(x, y), size(cell_w, cell_h)),
                            color,
                        ));
                    }
                }

                // Cursor
                // Determine whether the cursor should be drawn at all:
                // - Must be marked visible by the grid
                // - If the grid's blink flag is set, respect the blink timer phase
                let should_draw_cursor = cursor.visible
                    && cursor.row < visible.len()
                    && (!cursor.blink || cursor_blink_on);

                if should_draw_cursor {
                    let cx_x = bounds.origin.x + cell_w * cursor.col as f32;
                    let cx_y = bounds.origin.y + cell_h * cursor.row as f32;
                    let cursor_color = palette.cursor;

                    // Determine cursor width: 2 cells for wide chars, 1 for normal.
                    let cursor_w = if let Some(row) = visible.get(cursor.row) {
                        if let Some(cell) = row.get(cursor.col) {
                            if cell.wide == CellWidth::Wide {
                                cell_w * 2.0
                            } else {
                                cell_w
                            }
                        } else {
                            cell_w
                        }
                    } else {
                        cell_w
                    };

                    if !has_focus && cursor.shape == CursorShape::Block {
                        // Hollow cursor: outline only (unfocused block)
                        let bw = px(1.0);
                        // Top edge
                        window.paint_quad(fill(
                            Bounds::new(point(cx_x, cx_y), size(cursor_w, bw)),
                            cursor_color,
                        ));
                        // Bottom edge
                        window.paint_quad(fill(
                            Bounds::new(point(cx_x, cx_y + cell_h - bw), size(cursor_w, bw)),
                            cursor_color,
                        ));
                        // Left edge
                        window.paint_quad(fill(
                            Bounds::new(point(cx_x, cx_y), size(bw, cell_h)),
                            cursor_color,
                        ));
                        // Right edge
                        window.paint_quad(fill(
                            Bounds::new(point(cx_x + cursor_w - bw, cx_y), size(bw, cell_h)),
                            cursor_color,
                        ));
                    } else {
                        // Focused cursor (or unfocused non-block shapes)
                        match cursor.shape {
                            CursorShape::Block => {
                                window.paint_quad(fill(
                                    Bounds::new(point(cx_x, cx_y), size(cursor_w, cell_h)),
                                    cursor_color,
                                ));
                                // Draw the character under the cursor in the background color
                                // so it remains readable against the filled block.
                                if let Some(row) = visible.get(cursor.row) {
                                    if let Some(cell) = row.get(cursor.col) {
                                        let ch = cell.c;
                                        if ch != ' ' && ch != '\0' {
                                            let bg = palette.background_color();
                                            if let Some((fid, gid)) =
                                                cache.lookup(ch, cell.attrs.bold, cell.attrs.italic)
                                            {
                                                let _ = window.paint_glyph(
                                                    point(cx_x, cx_y + baseline),
                                                    fid,
                                                    gid,
                                                    fs,
                                                    bg,
                                                );
                                            } else {
                                                // Non-ASCII (e.g. CJK) under cursor: use shape_line
                                                let f = match (cell.attrs.bold, cell.attrs.italic) {
                                                    (true, true) => {
                                                        cache.base_font.clone().bold().italic()
                                                    }
                                                    (true, false) => cache.base_font.clone().bold(),
                                                    (false, true) => {
                                                        cache.base_font.clone().italic()
                                                    }
                                                    _ => cache.base_font.clone(),
                                                };
                                                let mut char_str = ch.to_string();
                                                for &comb in &cell.combining {
                                                    char_str.push(comb);
                                                }
                                                let s: SharedString = char_str.into();
                                                let blen = s.len();
                                                let shaped = window.text_system().shape_line(
                                                    s,
                                                    fs,
                                                    &[TextRun {
                                                        len: blen,
                                                        font: f,
                                                        color: bg,
                                                        background_color: None,
                                                        underline: None,
                                                        strikethrough: None,
                                                    }],
                                                    None,
                                                );
                                                let _ = shaped.paint(
                                                    point(cx_x, cx_y),
                                                    cell_h,
                                                    window,
                                                    _cx,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            CursorShape::Bar => {
                                window.paint_quad(fill(
                                    Bounds::new(point(cx_x, cx_y), size(px(2.0), cell_h)),
                                    ShellDeckColors::primary(),
                                ));
                            }
                            CursorShape::Underline => {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(cx_x, cx_y + cell_h - px(2.0)),
                                        size(cursor_w, px(2.0)),
                                    ),
                                    ShellDeckColors::primary(),
                                ));
                            }
                        }
                    }
                }

                // Scrollbar
                let (total_lines, visible_lines, scroll_off) = grid.scroll_info();
                if total_lines > visible_lines {
                    let scrollbar_w = px(SCROLLBAR_WIDTH);
                    // Use actual grid content height, not full canvas height
                    let grid_content_height = cell_h * visible_lines as f32;
                    let track_height = grid_content_height.min(bounds.size.height);
                    let track_x =
                        bounds.origin.x + bounds.size.width - scrollbar_w - px(SCROLLBAR_MARGIN);
                    let track_y = bounds.origin.y;

                    window.paint_quad(fill(
                        Bounds::new(point(track_x, track_y), size(scrollbar_w, track_height)),
                        // Faint: the track sits inside a pane card.
                        hsla(0.0, 0.0, 0.5, 0.12),
                    ));

                    let ratio = visible_lines as f32 / total_lines as f32;
                    let thumb_height = (track_height * ratio).max(px(20.0));
                    let scrollable = total_lines - visible_lines;
                    let position = if scrollable > 0 {
                        1.0 - (scroll_off as f32 / scrollable as f32)
                    } else {
                        1.0
                    };
                    let thumb_y = track_y + (track_height - thumb_height) * position;

                    window.paint_quad(PaintQuad {
                        bounds: Bounds::new(
                            point(track_x, thumb_y),
                            size(scrollbar_w, thumb_height),
                        ),
                        corner_radii: Corners::all(px(3.0)),
                        background: hsla(0.0, 0.0, 0.5, 0.5).into(),
                        border_widths: Edges::default(),
                        border_color: transparent_black(),
                        border_style: BorderStyle::default(),
                        continuous_corners: false,
                        transform: TransformationMatrix::unit(),
                        blend_mode: BlendMode::default(),
                    });
                }
            },
        )
        .size_full()
    }

    /// Convert a pixel position (window-relative) to grid cell coordinates.
    /// Returns (col, row) 1-indexed for terminal escape sequences.
    fn pixel_to_cell(&self, position: Point<Pixels>) -> Option<(u16, u16)> {
        let fs = self.effective_font_size();
        let cell_width = fs * 0.6;
        let cell_height = fs * 1.4;

        let grid_x = position.x - px(self.grid_x_offset);
        let grid_y = position.y - px(self.grid_y_offset);

        if grid_x < px(0.0) || grid_y < px(0.0) {
            return None;
        }

        let col = (grid_x / px(cell_width)).floor() as u16 + 1;
        let row = (grid_y / px(cell_height)).floor() as u16 + 1;

        if col < 1 || row < 1 || col > self.last_grid_cols || row > self.last_grid_rows {
            return None;
        }

        Some((col, row))
    }

    /// Convert a pixel position to 0-indexed grid cell coordinates (for selection).
    fn pixel_to_cell_zero(&self, position: Point<Pixels>) -> Option<(usize, usize)> {
        let fs = self.effective_font_size();
        let cell_width = fs * 0.6;
        let cell_height = fs * 1.4;

        let grid_x = position.x - px(self.grid_x_offset);
        let grid_y = position.y - px(self.grid_y_offset);

        if grid_x < px(0.0) || grid_y < px(0.0) {
            return None;
        }

        let col = (grid_x / px(cell_width)).floor() as usize;
        let row = (grid_y / px(cell_height)).floor() as usize;

        let max_col = (self.last_grid_cols as usize).saturating_sub(1);
        let max_row = (self.last_grid_rows as usize).saturating_sub(1);

        Some((col.min(max_col), row.min(max_row)))
    }

    /// Check if a window-relative position falls on the scrollbar track
    /// (rightmost ~10px of the focused grid, as it was last painted).
    fn is_in_scrollbar_area(&self, position: Point<Pixels>, _window: &Window) -> bool {
        if self.grid_right <= 0.0 {
            return false;
        }
        let scrollbar_start = px(self.grid_right - (SCROLLBAR_WIDTH + SCROLLBAR_MARGIN + 2.0));
        position.x >= scrollbar_start
            && position.x <= px(self.grid_right + layout::PANE_INSET_X)
            && position.y >= px(self.grid_y_offset)
            && position.y <= px(self.grid_bottom)
    }

    /// Scroll the active session's grid to a position based on the mouse y
    /// coordinate within the scrollbar track.
    fn scrollbar_scroll_to_y(&mut self, y: Pixels, _window: &Window) {
        let track_top = px(self.grid_y_offset);
        let track_height = px(self.grid_bottom) - track_top;
        if track_height <= px(0.0) {
            return;
        }

        let relative_y = (y - track_top).max(px(0.0)).min(track_height);
        let fraction = relative_y / track_height;

        if let Some(session) = self.active_session() {
            let mut grid = session.grid.lock();
            let (total, visible, _) = grid.scroll_info();
            if total > visible {
                let scrollable = total - visible;
                // fraction 0.0 = top (max scroll_up), 1.0 = bottom (scroll_offset = 0)
                let offset = ((1.0 - fraction) * scrollable as f32).round() as usize;
                grid.set_scroll_offset(offset.min(scrollable));
            }
        }
    }

    /// Copy selected text to clipboard.
    fn copy_selection(&self, cx: &App) {
        if let Some(session) = self.active_session() {
            let grid = session.grid.lock();
            if let Some(text) = grid.selected_text() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
    }

    /// Toggle search bar visibility.
    fn toggle_search(&mut self) {
        self.search_visible = !self.search_visible;
        if !self.search_visible {
            self.search_query.clear();
            self.search_matches.clear();
            self.search_current_idx = None;
        }
    }

    /// Update search results based on current query.
    fn update_search(&mut self) {
        if self.search_query.is_empty() {
            self.search_matches.clear();
            self.search_current_idx = None;
            return;
        }
        let grid_arc = self.active_session().map(|s| s.grid.clone());
        if let Some(grid_arc) = grid_arc {
            let grid = grid_arc.lock();
            let matches = grid.search(
                &self.search_query,
                self.search_case_sensitive,
                self.search_regex,
            );
            drop(grid);
            if matches.is_empty() {
                self.search_current_idx = None;
            } else {
                self.search_current_idx = Some(matches.len().saturating_sub(1));
            }
            self.search_matches = matches;
        }
    }

    /// Navigate to next search match.
    fn search_next(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let idx = self.search_current_idx.unwrap_or(0);
        self.search_current_idx = Some((idx + 1) % self.search_matches.len());
    }

    /// Navigate to previous search match.
    fn search_prev(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let idx = self.search_current_idx.unwrap_or(0);
        self.search_current_idx = Some(if idx == 0 {
            self.search_matches.len() - 1
        } else {
            idx - 1
        });
    }

    /// Render the search bar overlay.
    fn render_search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let match_info = if self.search_matches.is_empty() {
            if self.search_query.is_empty() {
                String::new()
            } else {
                t!("terminal.search.no_matches").to_string()
            }
        } else {
            let current = self.search_current_idx.map(|i| i + 1).unwrap_or(0);
            t!(
                "terminal.search.match_count",
                current = current,
                total = self.search_matches.len()
            )
            .to_string()
        };

        div()
            .absolute()
            .top(px(8.0))
            .right(px(16.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .py(px(6.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(8.0))
            .shadow_md()
            .child(div().text_size(px(13.0)).min_w(px(150.0)).child(
                if self.search_query.is_empty() {
                    div()
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("terminal.search.placeholder").to_string())
                } else {
                    div()
                        .text_color(ShellDeckColors::text_primary())
                        .flex()
                        .child(self.search_query.clone())
                        .child(div().w(px(1.0)).h(px(14.0)).bg(ShellDeckColors::primary()))
                },
            ))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(match_info),
            )
            .child(
                // Prev button
                div()
                    .id("search-prev")
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                    .child("<")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.search_prev();
                        cx.notify();
                    })),
            )
            .child(
                // Next button
                div()
                    .id("search-next")
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                    .child(">")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.search_next();
                        cx.notify();
                    })),
            )
            .child(
                // Close button
                div()
                    .id("search-close")
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|el| el.text_color(ShellDeckColors::error()))
                    .child(
                        svg()
                            .path("icons/lucide/x.svg")
                            .size(px(12.0))
                            .text_color(ShellDeckColors::text_muted()),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_search();
                        cx.notify();
                    })),
            )
    }

    /// Paste clipboard content into the terminal.
    fn paste_clipboard(&self, cx: &App) {
        if let Some(item) = cx.read_from_clipboard() {
            if let Some(text) = item.text() {
                if let Some(session) = self.active_session() {
                    let bracketed = session.grid.lock().bracketed_paste();
                    if bracketed {
                        let mut data = Vec::with_capacity(text.len() + 12);
                        data.extend_from_slice(b"\x1b[200~");
                        data.extend_from_slice(text.as_bytes());
                        data.extend_from_slice(b"\x1b[201~");
                        session.write_input(&data);
                    } else {
                        session.write_input(text.as_bytes());
                    }
                }
            }
        }
    }

    /// Render the context menu overlay.
    fn render_context_menu(
        &self,
        state: &ContextMenuState,
        // Window-space origin of the element the menu is placed in.
        origin: (f32, f32),
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut menu = div()
            .id("context-menu")
            .absolute()
            .left(state.position.x - px(origin.0))
            .top(state.position.y - px(origin.1))
            .min_w(px(180.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(8.0))
            .shadow_lg()
            .py(px(4.0))
            .flex()
            .flex_col()
            // Capture mouse events so they don't pass through to the grid underneath.
            .on_mouse_down(MouseButton::Left, |_, _, _| {})
            .on_mouse_down(MouseButton::Right, |_, _, _| {});

        let menu_item = |id: &str, label: &str| {
            div()
                .id(ElementId::from(SharedString::from(id.to_string())))
                .px(px(12.0))
                .py(px(6.0))
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_primary())
                .cursor_pointer()
                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                .child(label.to_string())
        };

        let ctx_copy = t!("terminal.ctx.copy");
        let ctx_paste = t!("terminal.ctx.paste");
        let ctx_search = t!("terminal.ctx.search");
        let ctx_select_all = t!("terminal.ctx.select_all");
        let ctx_clear_terminal = t!("terminal.ctx.clear_terminal");
        let ctx_open_link = t!("terminal.ctx.open_link");
        let ctx_copy_link = t!("terminal.ctx.copy_link");

        menu = menu
            .child(
                menu_item("ctx-copy", ctx_copy.as_ref()).on_click(cx.listener(|this, _, _, cx| {
                    this.copy_selection(cx);
                    this.context_menu = None;
                    cx.notify();
                })),
            )
            .child(
                menu_item("ctx-paste", ctx_paste.as_ref()).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.paste_clipboard(cx);
                        this.context_menu = None;
                        cx.notify();
                    },
                )),
            )
            .child(
                div()
                    .h(px(1.0))
                    .mx(px(8.0))
                    .my(px(4.0))
                    .bg(ShellDeckColors::border()),
            )
            .child(
                menu_item("ctx-select-all", ctx_select_all.as_ref()).on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(session) = this.active_session() {
                            let mut grid = session.grid.lock();
                            let rows = grid.rows;
                            let cols = grid.cols;
                            grid.start_selection(0, 0);
                            grid.update_selection(cols.saturating_sub(1), rows.saturating_sub(1));
                            grid.end_selection();
                        }
                        this.context_menu = None;
                        cx.notify();
                    },
                )),
            )
            .child(
                menu_item("ctx-clear", ctx_clear_terminal.as_ref()).on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(session) = this.active_session() {
                            let mut grid = session.grid.lock();
                            grid.erase_display(2);
                            grid.cursor_to(0, 0);
                        }
                        this.context_menu = None;
                        cx.notify();
                    },
                )),
            )
            .child(
                div()
                    .h(px(1.0))
                    .mx(px(8.0))
                    .my(px(4.0))
                    .bg(ShellDeckColors::border()),
            )
            .child(
                menu_item("ctx-search", ctx_search.as_ref()).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.search_visible = true;
                        this.context_menu = None;
                        cx.notify();
                    },
                )),
            );

        // URL actions if applicable
        if let Some(url) = &state.url {
            let url_clone = url.clone();
            let url_copy = url.clone();
            menu = menu
                .child(
                    div()
                        .h(px(1.0))
                        .mx(px(8.0))
                        .my(px(4.0))
                        .bg(ShellDeckColors::border()),
                )
                .child(
                    menu_item("ctx-open-link", ctx_open_link.as_ref()).on_click(cx.listener(
                        move |this, _, _, cx| {
                            let _ = open::that(&url_clone);
                            this.context_menu = None;
                            cx.notify();
                        },
                    )),
                )
                .child(
                    menu_item("ctx-copy-link", ctx_copy_link.as_ref()).on_click(cx.listener(
                        move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(url_copy.clone()));
                            this.context_menu = None;
                            cx.notify();
                        },
                    )),
                );
        }

        menu
    }

    /// Render the right-click context menu for a terminal tab, plus a
    /// transparent backdrop that dismisses it on any outside click.
    fn render_tab_context_menu(
        &self,
        state: &TabMenuState,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tab_id = state.tab_id;
        let has_left = self.tab_has_left(tab_id);
        let has_right = self.tab_has_right(tab_id);

        let tab_new = t!("terminal.tab.new");
        let tab_duplicate = t!("terminal.tab.duplicate");
        let tab_close = t!("terminal.tab.close");
        let tab_close_left = t!("terminal.tab.close_left");
        let tab_close_right = t!("terminal.tab.close_right");

        // Convert the window-relative click x into terminal-view-local x
        // (the view starts just right of the sidebar). The menu drops down
        // just below the tab bar.
        let scale = Self::ui_scale(window);
        let left = (state.position.x - px(self.content_left())).max(px(0.0));
        // Opened from the session list there is no tab strip to hang from:
        // the menu opens at the height of the row, in view coordinates.
        let top = if self.session_list_shown() {
            (state.position.y - px(TITLEBAR_HEIGHT * scale)).max(px(0.0))
        } else {
            px(TAB_BAR_HEIGHT * scale)
        };

        // Enabled menu item.
        let item = |id: &str, label: &str| {
            div()
                .id(ElementId::from(SharedString::from(id.to_string())))
                .px(px(12.0))
                .py(px(6.0))
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_primary())
                .cursor_pointer()
                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                .child(label.to_string())
        };
        // Disabled (greyed, non-interactive) menu item.
        let disabled_item = |label: &str| {
            div()
                .px(px(12.0))
                .py(px(6.0))
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_muted().opacity(0.5))
                .child(label.to_string())
        };
        let separator = || {
            div()
                .h(px(1.0))
                .mx(px(8.0))
                .my(px(4.0))
                .bg(ShellDeckColors::border())
        };

        let mut menu = div()
            .id("tab-context-menu")
            .absolute()
            .left(left)
            .top(top)
            .min_w(px(190.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(8.0))
            .shadow_lg()
            .py(px(4.0))
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, _| {})
            .on_mouse_down(MouseButton::Right, |_, _, _| {});

        menu = menu
            .child(item("tab-ctx-new", tab_new.as_ref()).on_click(cx.listener(
                |this, _, window, cx| {
                    this.tab_context_menu = None;
                    this.spawn_local_terminal(cx);
                    this.focus_handle.focus(window);
                    cx.emit(TerminalEvent::NewTabRequested);
                    cx.notify();
                },
            )))
            .child(
                item("tab-ctx-duplicate", tab_duplicate.as_ref()).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.tab_context_menu = None;
                        this.duplicate_tab(tab_id, cx);
                        cx.notify();
                    },
                )),
            )
            .child(separator())
            .child(
                item("tab-ctx-close", tab_close.as_ref()).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.tab_context_menu = None;
                        this.close_tab(tab_id);
                        cx.emit(TerminalEvent::TabClosed(tab_id));
                        cx.notify();
                    },
                )),
            );

        if has_left {
            menu = menu.child(
                item("tab-ctx-close-left", tab_close_left.as_ref()).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.tab_context_menu = None;
                        this.close_tabs_to_left(tab_id);
                        cx.emit(TerminalEvent::TabClosed(tab_id));
                        cx.notify();
                    },
                )),
            );
        } else {
            menu = menu.child(disabled_item(tab_close_left.as_ref()));
        }

        if has_right {
            menu = menu.child(
                item("tab-ctx-close-right", tab_close_right.as_ref()).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.tab_context_menu = None;
                        this.close_tabs_to_right(tab_id);
                        cx.emit(TerminalEvent::TabClosed(tab_id));
                        cx.notify();
                    },
                )),
            );
        } else {
            menu = menu.child(disabled_item(tab_close_right.as_ref()));
        }

        // Transparent backdrop captures outside clicks to dismiss the menu.
        let backdrop = div()
            .id("tab-context-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.tab_context_menu = None;
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, _, cx| {
                    this.tab_context_menu = None;
                    cx.notify();
                }),
            );

        // The layer covers the view it is added to. Left in the flow, it was
        // an empty box under the panes: the menu and its backdrop, placed
        // from that box, fell outside the window.
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(backdrop)
            .child(menu)
    }

    /// Render a read-only grid for the unfocused split pane.
    /// Clicking anywhere on it toggles focus to this pane.
    fn render_split_passive_grid(
        &self,
        focus_target: PaneId,
        grid_arc: Arc<parking_lot::Mutex<TerminalGrid>>,
        cache: Arc<GlyphCache>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut cursor = grid_arc.lock().cursor.clone();
        cursor.shape = self.effective_cursor_shape(cursor.shape);
        let focus = self.focus_handle.clone();

        let focus2 = self.focus_handle.clone();
        let target_id = ElementId::from(SharedString::from(format!("passive-{focus_target:?}")));

        div()
            .id(target_id)
            .relative()
            .size_full()
            .bg(self.palette.background_color())
            .p(px(4.0))
            .overflow_hidden()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                    this.focus_pane(focus_target);
                    this.needs_focus = true;
                    focus.focus(window);
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.focus_pane(focus_target);
                    this.needs_focus = true;
                    focus2.focus(window);
                    this.context_menu = Some(ContextMenuState {
                        position: event.position,
                        url: None,
                    });
                    cx.notify();
                }),
            )
            .child(Self::create_grid_canvas(
                cache,
                grid_arc,
                cursor,
                vec![],
                None,
                vec![],
                self.palette.clone(),
                false, // passive grid is never focused
                true,  // cursor always visible (no blink) in passive pane
                None,  // passive panes never drive the click offset
            ))
    }

    /// Get the effective font size (base * zoom).
    fn effective_font_size(&self) -> f32 {
        let zoom = self
            .tabs
            .get(self.pane.active_index)
            .map(|t| t.zoom_level)
            .unwrap_or(1.0);
        self.font_size * zoom
    }

    /// Zoom in on the active terminal.
    fn zoom_in(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.pane.active_index) {
            tab.zoom_level = (tab.zoom_level * 1.1).min(3.0);
            self.glyph_cache = None; // Invalidate
        }
    }

    /// Zoom out on the active terminal.
    fn zoom_out(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.pane.active_index) {
            tab.zoom_level = (tab.zoom_level / 1.1).max(0.5);
            self.glyph_cache = None; // Invalidate
        }
    }

    /// Reset zoom to 1.0.
    fn zoom_reset(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.pane.active_index) {
            tab.zoom_level = 1.0;
            self.glyph_cache = None; // Invalidate
        }
    }

    /// Split the current pane horizontally (side by side).
    pub fn split_horizontal(&mut self, cx: &mut Context<Self>) {
        self.do_split(SplitDirection::Horizontal, cx);
    }

    /// Split the current pane vertically (top/bottom).
    pub fn split_vertical(&mut self, cx: &mut Context<Self>) {
        self.do_split(SplitDirection::Vertical, cx);
    }

    fn do_split(&mut self, direction: SplitDirection, cx: &mut Context<Self>) {
        if self.pane.sessions.is_empty() {
            return;
        }

        if self.active_session().is_some_and(|s| s.is_tmux_managed()) {
            self.tmux_action(
                match direction {
                    SplitDirection::Horizontal => TmuxAction::SplitHorizontal,
                    SplitDirection::Vertical => TmuxAction::SplitVertical,
                },
                cx,
            );
            return;
        }

        // SSH tabs ask the workspace to open another session for the connection;
        // it then calls `set_split_session` to install it as a new pane.
        let connection_id = self
            .tabs
            .get(self.pane.active_index)
            .and_then(|t| t.connection_id);

        if let Some(conn_id) = connection_id {
            cx.emit(TerminalEvent::SplitRequested {
                connection_id: conn_id,
                direction,
            });
            return;
        }

        // Local terminal: spawn directly.
        self.spawn_local_split(direction, cx);
    }

    fn tmux_action(&self, action: TmuxAction, cx: &mut Context<Self>) {
        if !self
            .active_session()
            .is_some_and(|session| session.request_tmux_action(action))
        {
            cx.emit(TerminalEvent::TmuxUnavailable);
        }
    }

    /// Spawn a local terminal and add it as a new pane splitting the focused one.
    fn spawn_local_split(&mut self, direction: SplitDirection, cx: &mut Context<Self>) {
        let (rows, cols) = if self.last_grid_rows > 0 {
            (self.last_grid_rows, self.last_grid_cols)
        } else {
            (24, 80)
        };
        let new_session = match self.spawn_configured_local(rows, cols) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to spawn split terminal: {}", e);
                return;
            }
        };
        self.install_split_pane(new_session, direction);
        self.ensure_refresh_running(cx);
        cx.notify();
    }

    /// Install `session` as a new pane splitting the currently focused pane.
    fn install_split_pane(&mut self, session: TerminalSession, direction: SplitDirection) {
        let new_id = Uuid::new_v4();
        let target = self.layout.focused;
        session
            .grid
            .lock()
            .set_max_scrollback(self.configured_scrollback);
        // Wire the split's reader thread to wake the UI on output.
        session.set_output_notifier(self.output_tx.clone());
        self.layout.extra.insert(new_id, session);
        self.layout.split_leaf(target, direction, new_id);
        self.layout.focused = PaneId::Extra(new_id);
        self.layout.zoomed = false;
        // Force a resize pass to size the new pane on next render.
        self.last_grid_rows = 0;
        self.last_grid_cols = 0;
    }

    /// Set a split pane from an externally-created session (e.g. SSH), splitting
    /// the currently focused pane.
    pub fn set_split_session(
        &mut self,
        session: TerminalSession,
        direction: SplitDirection,
        cx: &mut Context<Self>,
    ) {
        self.install_split_pane(session, direction);
        self.ensure_refresh_running(cx);
        cx.notify();
    }

    /// Flip the orientation of the split that holds the focused pane.
    fn toggle_split_direction(&mut self) {
        let target = self.layout.focused;
        fn flip(node: &mut PaneNode, target: PaneId) -> bool {
            match node {
                PaneNode::Leaf(_) => false,
                PaneNode::Split {
                    direction, a, b, ..
                } => {
                    let child_is_target = matches!(**a, PaneNode::Leaf(id) if id == target)
                        || matches!(**b, PaneNode::Leaf(id) if id == target);
                    if child_is_target {
                        *direction = match direction {
                            SplitDirection::Horizontal => SplitDirection::Vertical,
                            SplitDirection::Vertical => SplitDirection::Horizontal,
                        };
                        return true;
                    }
                    flip(a, target) || flip(b, target)
                }
            }
        }
        flip(&mut self.layout.tree, target);
        self.last_grid_rows = 0;
        self.last_grid_cols = 0;
    }

    /// Close the focused pane. Extra panes are simply dropped; closing the
    /// primary pane promotes another pane's session into the primary slot so the
    /// tab keeps a valid `pane.sessions` entry. No-op if only one pane remains.
    fn close_split(&mut self) {
        let leaves = self.layout.leaves();
        if leaves.len() <= 1 {
            return;
        }
        match self.layout.focused {
            PaneId::Extra(id) => {
                self.layout.remove_leaf(PaneId::Extra(id));
                if let Some(session) = self.layout.extra.remove(&id) {
                    session.terminate();
                }
            }
            PaneId::Primary => {
                // Promote the first extra pane into the primary slot.
                let Some(successor) = leaves.iter().find_map(|p| match p {
                    PaneId::Extra(id) => Some(*id),
                    PaneId::Primary => None,
                }) else {
                    return;
                };
                if let Some(session) = self.layout.extra.remove(&successor) {
                    let idx = self.pane.active_index;
                    if idx < self.pane.sessions.len() {
                        if let Some(tab) = self.tabs.get_mut(idx) {
                            tab.title = session.title.clone();
                            tab.state = session.state.clone();
                        }
                        let closed = std::mem::replace(&mut self.pane.sessions[idx], session);
                        closed.terminate();
                    }
                }
                // The primary leaf now shows the promoted session; drop the
                // successor's leaf from the tree.
                self.layout.remove_leaf(PaneId::Extra(successor));
            }
        }
        self.layout.focused = PaneId::Primary;
        self.layout.zoomed = false;
        self.last_grid_rows = 0;
        self.last_grid_cols = 0;
    }

    /// Encode a mouse event as a terminal escape sequence.
    fn encode_mouse(
        encoding: MouseEncoding,
        button: u8,
        col: u16,
        row: u16,
        press: bool,
    ) -> Vec<u8> {
        match encoding {
            MouseEncoding::Sgr => {
                let suffix = if press { 'M' } else { 'm' };
                format!("\x1b[<{};{};{}{}", button, col, row, suffix).into_bytes()
            }
            MouseEncoding::Normal => {
                let b = button + 32;
                let c = (col as u8).saturating_add(32);
                let r = (row as u8).saturating_add(32);
                vec![0x1b, b'[', b'M', b, c, r]
            }
        }
    }

    /// Claude brand orange (#D97757).
    fn claude_orange() -> gpui::Hsla {
        gpui::rgb(0xD97757).into()
    }

    fn codex_green() -> gpui::Hsla {
        gpui::rgb(0x10A37F).into()
    }

    /// A rounded badge bearing the Simple Icons Claude Code mark.
    fn claude_logo(size: f32) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(size))
            .h(px(size))
            .rounded(px(size * 0.26))
            .bg(Self::claude_orange())
            .child(crate::icons::simple_icon(
                "claudecode",
                size * 0.72,
                gpui::white(),
            ))
    }

    fn codex_logo(size: f32) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(size))
            .h(px(size))
            .rounded(px(size * 0.26))
            .bg(Self::codex_green())
            .child(crate::icons::simple_icon(
                "openai",
                size * 0.72,
                gpui::white(),
            ))
    }

    fn terminal_logo(size: f32) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(size))
            .h(px(size))
            .rounded(px(size * 0.26))
            .bg(ShellDeckColors::primary())
            .child(crate::icons::lucide_icon(
                "terminal",
                size * 0.58,
                gpui::white(),
            ))
    }

    /// Two-line launch actions are custom because adabraka's Button supports a
    /// single label but not the command preview used by this terminal surface.
    fn launcher_card(
        id: &'static str,
        title: String,
        caption: String,
        command: SharedString,
        accent: Hsla,
        logo: AnyElement,
        width: f32,
    ) -> Stateful<Div> {
        use crate::scale::px;
        let card = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .w(px(width))
            .h(px(84.0))
            .flex_shrink_0()
            .px(px(14.0))
            .py(px(12.0))
            .rounded(px(8.0))
            .bg(if id == "empty-launch-terminal" {
                accent
            } else {
                ShellDeckColors::bg_surface()
            })
            .border_1()
            .border_color(if id == "empty-launch-terminal" {
                accent
            } else {
                ShellDeckColors::border()
            })
            .cursor_pointer()
            .hover(move |el| {
                el.bg(if id == "empty-launch-terminal" {
                    accent.opacity(0.88)
                } else {
                    ShellDeckColors::hover_bg()
                })
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(26.0))
                    .flex_shrink_0()
                    .child(logo),
            )
            .child(
                div()
                    .id(SharedString::from(format!("{id}-text")))
                    .debug_selector(move || format!("{id}-text"))
                    .flex()
                    .flex_col()
                    .w(px((width - 66.0).max(1.0)))
                    .h(px(56.0))
                    .justify_center()
                    .gap(px(2.0))
                    .overflow_hidden()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .w_full()
                            .line_height(px(18.0))
                            .line_clamp(2)
                            .max_h(px(36.0))
                            .flex_shrink_0()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if id == "empty-launch-terminal" {
                                gpui::white()
                            } else {
                                ShellDeckColors::text_primary()
                            })
                            .child(title),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(14.0))
                            .line_height(px(14.0))
                            .flex_shrink_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(10.0))
                            .text_color(if id == "empty-launch-terminal" {
                                gpui::white().opacity(0.85)
                            } else {
                                ShellDeckColors::text_muted()
                            })
                            .child(caption),
                    ),
            );
        trace_layout_for_preview(id.into(), card)
            .id(id)
            .debug_selector(move || id.into())
            // La commande exacte reste consultable, mais au survol : dans la
            // carte elle se coupait en plein mot — « --dangerously-skip-permiss »
            // — ce qui déforme précisément l'information qui compte.
            .tooltip(move |_, cx| {
                cx.new(|_| TerminalToolbarTooltip {
                    label: command.clone(),
                })
                .into()
            })
    }
}

fn snapshot_pane_key(id: PaneId) -> TerminalPaneKey {
    match id {
        PaneId::Primary => TerminalPaneKey::Primary,
        PaneId::Extra(id) => TerminalPaneKey::Extra(id),
    }
}

fn restore_pane_key(id: TerminalPaneKey) -> PaneId {
    match id {
        TerminalPaneKey::Primary => PaneId::Primary,
        TerminalPaneKey::Extra(id) => PaneId::Extra(id),
    }
}

fn snapshot_pane_node(node: &PaneNode) -> TerminalPaneNodeSnapshot {
    match node {
        PaneNode::Leaf(id) => TerminalPaneNodeSnapshot::Leaf(snapshot_pane_key(*id)),
        PaneNode::Split {
            direction,
            ratio,
            a,
            b,
        } => TerminalPaneNodeSnapshot::Split {
            horizontal: matches!(direction, SplitDirection::Horizontal),
            ratio: *ratio,
            first: Box::new(snapshot_pane_node(a)),
            second: Box::new(snapshot_pane_node(b)),
        },
    }
}

fn apply_pane_node_snapshot(node: &mut PaneNode, snapshot: &TerminalPaneNodeSnapshot) {
    match (node, snapshot) {
        (PaneNode::Leaf(current), TerminalPaneNodeSnapshot::Leaf(saved))
            if snapshot_pane_key(*current) == *saved => {}
        (
            PaneNode::Split {
                direction,
                ratio,
                a,
                b,
            },
            TerminalPaneNodeSnapshot::Split {
                horizontal,
                ratio: saved_ratio,
                first,
                second,
            },
        ) if matches!(direction, SplitDirection::Horizontal) == *horizontal => {
            *ratio = saved_ratio.clamp(0.1, 0.9);
            apply_pane_node_snapshot(a, first);
            apply_pane_node_snapshot(b, second);
        }
        _ => {}
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Rendered means on screen: with the window active, the tab in
        // front has been looked at.
        if window.is_window_active() {
            self.mark_active_tab_seen(cx);
        }
        // Fixed for the frame before any geometry is computed from it.
        self.session_list_width = if self.session_list_shown() {
            SESSION_LIST_WIDTH * Self::ui_scale(window)
        } else {
            0.0
        };
        let mut container = div()
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(ShellDeckColors::bg_sidebar());

        if self.pane.sessions.is_empty() {
            container = container.child(self.render_empty_state(window, cx));
            let measure = cx.entity().downgrade();
            container = container.child(
                canvas(
                    move |bounds, _, cx| {
                        let width = bounds.size.width.to_f64() as f32;
                        cx.defer(move |cx| {
                            let _ = measure.update(cx, |this, cx| {
                                if (this.empty_state_width - width).abs() > 1.0 {
                                    this.empty_state_width = width;
                                    cx.notify();
                                }
                            });
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        } else {
            if self.needs_focus {
                self.needs_focus = false;
                self.focus_handle.focus(window);
            }

            // --- Focus change detection & blink timer management ---
            let focused_now = self.focus_handle.is_focused(window);
            if focused_now != self.has_focus {
                self.has_focus = focused_now;
                if focused_now {
                    // Gained focus: start blink timer if grid wants blinking
                    let grid_blink = self
                        .active_session()
                        .map(|s| s.grid.lock().cursor.blink)
                        .unwrap_or(false);
                    if grid_blink {
                        self.start_cursor_blink(cx);
                    } else {
                        self.stop_cursor_blink();
                    }
                    // Send focus-in event if app requested it (mode 1004)
                    if let Some(session) = self.active_session() {
                        if session.grid.lock().focus_reporting() {
                            session.write_input(b"\x1b[I");
                        }
                    }
                } else {
                    // Lost focus: stop blinking, show steady cursor
                    self.stop_cursor_blink();
                    // Send focus-out event if app requested it (mode 1004)
                    if let Some(session) = self.active_session() {
                        if session.grid.lock().focus_reporting() {
                            session.write_input(b"\x1b[O");
                        }
                    }
                }
            }

            self.resize_if_needed(window);

            // The session list is the tab strip in another shape: never both.
            if !self.session_list_shown() {
                container = container.child(self.render_tab_bar(window, cx));
            }

            // Ensure glyph cache is ready
            self.ensure_glyph_cache(window);
            let cache = self
                .glyph_cache
                .as_ref()
                .expect("ensure_glyph_cache called above")
                .clone();

            // ---- Pane layout rendering (recursive split tree) ----
            // Cards are placed inside `outer`, the layer's own rectangle.
            let outer = self.content_area(window);
            let area = self.panes_area(window);
            let (leaves, dividers) = self.layout.compute(area, SPLIT_DIVIDER_SIZE);

            // The focused pane drives the interactive coordinate offsets.
            let focused_rect = leaves
                .iter()
                .find(|(id, _)| *id == self.layout.focused)
                .map(|(_, r)| *r)
                .unwrap_or(area);
            let header_height = self.pane_header_height(window);
            let focused_grid = layout::grid_rect(focused_rect, header_height);
            self.grid_x_offset = focused_grid.x;
            self.grid_y_offset = focused_grid.y;

            // Clear dirty flags on every visible pane this frame, noting whether
            // the focused pane changed (so URL detection can be skipped on
            // unchanged repaints like cursor-blink frames).
            let mut focused_dirty = false;
            let mut focused_session: Option<Uuid> = None;
            for (id, _) in &leaves {
                if let Some(s) = self.session_for(*id) {
                    let mut g = s.grid.lock();
                    if *id == self.layout.focused {
                        focused_dirty = g.dirty;
                        focused_session = Some(s.id);
                    }
                    g.dirty = false;
                }
            }

            // A maximized pane is alone on screen and still part of a split.
            let split = self.layout.is_split();
            let mut pane_layer = div()
                .id("pane-layer")
                .relative()
                .flex_grow()
                .size_full()
                .bg(Self::canvas_tint())
                .overflow_hidden();

            for (id, rect) in &leaves {
                let id = *id;
                let rect = *rect;
                let grid_arc = match self.session_for(id) {
                    Some(s) => s.grid.clone(),
                    None => continue,
                };
                let local_x = rect.x - outer.x;
                let local_y = rect.y - outer.y;
                let is_focused = id == self.layout.focused;

                // A pane is a card on the pane area. The focused card of a
                // split is told apart by its border; the grid paints the
                // same background, inset from the rounded corners.
                let mut pane = div()
                    .absolute()
                    .left(px(local_x))
                    .top(px(local_y))
                    .w(px(rect.w))
                    .h(px(rect.h))
                    .flex()
                    .flex_col()
                    .rounded(px(layout::PANE_RADIUS))
                    .bg(self.palette.background_color())
                    .border_1()
                    .border_color(if split && is_focused {
                        ShellDeckColors::primary().opacity(0.55)
                    } else {
                        ShellDeckColors::border()
                    })
                    .shadow_sm()
                    .overflow_hidden();

                // A header only when split: the tab already names the only
                // pane. The grid below it is sized from `layout::grid_rect`
                // with the same height and the same padding as `wrapper`.
                if split {
                    pane = pane.child(self.render_pane_header(id, is_focused, window, cx));
                }
                // The grid element pads itself by 4px on every side.
                let grid_own_padding = 4.0;
                let mut wrapper = div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.0))
                    .w_full()
                    .px(px(layout::PANE_INSET_X - grid_own_padding))
                    .pt(px(layout::pane_inset_top(header_height) - grid_own_padding))
                    .pb(px(layout::PANE_INSET_BOTTOM - grid_own_padding))
                    .overflow_hidden();

                if is_focused {
                    // Recompute URL underlines only when the focused content
                    // actually changed (dirty) or the focused session switched;
                    // otherwise reuse the cached set so blink/idle repaints don't
                    // re-run the regex over every visible row.
                    let urls_stale = focused_dirty || self.last_url_session != focused_session;
                    let (mouse_mode, mouse_encoding, mut cursor) = {
                        let g = grid_arc.lock();
                        if urls_stale {
                            self.detected_urls = detect_grid_urls(&g);
                            self.last_url_session = focused_session;
                        }
                        (g.mouse_mode, g.mouse_encoding, g.cursor.clone())
                    };
                    cursor.shape = self.effective_cursor_shape(cursor.shape);
                    wrapper = wrapper.child(self.render_terminal_grid(
                        mouse_mode,
                        mouse_encoding,
                        cursor,
                        cache.clone(),
                        grid_arc,
                        cx,
                    ));
                    if self.search_visible {
                        wrapper = wrapper.child(self.render_search_bar(cx));
                    }
                    if let Some(state) = self.context_menu.clone() {
                        // The menu is placed inside `wrapper`, from a
                        // pointer position given in window space.
                        let wrapper_origin = match self.pane_layer_origin {
                            Some((x, y)) => (
                                x + local_x + layout::PANE_BORDER,
                                y + local_y + layout::PANE_BORDER + header_height,
                            ),
                            None => (
                                self.grid_x_offset - layout::PANE_INSET_X,
                                self.grid_y_offset - layout::pane_inset_top(header_height),
                            ),
                        };
                        wrapper =
                            wrapper.child(self.render_context_menu(&state, wrapper_origin, cx));
                    }
                } else {
                    wrapper = wrapper.child(self.render_split_passive_grid(
                        id,
                        grid_arc,
                        cache.clone(),
                        cx,
                    ));
                }

                pane_layer = pane_layer.child(pane.child(wrapper));
            }

            // Draggable dividers (each carries the path to its split node).
            for d in &dividers {
                let local_x = d.rect.x - outer.x;
                let local_y = d.rect.y - outer.y;
                let is_h = matches!(d.direction, SplitDirection::Horizontal);
                let path = d.path.clone();
                let mut div_el = div()
                    .id(ElementId::from(SharedString::from(format!(
                        "split-divider-{}-{}",
                        local_x as i32, local_y as i32
                    ))))
                    .absolute()
                    .left(px(local_x))
                    .top(px(local_y))
                    .w(px(d.rect.w))
                    .h(px(d.rect.h))
                    .rounded_full()
                    .hover(|el| el.bg(ShellDeckColors::primary().opacity(0.35)));
                div_el = if is_h {
                    div_el.cursor_col_resize()
                } else {
                    div_el.cursor_row_resize()
                };
                if self.split_dragging {
                    div_el = div_el.bg(ShellDeckColors::primary().opacity(0.35));
                }
                div_el = div_el.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _w, cx| {
                        this.split_dragging = true;
                        this.active_divider = Some((path.clone(), is_h));
                        cx.notify();
                    }),
                );
                pane_layer = pane_layer.child(div_el);
            }

            // Global divider drag: let the mouse roam across panes while resizing.
            if self.split_dragging {
                if let Some((path, is_h)) = self.active_divider.clone() {
                    pane_layer = pane_layer
                        .on_mouse_move(cx.listener(
                            move |this, event: &MouseMoveEvent, window, cx| {
                                let area = this.panes_area(window);
                                if let Some(nr) =
                                    this.layout.node_rect(&path, area, SPLIT_DIVIDER_SIZE)
                                {
                                    let ratio = if is_h {
                                        (event.position.x.to_f64() as f32 - nr.x) / nr.w.max(1.0)
                                    } else {
                                        (event.position.y.to_f64() as f32 - nr.y) / nr.h.max(1.0)
                                    };
                                    this.layout.set_ratio_at(&path, ratio);
                                    this.resize_if_needed(window);
                                    cx.notify();
                                }
                            },
                        ))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseUpEvent, _w, cx| {
                                this.split_dragging = false;
                                this.active_divider = None;
                                cx.notify();
                            }),
                        );
                }
            }

            // Record the layer's real size; a change lays the cards out
            // again on the next frame.
            let measure = cx.entity().downgrade();
            pane_layer = pane_layer.child(
                canvas(
                    move |bounds, _, cx| {
                        let size = (
                            (bounds.size.width.to_f64() as f32).max(1.0),
                            (bounds.size.height.to_f64() as f32).max(1.0),
                        );
                        let origin = (
                            bounds.origin.x.to_f64() as f32,
                            bounds.origin.y.to_f64() as f32,
                        );
                        cx.defer(move |cx| {
                            let _ = measure.update(cx, |this, cx| {
                                this.pane_layer_origin = Some(origin);
                                let changed = this.pane_layer_size.is_none_or(|(w, h)| {
                                    (w - size.0).abs() > 0.5 || (h - size.1).abs() > 0.5
                                });
                                if changed {
                                    this.pane_layer_size = Some(size);
                                    cx.notify();
                                }
                            });
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );

            container = container.child(pane_layer);

            // The actions float over the top of the pane area, under their
            // button: no row is taken from the panes and no grid is resized.
            if self.actions_open {
                let top = px(self.chrome_top_offset(window) + layout::PANE_MARGIN / 2.0);
                let mut card = div()
                    .id("terminal-actions")
                    .absolute()
                    .top(top)
                    .rounded(px(layout::PANE_RADIUS))
                    .bg(ShellDeckColors::bg_surface())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .shadow_lg()
                    .occlude()
                    .child(self.render_toolbar(window, cx));
                card = if self.session_list_shown() {
                    card.left(px(layout::PANE_MARGIN))
                } else {
                    card.right(px(layout::PANE_MARGIN))
                };
                container = container.child(card);
            }

            // Tab context menu overlays the whole terminal view.
            if let Some(state) = self.tab_context_menu.clone() {
                container = container.child(self.render_tab_context_menu(&state, window, cx));
            }
        }

        if !self.session_list_shown() {
            return container;
        }
        div()
            .flex()
            .flex_row()
            .size_full()
            .child(self.render_session_list(window, cx))
            .child(container.flex_1().min_w(px(0.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        validate_ai_command, AuthorizedLocalRoot, LocalCwdScope, PaneId, PaneNode, TerminalView,
    };
    use gpui::{AppContext, TestAppContext};

    // SDTEST-3092 / SDUC-575
    #[test]
    fn sdtest_3092_full_width_terminal_starts_at_the_window_edge() {
        // Beside the sidebar: right of the rail, the panel and the handle.
        assert_eq!(
            super::content_left(308.0, false, 0.0),
            308.0 + super::SIDEBAR_HANDLE_WIDTH
        );
        // Focused Terminal view: no sidebar is drawn. Reserving its width
        // would shift every pane, tab menu and column count by the rail.
        assert_eq!(super::content_left(308.0, true, 0.0), 0.0);
    }

    // SDTEST-3110 / SDUC-579. The session list takes its width from the
    // content and the tab strip's row from the chrome: the panes are sized
    // for what is drawn, in the full app and in the focused Terminal view.
    #[test]
    fn sdtest_3110_the_session_list_moves_the_content_and_replaces_the_tab_strip() {
        assert_eq!(super::content_left(48.0, true, 232.0), 232.0);
        assert_eq!(
            super::content_left(48.0, false, 232.0),
            48.0 + super::SIDEBAR_HANDLE_WIDTH + 232.0
        );
        // No toolbar row: the tab strip is the only row above the panes,
        // and the session list leaves none.
        assert_eq!(super::chrome_rows_height(false), super::TAB_BAR_HEIGHT);
        assert_eq!(super::chrome_rows_height(true), 0.0);
    }

    // SDTEST-1333 / SDTEST-1334 retired 2026-08-06: command_available now
    // delegates to shelldeck_core::util::executable_on_path, whose contracts
    // (multi-dir PATH walk, PATHEXT extensions, unix +x check) are pinned by
    // SDTEST-1591 in shelldeck-core.

    #[test]
    fn ai_command_accepts_exactly_one_non_empty_line() {
        assert_eq!(validate_ai_command("  ls -la  "), Ok("ls -la"));
        assert_eq!(validate_ai_command(""), Err("empty"));
        assert_eq!(
            validate_ai_command("echo first\necho second"),
            Err("multiline")
        );
    }

    #[test]
    fn native_workspace_snapshot_reapplies_split_focus_and_ratio() {
        let mut app = TestAppContext::single();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let captured = terminal.update(&mut app, |terminal, cx| {
            terminal.spawn_local_terminal(cx);
            terminal.split_horizontal(cx);
            terminal.toggle_split_focus();
            let PaneNode::Split { ratio, .. } = &mut terminal.layout.tree else {
                panic!("local split must create a native split tree");
            };
            *ratio = 0.63;
            terminal.capture_workspace_snapshot()
        });
        terminal.update(&mut app, |terminal, _| {
            terminal.toggle_split_focus();
            let PaneNode::Split { ratio, .. } = &mut terminal.layout.tree else {
                panic!("split must remain live");
            };
            *ratio = 0.25;
            terminal.apply_workspace_snapshot(&captured);
            assert_eq!(terminal.capture_workspace_snapshot(), captured);
            terminal.close_all_sessions();
        });
    }

    // SDTEST-3037 (SDUC-574): closing the last tab stops the refresh loop;
    // the next terminal must start a new one. Found in the running app:
    // without it a terminal opened after the last tab was closed repainted
    // only on input events, so a session daemon pane looked frozen.
    #[test]
    fn sdtest_3037_the_refresh_loop_restarts_after_the_last_tab_was_closed() {
        let mut app = TestAppContext::single();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        terminal.update(&mut app, |terminal, cx| {
            terminal.spawn_local_terminal(cx);
            assert!(terminal._refresh_task.is_some());
            let first = terminal.tabs[0].id;
            terminal.close_tab(first);
            assert_eq!(terminal.tab_count(), 0);
            assert!(
                terminal._refresh_task.is_none(),
                "no loop without a terminal"
            );

            terminal.spawn_local_terminal(cx);
            assert_eq!(terminal.tab_count(), 1);
            assert!(
                terminal._refresh_task.is_some(),
                "a terminal opened after the last tab was closed needs a refresh loop"
            );
            // Sessions are wired to a channel the new loop listens on.
            assert!(!terminal.output_tx.is_closed());
            terminal.close_all_sessions();
        });
    }

    // SDTEST-3149 (SDUC-584): the view finds the project of each tab off
    // the render path, never looks on this disk for the folder of a
    // connection tab, and next/previous project selects the tab used last
    // in the project arrived at.
    #[test]
    fn sdtest_3149_tabs_get_a_project_and_the_view_moves_between_projects() {
        use super::projects::{project_groups, ProjectKey};
        use shelldeck_terminal::session::TerminalSession;

        let disk = tempfile::tempdir().unwrap();
        let base = disk.path().canonicalize().unwrap();
        let repo = base.join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir_all(base.join("plain")).unwrap();
        let text = |path: std::path::PathBuf| path.to_str().unwrap().to_string();
        let (repo_text, src, plain) = (
            text(repo.clone()),
            text(repo.join("src")),
            text(base.join("plain")),
        );

        fn tab_in(folder: &str) -> TerminalSession {
            let (session, _output, _input) =
                TerminalSession::spawn_ssh("Terminal".into(), 24, 80).unwrap();
            session.grid.lock().working_directory = Some(folder.to_string());
            session
        }

        let mut app = TestAppContext::single();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        let connection = uuid::Uuid::from_u128(7);
        let ids = terminal.update(&mut app, |terminal, cx| {
            // Not the home of whoever runs the tests.
            terminal.home_dir = None;
            terminal.add_session(tab_in(&src));
            terminal.add_session(tab_in(&repo_text));
            terminal.add_session(tab_in(&plain));
            // A server that happens to have the same path.
            terminal.add_session_with_connection(tab_in(&src), Some(connection));
            assert!(terminal.follow_tab_folders(cx));
            assert!(!terminal.follow_tab_folders(cx), "nothing moved since");
            // Nothing is read from the disk on this thread: until the
            // lookups end, the local tabs have no project.
            assert!(terminal.tab_places()[..3]
                .iter()
                .all(|place| place.root.is_none()));
            terminal.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>()
        });
        app.run_until_parked();

        terminal.update(&mut app, |terminal, _| {
            let groups = project_groups(&terminal.tab_places());
            let summary: Vec<(Option<&str>, bool, &[usize])> = groups
                .iter()
                .map(|group| match &group.key {
                    ProjectKey::Folder {
                        connection_id,
                        root,
                    } => (
                        Some(root.as_str()),
                        connection_id.is_some(),
                        group.tabs.as_slice(),
                    ),
                    ProjectKey::None => (None, false, group.tabs.as_slice()),
                })
                .collect();
            assert_eq!(
                summary,
                vec![
                    (Some(repo_text.as_str()), false, &[0, 1][..]),
                    (Some(plain.as_str()), false, &[2][..]),
                    // The repository above it was not looked for here.
                    (Some(src.as_str()), true, &[3][..]),
                ]
            );

            // Use the second tab of the repository, then the plain folder.
            terminal.select_tab(ids[1]);
            terminal.select_tab(ids[2]);
            assert_eq!(terminal.select_adjacent_project(true), Some(ids[3]));
            // Round the end, onto the tab of the repository used last.
            terminal.collapsed_projects.insert(groups[0].key.clone());
            assert_eq!(terminal.select_adjacent_project(true), Some(ids[1]));
            assert!(terminal.tabs[1].is_active);
            assert!(
                terminal.collapsed_projects.is_empty(),
                "the project arrived at is unfolded"
            );
            assert_eq!(terminal.select_adjacent_project(false), Some(ids[3]));

            // A closed tab is no longer the one a project comes back to.
            terminal.close_tab(ids[1]);
            assert_eq!(terminal.select_adjacent_project(true), Some(ids[0]));
            // One project left: nowhere to go.
            terminal.close_tab(ids[2]);
            terminal.close_tab(ids[3]);
            assert_eq!(terminal.select_adjacent_project(true), None);
            terminal.close_all_sessions();
        });
    }

    // SDTEST-3022 (SDUC-574): an SSH tab whose host turns out to run the
    // session daemon is handed over to a daemon pane in place: same tab,
    // same position, the old session dropped and never terminated. Such a
    // tab is not saved as a local daemon layout.
    #[test]
    fn sdtest_3022_an_ssh_tab_is_handed_over_to_a_daemon_pane_in_place() {
        use super::SplitDirection;
        use shelldeck_terminal::attached::AttachedPhase;
        use shelldeck_terminal::session::TerminalSession;
        use uuid::Uuid;

        static ENDED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        fn attached(pane: u128) -> TerminalSession {
            let (mut session, feed, _input) =
                TerminalSession::spawn_attached("host".into(), 24, 80, 100, 1).unwrap();
            session.set_terminate_fn(Box::new(|| {
                ENDED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }));
            feed.status().set_pane(Uuid::from_u128(pane));
            feed.status().set_phase(AttachedPhase::Live);
            session
        }
        fn ssh() -> TerminalSession {
            TerminalSession::spawn_ssh("host".into(), 24, 80).unwrap().0
        }

        let mut app = TestAppContext::single();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        terminal.update(&mut app, |terminal, _| {
            terminal.set_persistent_sessions(true);
            let connection = Uuid::from_u128(77);
            let first = ssh();
            let first_id = first.id;
            terminal.add_session_with_connection(first, Some(connection));
            let second = ssh();
            let second_id = second.id;
            terminal.add_session_with_connection(second, Some(connection));
            terminal.last_grid_rows = 24;
            terminal.last_grid_cols = 80;
            assert!(terminal.has_session(first_id) && terminal.has_session(second_id));

            // The background tab is handed over: its place, id and
            // connection stay, the session behind it is the daemon pane.
            assert!(terminal.replace_session(first_id, attached(1)));
            assert_eq!(terminal.tabs.len(), 2);
            assert_eq!(terminal.tabs[0].id, first_id);
            assert_eq!(terminal.tabs[0].connection_id, Some(connection));
            assert_eq!(
                terminal.pane.sessions[0].daemon_pane(),
                Some(Uuid::from_u128(1))
            );
            // The tab and its session keep one identity: everything that
            // finds a tab's session by the tab id still does.
            assert_eq!(terminal.pane.sessions[0].id, terminal.tabs[0].id);
            assert!(terminal.has_session(first_id));
            assert_eq!(
                terminal
                    .session_by_id(first_id)
                    .and_then(|session| session.daemon_pane()),
                Some(Uuid::from_u128(1))
            );
            assert_eq!(terminal.pane.active_index, 1, "the active tab did not move");
            assert_eq!(terminal.last_grid_rows, 0, "the new session gets sized");
            assert_eq!(ENDED.load(std::sync::atomic::Ordering::Relaxed), 0);
            // Selecting the handed-over tab by its id works.
            terminal.select_tab(first_id);
            assert_eq!(terminal.pane.active_index, 0);
            assert!(terminal.tabs[0].is_active);
            terminal.select_tab(second_id);

            // A session that is no longer shown (tab closed while it was
            // connecting) is not replaced.
            assert!(!terminal.replace_session(Uuid::from_u128(404), attached(2)));

            // A split pane can be handed over as well, under its key.
            let split = ssh();
            let split_id = split.id;
            terminal.install_split_pane(split, SplitDirection::Horizontal);
            assert!(terminal.replace_session(split_id, attached(3)));
            assert!(terminal
                .layout
                .extra
                .values()
                .any(|session| session.daemon_pane() == Some(Uuid::from_u128(3))
                    && session.id == split_id));
            assert!(terminal.has_session(split_id));

            // Tabs of an SSH connection are never saved as panes of the
            // local daemon: their panes live on the host.
            assert_eq!(terminal.daemon_tab_layouts(), vec![None, None]);
            assert_eq!(ENDED.load(std::sync::atomic::Ordering::Relaxed), 0);
        });
    }

    // SDTEST-3069 (SDUC-574): SSH tabs made of panes of their host's
    // daemon save their split tree by pane id (local tabs never appear
    // there), and a tab handed over to such a pane gets its saved splits
    // back, whether it is the active tab or not.
    #[test]
    fn sdtest_3069_ssh_daemon_tabs_save_and_get_back_their_splits() {
        use shelldeck_core::config::workspace_state::{DaemonPaneNode, DaemonTabLayout};
        use shelldeck_terminal::attached::AttachedPhase;
        use shelldeck_terminal::session::TerminalSession;
        use uuid::Uuid;

        fn attached(pane: u128) -> TerminalSession {
            let (session, feed, _input) =
                TerminalSession::spawn_attached("host".into(), 24, 80, 100, 1).unwrap();
            feed.status().set_pane(Uuid::from_u128(pane));
            feed.status().set_phase(AttachedPhase::Live);
            session
        }
        fn ssh() -> TerminalSession {
            TerminalSession::spawn_ssh("host".into(), 24, 80).unwrap().0
        }
        let leaf = |pane: u128| DaemonPaneNode::Pane {
            pane: Uuid::from_u128(pane),
        };
        let saved = DaemonTabLayout {
            tree: DaemonPaneNode::Split {
                horizontal: false,
                ratio: 0.4,
                first: Box::new(leaf(1)),
                second: Box::new(leaf(2)),
            },
            focused: Some(Uuid::from_u128(2)),
            named: true,
        };

        let mut app = TestAppContext::single();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        terminal.update(&mut app, |terminal, _| {
            terminal.set_persistent_sessions(true);
            let connection = Uuid::from_u128(77);
            // Tab 1: SSH, still connecting (a plain SSH session).
            let first = ssh();
            let first_id = first.id;
            terminal.add_session_with_connection(first, Some(connection));
            // Tab 2: SSH, same host, the active one.
            let second = ssh();
            let second_id = second.id;
            terminal.add_session_with_connection(second, Some(connection));
            // Tab 3: a local daemon pane.
            terminal.add_session(attached(9));
            terminal.select_tab(second_id);
            assert_eq!(terminal.remote_tab_layouts(), vec![None, None, None]);

            // The host answers for the inactive tab: it is handed over and
            // gets its splits back in its stored layout.
            assert!(terminal.replace_session(first_id, attached(1)));
            assert!(terminal.install_daemon_splits(
                first_id,
                Uuid::from_u128(1),
                vec![(Uuid::from_u128(2), attached(2))],
                &saved,
            ));
            // And for the active tab, with one pane of its tree left.
            assert!(terminal.replace_session(second_id, attached(3)));
            assert_eq!(terminal.tabs[terminal.pane.active_index].id, second_id);
            assert!(terminal.layout.extra.is_empty());

            assert_eq!(
                terminal.remote_tab_layouts(),
                vec![
                    Some(saved.clone()),
                    Some(DaemonTabLayout {
                        tree: leaf(3),
                        focused: Some(Uuid::from_u128(3)),
                        named: false,
                    }),
                    None,
                ]
            );
            assert_eq!(
                terminal.daemon_tab_layouts()[..2],
                [None, None],
                "panes of a host are never saved as local daemon panes"
            );
            assert!(terminal.tabs[0].named);

            // The restored tab shows both panes, and gets its focus back.
            terminal.activate_restored_tab(first_id, saved.focused);
            assert_eq!(terminal.layout.extra.len(), 1);
            assert_eq!(
                terminal.active_session().and_then(|s| s.daemon_pane()),
                Some(Uuid::from_u128(2))
            );
            // No such tab: nothing happens.
            assert!(!terminal.install_daemon_splits(
                Uuid::from_u128(404),
                Uuid::from_u128(1),
                Vec::new(),
                &saved,
            ));
        });
    }

    // SDTEST-3013 (SDUC-574): the layout saved for session daemon tabs
    // names daemon panes, tab by tab, and leaves out what cannot come
    // back; a restored tab gets its focused pane back when activated. The
    // sessions are attached sessions without a daemon behind them.
    #[test]
    fn sdtest_3013_daemon_tabs_are_saved_by_pane_id_and_refocused() {
        use super::SplitDirection;
        use shelldeck_core::config::workspace_state::{DaemonPaneNode, DaemonTabLayout};
        use shelldeck_terminal::attached::AttachedPhase;
        use shelldeck_terminal::session::TerminalSession;
        use uuid::Uuid;

        // An attached session as the supervisor hands it out. `pane`: the
        // daemon pane, when it is known already.
        static ENDED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        fn attached(pane: Option<u128>, phase: AttachedPhase) -> TerminalSession {
            let (mut session, feed, _input) =
                TerminalSession::spawn_attached("Terminal".into(), 24, 80, 100, 1).unwrap();
            session.set_terminate_fn(Box::new(|| {
                ENDED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }));
            if let Some(pane) = pane {
                feed.status().set_pane(Uuid::from_u128(pane));
            }
            feed.status().set_phase(phase);
            session
        }
        let leaf = |pane: u128| DaemonPaneNode::Pane {
            pane: Uuid::from_u128(pane),
        };

        let mut app = TestAppContext::single();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        terminal.update(&mut app, |terminal, _| {
            terminal.set_persistent_sessions(true);
            assert!(terminal.restores_daemon_panes());

            // Tab A: pane 1 | (pane 2 / an ended pane), focus on pane 2.
            terminal.add_session(attached(Some(1), AttachedPhase::Live));
            let tab_a = terminal.tabs[0].id;
            terminal.install_split_pane(
                attached(Some(2), AttachedPhase::Reconnecting),
                SplitDirection::Horizontal,
            );
            let pane_two = terminal.layout.focused;
            terminal.install_split_pane(
                attached(Some(3), AttachedPhase::Ended { exit_code: Some(0) }),
                SplitDirection::Vertical,
            );
            terminal.focus_pane(pane_two);
            let PaneNode::Split { ratio, .. } = &mut terminal.layout.tree else {
                panic!("split tree");
            };
            *ratio = 0.3;
            // Tab B: still connecting, its pane does not exist yet.
            terminal.add_session(attached(None, AttachedPhase::Connecting));
            // Tab C: one pane.
            terminal.add_session(attached(Some(5), AttachedPhase::Live));
            // Tab D: its pane runs under a daemon of another version of
            // the app. This app cannot show it, and keeps its place.
            terminal.add_session(attached(Some(6), AttachedPhase::OtherVersion));

            let saved = terminal.daemon_tab_layouts();
            assert_eq!(
                saved,
                vec![
                    Some(DaemonTabLayout {
                        tree: DaemonPaneNode::Split {
                            horizontal: true,
                            ratio: 0.3,
                            first: Box::new(leaf(1)),
                            second: Box::new(leaf(2)),
                        },
                        focused: Some(Uuid::from_u128(2)),
                        named: false,
                    }),
                    None,
                    Some(DaemonTabLayout {
                        tree: leaf(5),
                        focused: Some(Uuid::from_u128(5)),
                        named: false,
                    }),
                    Some(DaemonTabLayout {
                        tree: leaf(6),
                        focused: Some(Uuid::from_u128(6)),
                        named: false,
                    }),
                ]
            );

            // Selecting a tab focuses its primary pane; a restored tab
            // gets the saved pane back.
            terminal.activate_restored_tab(tab_a, Some(Uuid::from_u128(2)));
            assert_eq!(terminal.tabs[terminal.pane.active_index].id, tab_a);
            assert_eq!(terminal.layout.focused, pane_two);
            assert_eq!(
                terminal.active_session().and_then(|s| s.daemon_pane()),
                Some(Uuid::from_u128(2))
            );
            // A pane that is not in the tab leaves the focus alone.
            terminal.activate_restored_tab(tab_a, Some(Uuid::from_u128(77)));
            assert_eq!(terminal.layout.focused, pane_two);

            // Sign-out ends every pane, in every tab and split; a plain
            // close (app quit) ends none.
            terminal.end_daemon_panes();
            assert_eq!(ENDED.load(std::sync::atomic::Ordering::Relaxed), 6);

            // A workspace-scoped view never takes daemon panes.
            terminal.default_cwd = LocalCwdScope::Required {
                intended: "/nonexistent".into(),
                authority: None,
            };
            assert!(!terminal.restores_daemon_panes());
            terminal.close_all_sessions();
            assert_eq!(ENDED.load(std::sync::atomic::Ordering::Relaxed), 6);
        });
    }

    #[test]
    fn canonical_workspace_cwd_reaches_tabs_splits_and_cli_created_terminals() {
        let root = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(root.path()).unwrap();
        let mut app = TestAppContext::single();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        terminal.update(&mut app, |terminal, cx| {
            terminal.set_default_cwd(&canonical).unwrap();
            terminal.spawn_local_terminal(cx);
            terminal.spawn_local_terminal(cx);
            assert!(terminal
                .pane
                .sessions
                .iter()
                .all(|session| session.initial_cwd() == Some(canonical.as_path())));
            terminal.split_horizontal(cx);
            assert!(terminal
                .layout
                .extra
                .values()
                .all(|session| session.initial_cwd() == Some(canonical.as_path())));
            terminal.close_all_sessions();
            terminal.launch_cli("true", "test", cx);
            assert_eq!(
                terminal
                    .active_session()
                    .and_then(|session| session.initial_cwd()),
                Some(canonical.as_path())
            );
            terminal.close_all_sessions();
        });
    }

    // SDTEST-1736
    #[test]
    fn missing_required_workspace_cwd_blocks_open_and_cli_without_home_fallback() {
        let root = tempfile::tempdir().unwrap();
        let intended = root.path().to_path_buf();
        root.close().unwrap();
        let mut app = TestAppContext::single();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        terminal.update(&mut app, |terminal, cx| {
            assert!(terminal.set_default_cwd(&intended).is_err());
            let message = terminal.required_cwd_unavailable_message().unwrap();
            assert!(message.contains(&intended.display().to_string()));
            assert!(
                message.contains("Aucun terminal local") || message.contains("No local terminal")
            );
            terminal.spawn_local_terminal(cx);
            terminal.launch_cli("true", "test", cx);
            assert_eq!(terminal.tab_count(), 0);
            assert!(terminal.active_session().is_none());
        });
    }

    // SDTEST-1736
    #[test]
    fn disappearing_required_workspace_cwd_blocks_split_of_a_live_local_session() {
        let initial_root = tempfile::tempdir().unwrap();
        let split_root = tempfile::tempdir().unwrap();
        let disappearing = std::fs::canonicalize(split_root.path()).unwrap();
        let mut app = TestAppContext::single();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        terminal.update(&mut app, |terminal, cx| {
            terminal.set_default_cwd(initial_root.path()).unwrap();
            terminal.spawn_local_terminal(cx);
            assert_eq!(terminal.tab_count(), 1, "fixture must own a live local tab");

            terminal.set_default_cwd(&disappearing).unwrap();
            split_root.close().unwrap();
            assert!(!disappearing.exists(), "authorized split cwd must be gone");

            terminal.split_horizontal(cx);
            assert_eq!(terminal.tab_count(), 1);
            assert!(
                terminal.layout.extra.is_empty(),
                "failed split must not retain a secondary PTY"
            );
            assert!(
                matches!(terminal.layout.tree, PaneNode::Leaf(PaneId::Primary)),
                "failed split must not mutate the pane tree"
            );
            terminal.close_all_sessions();
        });
    }

    // SDTEST-1769 — SDUC-491
    #[test]
    fn prepared_workspace_pty_stays_detached_until_explicit_publication() {
        let root = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(root.path()).unwrap();
        let authority = AuthorizedLocalRoot::capture(&canonical).unwrap();
        let mut app = TestAppContext::single();
        let terminal = app.update(|cx| cx.new(TerminalView::new));
        terminal.update(&mut app, |terminal, cx| {
            let abandoned = terminal
                .prepare_authorized_local_terminal(&authority)
                .unwrap();
            assert_eq!(terminal.tab_count(), 0);
            assert!(terminal.active_session().is_none());
            assert!(matches!(terminal.default_cwd, LocalCwdScope::Unscoped));
            drop(abandoned);
            assert_eq!(terminal.tab_count(), 0);
            assert!(matches!(terminal.default_cwd, LocalCwdScope::Unscoped));

            let prepared = terminal
                .prepare_authorized_local_terminal(&authority)
                .unwrap();
            terminal.commit_prepared_local_terminal(prepared, cx);
            assert_eq!(terminal.tab_count(), 1);
            assert!(matches!(
                &terminal.default_cwd,
                LocalCwdScope::Required { intended, .. } if intended == &canonical
            ));
            terminal.close_all_sessions();
        });
    }
}
/// Capability for one exact local directory. The open file identity prevents
/// a path from being swapped between authorization and PTY creation.
#[derive(Clone, Debug)]
pub(crate) struct AuthorizedLocalRoot {
    canonical: std::path::PathBuf,
    #[cfg(unix)]
    directory: Arc<std::os::fd::OwnedFd>,
    #[cfg(unix)]
    identity: (u64, u64),
    #[cfg(windows)]
    directory_chain: Arc<Vec<same_file::Handle>>,
    #[cfg(not(any(unix, windows)))]
    identity: Arc<same_file::Handle>,
}

impl AuthorizedLocalRoot {
    pub(crate) fn capture(path: &std::path::Path) -> Result<Self, String> {
        let canonical = std::fs::canonicalize(path).map_err(|error| error.to_string())?;
        if !path.is_absolute() || canonical != path || !direct_directory_chain(path)? {
            return Err("authorized local root changed or contains an alias".into());
        }
        #[cfg(unix)]
        {
            let directory = open_direct_directory(path)?;
            let stat = rustix::fs::fstat(&directory).map_err(|error| error.to_string())?;
            let identity = (stat.st_dev as u64, stat.st_ino as u64);
            let authority = Self {
                canonical,
                directory: Arc::new(directory),
                identity,
            };
            authority.revalidate()?;
            Ok(authority)
        }
        #[cfg(not(unix))]
        #[cfg(windows)]
        {
            let directory_chain = open_windows_directory_chain(path)?;
            let authority = Self {
                canonical,
                directory_chain: Arc::new(directory_chain),
            };
            authority.revalidate()?;
            return Ok(authority);
        }
        #[cfg(not(any(unix, windows)))]
        let identity =
            same_file::Handle::from_path(&canonical).map_err(|error| error.to_string())?;
        #[cfg(not(any(unix, windows)))]
        Ok(Self {
            canonical,
            identity: Arc::new(identity),
        })
    }

    pub(crate) fn path(&self) -> &std::path::Path {
        &self.canonical
    }

    #[cfg(unix)]
    pub(crate) fn unix_identity(&self) -> (u64, u64) {
        self.identity
    }

    /// Stable command-line spelling for the already-open directory. Linux
    /// resolves this through the parent process' descriptor, so a concurrent
    /// rename or symlink replacement cannot redirect a spawned Git process.
    pub(crate) fn command_path(&self) -> std::path::PathBuf {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd as _;
            std::path::PathBuf::from(format!(
                "/proc/{}/fd/{}",
                std::process::id(),
                self.directory.as_ref().as_raw_fd()
            ))
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::fd::AsRawFd as _;
            return std::path::PathBuf::from(format!(
                "/dev/fd/{}",
                self.directory.as_ref().as_raw_fd()
            ));
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        self.canonical.clone()
    }

    /// Stable path for an argument interpreted after `exec` (unlike
    /// `current_dir`, which is resolved before it). macOS therefore inherits
    /// one short-lived duplicate descriptor; the returned guard keeps the
    /// parent's copy alive only until the child command finishes.
    pub(crate) fn inherited_argument_path(
        &self,
    ) -> Result<(std::path::PathBuf, Option<std::fs::File>), String> {
        #[cfg(target_os = "macos")]
        {
            use rustix::io::{dup, fcntl_setfd, FdFlags};
            use std::os::fd::{AsRawFd as _, OwnedFd};

            let descriptor: OwnedFd =
                dup(self.directory.as_ref()).map_err(|error| error.to_string())?;
            fcntl_setfd(&descriptor, FdFlags::empty()).map_err(|error| error.to_string())?;
            let path = std::path::PathBuf::from(format!("/dev/fd/{}", descriptor.as_raw_fd()));
            return Ok((path, Some(std::fs::File::from(descriptor))));
        }
        #[cfg(not(target_os = "macos"))]
        Ok((self.command_path(), None))
    }

    pub(crate) fn revalidate(&self) -> Result<(), String> {
        if !direct_directory_chain(&self.canonical)? {
            return Err("authorized local root is no longer a direct directory".into());
        }
        let canonical =
            std::fs::canonicalize(&self.canonical).map_err(|error| error.to_string())?;
        if canonical != self.canonical {
            return Err("authorized local root identity changed".into());
        }
        #[cfg(unix)]
        {
            let current = open_direct_directory(&self.canonical)?;
            let stat = rustix::fs::fstat(&current).map_err(|error| error.to_string())?;
            let identity = (stat.st_dev as u64, stat.st_ino as u64);
            if identity != self.identity {
                return Err("authorized local root identity changed".into());
            }
        }
        #[cfg(windows)]
        {
            let current = open_windows_directory_chain(&self.canonical)?;
            if current.last() != self.directory_chain.last() {
                return Err("authorized local root identity changed".into());
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let identity =
                same_file::Handle::from_path(&canonical).map_err(|error| error.to_string())?;
            if identity != *self.identity {
                return Err("authorized local root identity changed".into());
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
fn open_windows_directory_chain(path: &std::path::Path) -> Result<Vec<same_file::Handle>, String> {
    use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut cursor = std::path::PathBuf::new();
    let mut handles = Vec::new();
    for component in path.components() {
        cursor.push(component.as_os_str());
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&cursor)
            .map_err(|error| error.to_string())?;
        let attributes = file
            .metadata()
            .map_err(|error| error.to_string())?
            .file_attributes();
        if attributes & FILE_ATTRIBUTE_DIRECTORY == 0
            || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err("authorized local root is not a direct directory".into());
        }
        handles.push(same_file::Handle::from_file(file).map_err(|error| error.to_string())?);
    }
    Ok(handles)
}

fn direct_directory_chain(path: &std::path::Path) -> Result<bool, String> {
    if !path.is_absolute() {
        return Ok(false);
    }
    let mut cursor = std::path::PathBuf::new();
    for component in path.components() {
        cursor.push(component.as_os_str());
        let metadata = std::fs::symlink_metadata(&cursor).map_err(|error| error.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() || is_windows_reparse(&metadata)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(unix)]
fn open_direct_directory(path: &std::path::Path) -> Result<std::os::fd::OwnedFd, String> {
    use rustix::fs::{open, openat, Mode, OFlags};
    use std::path::Component;

    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = open("/", flags, Mode::empty()).map_err(|error| error.to_string())?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                directory = openat(&directory, name, flags, Mode::empty())
                    .map_err(|error| error.to_string())?;
            }
            _ => return Err("authorized local root contains an invalid component".into()),
        }
    }
    Ok(directory)
}

#[cfg(windows)]
fn is_windows_reparse(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_windows_reparse(_metadata: &std::fs::Metadata) -> bool {
    false
}
