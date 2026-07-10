use gpui::*;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use shelldeck_core::config::app_config::ThemePreference;

/// A complete set of resolved UI colors for one theme. Every `ShellDeckColors`
/// accessor reads its value from the currently-active palette, so switching the
/// app theme is a single atomic palette swap.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub is_dark: bool,

    pub primary: Hsla,
    pub primary_hover: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub error: Hsla,

    pub bg_primary: Hsla,
    pub bg_sidebar: Hsla,
    pub bg_surface: Hsla,
    pub border: Hsla,
    pub text_primary: Hsla,
    pub text_muted: Hsla,
    pub terminal_bg: Hsla,

    pub hover_bg: Hsla,
    pub selected_bg: Hsla,
    pub badge_bg: Hsla,
    pub toggle_off_bg: Hsla,
    pub toggle_off_knob: Hsla,
    pub backdrop: Hsla,

    pub syntax_keyword: Hsla,
    pub syntax_builtin: Hsla,
    pub syntax_comment: Hsla,
    pub syntax_string: Hsla,
    pub syntax_variable: Hsla,
    pub syntax_operator: Hsla,
    pub syntax_number: Hsla,
    pub syntax_command_sub: Hsla,
    pub syntax_template_var: Hsla,

    pub line_number_fg: Hsla,
    pub line_number_bg: Hsla,
    pub cursor_line_bg: Hsla,
    pub hint_bg: Hsla,
}

static ACTIVE: Lazy<RwLock<Palette>> = Lazy::new(|| RwLock::new(dark_palette()));

/// Parse a `0xRRGGBB` literal into an `Hsla`.
fn hx(c: u32) -> Hsla {
    rgb(c).into()
}

/// Shift lightness, clamped to `[0, 1]`.
fn adjust_l(c: Hsla, delta: f32) -> Hsla {
    Hsla {
        l: (c.l + delta).clamp(0.0, 1.0),
        ..c
    }
}

/// Seed colors a theme actually cares about; everything else is derived in
/// [`build`]. This keeps each themed palette to a short, readable definition.
struct Seed {
    is_dark: bool,
    bg: Hsla,
    sidebar: Hsla,
    surface: Hsla,
    selected: Hsla,
    border: Hsla,
    text: Hsla,
    muted: Hsla,
    primary: Hsla,
    success: Hsla,
    warning: Hsla,
    error: Hsla,
    keyword: Hsla,
    builtin: Hsla,
    string: Hsla,
    variable: Hsla,
    number: Hsla,
    comment: Hsla,
    terminal_bg: Hsla,
}

/// Derive a full [`Palette`] from a [`Seed`].
fn build(s: Seed) -> Palette {
    let dark = s.is_dark;
    Palette {
        is_dark: dark,
        primary: s.primary,
        primary_hover: adjust_l(s.primary, -0.05),
        success: s.success,
        warning: s.warning,
        error: s.error,
        bg_primary: s.bg,
        bg_sidebar: s.sidebar,
        bg_surface: s.surface,
        border: s.border,
        text_primary: s.text,
        text_muted: s.muted,
        terminal_bg: s.terminal_bg,
        hover_bg: if dark {
            hsla(0.0, 0.0, 1.0, 0.06)
        } else {
            hsla(0.0, 0.0, 0.0, 0.06)
        },
        selected_bg: s.selected,
        badge_bg: s.selected,
        toggle_off_bg: adjust_l(s.border, if dark { 0.06 } else { -0.06 }),
        toggle_off_knob: if dark {
            hsla(0.0, 0.0, 0.55, 1.0)
        } else {
            hsla(0.0, 0.0, 0.98, 1.0)
        },
        backdrop: hsla(0.0, 0.0, 0.0, 0.45),
        syntax_keyword: s.keyword,
        syntax_builtin: s.builtin,
        syntax_comment: s.comment,
        syntax_string: s.string,
        syntax_variable: s.variable,
        syntax_operator: adjust_l(s.muted, if dark { 0.15 } else { -0.10 }),
        syntax_number: s.number,
        syntax_command_sub: s.variable,
        syntax_template_var: s.warning,
        line_number_fg: adjust_l(s.muted, if dark { -0.12 } else { 0.12 }),
        line_number_bg: s.sidebar,
        cursor_line_bg: if dark {
            hsla(0.0, 0.0, 1.0, 0.04)
        } else {
            hsla(0.0, 0.0, 0.0, 0.04)
        },
        hint_bg: if dark {
            hsla(0.0, 0.0, 0.15, 0.5)
        } else {
            hsla(0.0, 0.0, 0.0, 0.06)
        },
    }
}

// ---- Built-in palettes ----

/// Default dark theme — kept as an explicit literal so its exact, hand-tuned
/// look is preserved unchanged across the palette refactor.
fn dark_palette() -> Palette {
    Palette {
        is_dark: true,
        primary: hsla(0.63, 0.85, 0.55, 1.0),
        primary_hover: hsla(0.63, 0.85, 0.50, 1.0),
        success: hsla(0.40, 0.70, 0.45, 1.0),
        warning: hsla(0.10, 0.85, 0.55, 1.0),
        error: hsla(0.0, 0.75, 0.50, 1.0),
        bg_primary: hsla(0.0, 0.0, 0.10, 1.0),
        bg_sidebar: hsla(0.0, 0.0, 0.08, 1.0),
        bg_surface: hsla(0.0, 0.0, 0.13, 1.0),
        border: hsla(0.0, 0.0, 0.20, 1.0),
        text_primary: hsla(0.0, 0.0, 0.90, 1.0),
        text_muted: hsla(0.0, 0.0, 0.55, 1.0),
        terminal_bg: hsla(0.0, 0.0, 0.07, 1.0),
        hover_bg: hsla(0.0, 0.0, 1.0, 0.06),
        selected_bg: hsla(0.0, 0.0, 0.18, 1.0),
        badge_bg: hsla(0.0, 0.0, 0.20, 1.0),
        toggle_off_bg: hsla(0.0, 0.0, 0.25, 1.0),
        toggle_off_knob: hsla(0.0, 0.0, 0.50, 1.0),
        backdrop: hsla(0.0, 0.0, 0.0, 0.45),
        syntax_keyword: hsla(0.76, 0.70, 0.68, 1.0),
        syntax_builtin: hsla(0.50, 0.65, 0.65, 1.0),
        syntax_comment: hsla(0.0, 0.0, 0.45, 1.0),
        syntax_string: hsla(0.35, 0.60, 0.60, 1.0),
        syntax_variable: hsla(0.08, 0.75, 0.65, 1.0),
        syntax_operator: hsla(0.0, 0.0, 0.70, 1.0),
        syntax_number: hsla(0.47, 0.55, 0.60, 1.0),
        syntax_command_sub: hsla(0.12, 0.70, 0.65, 1.0),
        syntax_template_var: hsla(0.08, 0.85, 0.65, 1.0),
        line_number_fg: hsla(0.0, 0.0, 0.38, 1.0),
        line_number_bg: hsla(0.0, 0.0, 0.09, 1.0),
        cursor_line_bg: hsla(0.0, 0.0, 1.0, 0.04),
        hint_bg: hsla(0.0, 0.0, 0.15, 0.5),
    }
}

/// Default light theme — explicit literal, preserves the original look.
fn light_palette() -> Palette {
    Palette {
        is_dark: false,
        primary: hsla(0.63, 0.80, 0.42, 1.0),
        primary_hover: hsla(0.63, 0.80, 0.38, 1.0),
        success: hsla(0.40, 0.65, 0.38, 1.0),
        warning: hsla(0.10, 0.80, 0.45, 1.0),
        error: hsla(0.0, 0.70, 0.45, 1.0),
        bg_primary: hsla(0.0, 0.0, 0.96, 1.0),
        bg_sidebar: hsla(0.0, 0.0, 0.98, 1.0),
        bg_surface: hsla(0.0, 0.0, 0.93, 1.0),
        border: hsla(0.0, 0.0, 0.82, 1.0),
        text_primary: hsla(0.0, 0.0, 0.12, 1.0),
        text_muted: hsla(0.0, 0.0, 0.45, 1.0),
        terminal_bg: hsla(0.0, 0.0, 0.98, 1.0),
        hover_bg: hsla(0.0, 0.0, 0.0, 0.06),
        selected_bg: hsla(0.0, 0.0, 0.88, 1.0),
        badge_bg: hsla(0.0, 0.0, 0.88, 1.0),
        toggle_off_bg: hsla(0.0, 0.0, 0.78, 1.0),
        toggle_off_knob: hsla(0.0, 0.0, 0.96, 1.0),
        backdrop: hsla(0.0, 0.0, 0.0, 0.45),
        syntax_keyword: hsla(0.76, 0.65, 0.42, 1.0),
        syntax_builtin: hsla(0.50, 0.60, 0.38, 1.0),
        syntax_comment: hsla(0.0, 0.0, 0.55, 1.0),
        syntax_string: hsla(0.35, 0.55, 0.35, 1.0),
        syntax_variable: hsla(0.08, 0.70, 0.45, 1.0),
        syntax_operator: hsla(0.0, 0.0, 0.35, 1.0),
        syntax_number: hsla(0.47, 0.50, 0.38, 1.0),
        syntax_command_sub: hsla(0.12, 0.65, 0.42, 1.0),
        syntax_template_var: hsla(0.08, 0.80, 0.45, 1.0),
        line_number_fg: hsla(0.0, 0.0, 0.60, 1.0),
        line_number_bg: hsla(0.0, 0.0, 0.94, 1.0),
        cursor_line_bg: hsla(0.0, 0.0, 0.0, 0.04),
        hint_bg: hsla(0.0, 0.0, 0.0, 0.06),
    }
}

fn dracula() -> Palette {
    build(Seed {
        is_dark: true,
        bg: hx(0x282a36),
        sidebar: hx(0x21222c),
        surface: hx(0x343746),
        selected: hx(0x44475a),
        border: hx(0x44475a),
        text: hx(0xf8f8f2),
        muted: hx(0x6272a4),
        primary: hx(0xbd93f9),
        success: hx(0x50fa7b),
        warning: hx(0xffb86c),
        error: hx(0xff5555),
        keyword: hx(0xff79c6),
        builtin: hx(0x8be9fd),
        string: hx(0xf1fa8c),
        variable: hx(0xffb86c),
        number: hx(0xbd93f9),
        comment: hx(0x6272a4),
        terminal_bg: hx(0x282a36),
    })
}

fn nord() -> Palette {
    build(Seed {
        is_dark: true,
        bg: hx(0x2e3440),
        sidebar: hx(0x2b303b),
        surface: hx(0x3b4252),
        selected: hx(0x434c5e),
        border: hx(0x434c5e),
        text: hx(0xeceff4),
        muted: hx(0x7b88a1),
        primary: hx(0x88c0d0),
        success: hx(0xa3be8c),
        warning: hx(0xebcb8b),
        error: hx(0xbf616a),
        keyword: hx(0x81a1c1),
        builtin: hx(0x8fbcbb),
        string: hx(0xa3be8c),
        variable: hx(0xd08770),
        number: hx(0xb48ead),
        comment: hx(0x616e88),
        terminal_bg: hx(0x2e3440),
    })
}

fn tokyo_night() -> Palette {
    build(Seed {
        is_dark: true,
        bg: hx(0x1a1b26),
        sidebar: hx(0x16161e),
        surface: hx(0x1f2335),
        selected: hx(0x2a2e42),
        border: hx(0x2a2e42),
        text: hx(0xc0caf5),
        muted: hx(0x565f89),
        primary: hx(0x7aa2f7),
        success: hx(0x9ece6a),
        warning: hx(0xe0af68),
        error: hx(0xf7768e),
        keyword: hx(0xbb9af7),
        builtin: hx(0x7dcfff),
        string: hx(0x9ece6a),
        variable: hx(0xff9e64),
        number: hx(0xff9e64),
        comment: hx(0x565f89),
        terminal_bg: hx(0x1a1b26),
    })
}

fn gruvbox_dark() -> Palette {
    build(Seed {
        is_dark: true,
        bg: hx(0x282828),
        sidebar: hx(0x1d2021),
        surface: hx(0x32302f),
        selected: hx(0x3c3836),
        border: hx(0x3c3836),
        text: hx(0xebdbb2),
        muted: hx(0x928374),
        primary: hx(0xfe8019),
        success: hx(0xb8bb26),
        warning: hx(0xfabd2f),
        error: hx(0xfb4934),
        keyword: hx(0xfb4934),
        builtin: hx(0x8ec07c),
        string: hx(0xb8bb26),
        variable: hx(0x83a598),
        number: hx(0xd3869b),
        comment: hx(0x928374),
        terminal_bg: hx(0x282828),
    })
}

fn solarized_dark() -> Palette {
    build(Seed {
        is_dark: true,
        bg: hx(0x002b36),
        sidebar: hx(0x00252e),
        surface: hx(0x073642),
        selected: hx(0x073642),
        border: hx(0x0a4351),
        text: hx(0x93a1a1),
        muted: hx(0x586e75),
        primary: hx(0x268bd2),
        success: hx(0x859900),
        warning: hx(0xb58900),
        error: hx(0xdc322f),
        keyword: hx(0x859900),
        builtin: hx(0x2aa198),
        string: hx(0x2aa198),
        variable: hx(0xcb4b16),
        number: hx(0xd33682),
        comment: hx(0x586e75),
        terminal_bg: hx(0x002b36),
    })
}

fn solarized_light() -> Palette {
    build(Seed {
        is_dark: false,
        bg: hx(0xfdf6e3),
        sidebar: hx(0xf5efdc),
        surface: hx(0xeee8d5),
        selected: hx(0xe3ddc9),
        border: hx(0xddd6c1),
        text: hx(0x586e75),
        muted: hx(0x93a1a1),
        primary: hx(0x268bd2),
        success: hx(0x859900),
        warning: hx(0xb58900),
        error: hx(0xdc322f),
        keyword: hx(0x859900),
        builtin: hx(0x2aa198),
        string: hx(0x2aa198),
        variable: hx(0xcb4b16),
        number: hx(0xd33682),
        comment: hx(0x93a1a1),
        terminal_bg: hx(0xfdf6e3),
    })
}

fn catppuccin_mocha() -> Palette {
    build(Seed {
        is_dark: true,
        bg: hx(0x1e1e2e),
        sidebar: hx(0x181825),
        surface: hx(0x313244),
        selected: hx(0x45475a),
        border: hx(0x45475a),
        text: hx(0xcdd6f4),
        muted: hx(0xa6adc8),
        primary: hx(0xcba6f7),
        success: hx(0xa6e3a1),
        warning: hx(0xf9e2af),
        error: hx(0xf38ba8),
        keyword: hx(0xcba6f7),
        builtin: hx(0x89dceb),
        string: hx(0xa6e3a1),
        variable: hx(0xfab387),
        number: hx(0xfab387),
        comment: hx(0x6c7086),
        terminal_bg: hx(0x1e1e2e),
    })
}

fn one_dark() -> Palette {
    build(Seed {
        is_dark: true,
        bg: hx(0x282c34),
        sidebar: hx(0x21252b),
        surface: hx(0x2c313a),
        selected: hx(0x3a3f4b),
        border: hx(0x3a3f4b),
        text: hx(0xabb2bf),
        muted: hx(0x5c6370),
        primary: hx(0x61afef),
        success: hx(0x98c379),
        warning: hx(0xe5c07b),
        error: hx(0xe06c75),
        keyword: hx(0xc678dd),
        builtin: hx(0x56b6c2),
        string: hx(0x98c379),
        variable: hx(0xd19a66),
        number: hx(0xd19a66),
        comment: hx(0x5c6370),
        terminal_bg: hx(0x282c34),
    })
}

fn monokai() -> Palette {
    build(Seed {
        is_dark: true,
        bg: hx(0x272822),
        sidebar: hx(0x1e1f1c),
        surface: hx(0x33342c),
        selected: hx(0x3e3d32),
        border: hx(0x3e3d32),
        text: hx(0xf8f8f2),
        muted: hx(0x75715e),
        primary: hx(0xf92672),
        success: hx(0xa6e22e),
        warning: hx(0xe6db74),
        error: hx(0xf92672),
        keyword: hx(0xf92672),
        builtin: hx(0x66d9ef),
        string: hx(0xe6db74),
        variable: hx(0xfd971f),
        number: hx(0xae81ff),
        comment: hx(0x75715e),
        terminal_bg: hx(0x272822),
    })
}

fn rose_pine() -> Palette {
    build(Seed {
        is_dark: true,
        bg: hx(0x191724),
        sidebar: hx(0x1f1d2e),
        surface: hx(0x26233a),
        selected: hx(0x403d52),
        border: hx(0x403d52),
        text: hx(0xe0def4),
        muted: hx(0x908caa),
        primary: hx(0xc4a7e7),
        success: hx(0x9ccfd8),
        warning: hx(0xf6c177),
        error: hx(0xeb6f92),
        keyword: hx(0xc4a7e7),
        builtin: hx(0x9ccfd8),
        string: hx(0xf6c177),
        variable: hx(0xebbcba),
        number: hx(0x31748f),
        comment: hx(0x6e6a86),
        terminal_bg: hx(0x191724),
    })
}

/// Resolve a [`ThemePreference`] into its concrete palette.
pub fn palette_for(pref: &ThemePreference) -> Palette {
    use ThemePreference::*;
    match pref {
        Dark | System => dark_palette(),
        Light => light_palette(),
        Dracula => dracula(),
        Nord => nord(),
        TokyoNight => tokyo_night(),
        GruvboxDark => gruvbox_dark(),
        SolarizedDark => solarized_dark(),
        SolarizedLight => solarized_light(),
        CatppuccinMocha => catppuccin_mocha(),
        OneDark => one_dark(),
        Monokai => monokai(),
        RosePine => rose_pine(),
    }
}

/// ShellDeck brand colors — every accessor reads from the active palette.
pub struct ShellDeckColors;

impl ShellDeckColors {
    /// Swap the active theme to the given preference.
    pub fn set_theme(pref: &ThemePreference) {
        *ACTIVE.write() = palette_for(pref);
    }

    /// Backwards-compatible dark/light toggle.
    pub fn set_dark_mode(dark: bool) {
        *ACTIVE.write() = if dark {
            dark_palette()
        } else {
            light_palette()
        };
    }

    /// The full active palette (handy for swatches/previews).
    pub fn palette() -> Palette {
        *ACTIVE.read()
    }

    pub fn is_dark() -> bool {
        ACTIVE.read().is_dark
    }

    /// Primary brand color
    pub fn primary() -> Hsla {
        ACTIVE.read().primary
    }

    /// Primary hover
    pub fn primary_hover() -> Hsla {
        ACTIVE.read().primary_hover
    }

    /// Success green
    pub fn success() -> Hsla {
        ACTIVE.read().success
    }

    /// Warning amber
    pub fn warning() -> Hsla {
        ACTIVE.read().warning
    }

    /// Error red
    pub fn error() -> Hsla {
        ACTIVE.read().error
    }

    /// Background primary
    pub fn bg_primary() -> Hsla {
        ACTIVE.read().bg_primary
    }

    /// Sidebar background
    pub fn bg_sidebar() -> Hsla {
        ACTIVE.read().bg_sidebar
    }

    /// Surface background (cards, panels)
    pub fn bg_surface() -> Hsla {
        ACTIVE.read().bg_surface
    }

    /// Border color
    pub fn border() -> Hsla {
        ACTIVE.read().border
    }

    /// Text primary
    pub fn text_primary() -> Hsla {
        ACTIVE.read().text_primary
    }

    /// Text secondary/muted
    pub fn text_muted() -> Hsla {
        ACTIVE.read().text_muted
    }

    /// Terminal background
    pub fn terminal_bg() -> Hsla {
        ACTIVE.read().terminal_bg
    }

    /// Hover background for interactive items
    pub fn hover_bg() -> Hsla {
        ACTIVE.read().hover_bg
    }

    /// Selected/active item background
    pub fn selected_bg() -> Hsla {
        ACTIVE.read().selected_bg
    }

    /// Small badge/chip background
    pub fn badge_bg() -> Hsla {
        ACTIVE.read().badge_bg
    }

    /// Toggle switch off-state background
    pub fn toggle_off_bg() -> Hsla {
        ACTIVE.read().toggle_off_bg
    }

    /// Toggle switch off-state knob color
    pub fn toggle_off_knob() -> Hsla {
        ACTIVE.read().toggle_off_knob
    }

    /// Overlay backdrop color (semi-transparent)
    pub fn backdrop() -> Hsla {
        ACTIVE.read().backdrop
    }

    // ---- Syntax highlighting colors ----

    /// Keywords (if, then, for, while, etc.)
    pub fn syntax_keyword() -> Hsla {
        ACTIVE.read().syntax_keyword
    }

    /// Builtins (echo, cd, grep, etc.)
    pub fn syntax_builtin() -> Hsla {
        ACTIVE.read().syntax_builtin
    }

    /// Comments
    pub fn syntax_comment() -> Hsla {
        ACTIVE.read().syntax_comment
    }

    /// Strings
    pub fn syntax_string() -> Hsla {
        ACTIVE.read().syntax_string
    }

    /// Variables ($VAR, ${VAR})
    pub fn syntax_variable() -> Hsla {
        ACTIVE.read().syntax_variable
    }

    /// Operators (|, &&, >, etc.)
    pub fn syntax_operator() -> Hsla {
        ACTIVE.read().syntax_operator
    }

    /// Numbers
    pub fn syntax_number() -> Hsla {
        ACTIVE.read().syntax_number
    }

    /// Command substitution ($(...), `...`)
    pub fn syntax_command_sub() -> Hsla {
        ACTIVE.read().syntax_command_sub
    }

    /// Template variable `{{name}}`
    pub fn syntax_template_var() -> Hsla {
        ACTIVE.read().syntax_template_var
    }

    /// Line number foreground
    pub fn line_number_fg() -> Hsla {
        ACTIVE.read().line_number_fg
    }

    /// Line number gutter background
    pub fn line_number_bg() -> Hsla {
        ACTIVE.read().line_number_bg
    }

    /// Active cursor line background
    pub fn cursor_line_bg() -> Hsla {
        ACTIVE.read().cursor_line_bg
    }

    /// Subtle hint background (status bar pill, etc.)
    pub fn hint_bg() -> Hsla {
        ACTIVE.read().hint_bg
    }

    /// Connected status
    pub fn status_connected() -> Hsla {
        Self::success()
    }

    /// Disconnected status
    pub fn status_disconnected() -> Hsla {
        Self::text_muted()
    }

    /// Error status
    pub fn status_error() -> Hsla {
        Self::error()
    }
}

/// Build an adabraka-ui `Theme` whose color tokens follow the currently-active
/// `ShellDeckColors` palette. Used at startup (`main.rs`) and every time the
/// user switches theme (`Workspace::apply_palette`), so that all adabraka
/// widgets — `Input`, `Button`, `Card`, popovers, … — share the same look
/// as the app's custom-drawn UI instead of adabraka's shadcn defaults.
pub fn adabraka_theme_from_palette() -> adabraka_ui::prelude::Theme {
    use adabraka_ui::prelude::Theme;
    let mut theme = if ShellDeckColors::is_dark() {
        Theme::dark()
    } else {
        Theme::light()
    };
    let t = &mut theme.tokens;
    t.background = ShellDeckColors::bg_primary();
    t.foreground = ShellDeckColors::text_primary();
    t.card = ShellDeckColors::bg_surface();
    t.card_foreground = ShellDeckColors::text_primary();
    t.popover = ShellDeckColors::bg_surface();
    t.popover_foreground = ShellDeckColors::text_primary();
    // shadcn/ui `muted` is meant as a *visible* mid-tone surface (control
    // tracks, disabled fills, muted cards). Our `hint_bg` (semi-transparent
    // 15% grey) collapses into the background on any dark palette — the
    // adabraka `Toggle` OFF state was rendering as an invisible track under
    // an invisible knob on Catppuccin Mocha, Dracula, One Dark… Point at
    // `selected_bg` instead: it's already tuned per palette to stay visible
    // on both light and dark bases (see `.agents/theming.md`).
    t.muted = ShellDeckColors::selected_bg();
    t.muted_foreground = ShellDeckColors::text_muted();
    // Subtle list-row highlight — not full primary fill (unreadable with dark text).
    t.accent = ShellDeckColors::primary().opacity(if ShellDeckColors::is_dark() {
        0.22
    } else {
        0.14
    });
    t.accent_foreground = ShellDeckColors::text_primary();
    t.primary = ShellDeckColors::primary();
    t.primary_foreground = gpui::white();
    t.secondary = ShellDeckColors::badge_bg();
    t.secondary_foreground = ShellDeckColors::text_primary();
    t.destructive = ShellDeckColors::error();
    t.destructive_foreground = gpui::white();
    t.border = ShellDeckColors::border();
    t.input = ShellDeckColors::border();
    t.ring = ShellDeckColors::primary();
    // Adabraka's `Input` renders text in `font_mono` (JetBrains Mono by
    // default) — great for a code editor field, wrong for the plain-text
    // composer / form fields we use everywhere. Point mono at the UI font so
    // adabraka widgets pick up the same face as the rest of the app. Terminal
    // and file editors have their own font-family config and are unaffected.
    t.font_mono = t.font_family.clone();
    theme
}
