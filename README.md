# ShellDeck

A GPU-accelerated native desktop terminal and SSH companion app built with Rust. ShellDeck provides a unified control plane for managing terminal sessions, SSH connections, remote script execution, and port forwarding from a polished sidebar/dashboard UI.

## Features

- **GPU-Accelerated Rendering** -- Native performance via [GPUI](https://gpui.rs) framework
- **SSH Connection Manager** -- Auto-imports from `~/.ssh/config`, supports jump hosts, key auth, and password auth via OS keychain
- **Terminal Emulator** -- Full VTE escape sequence support (SGR, CSI, OSC), scrollback, alt screen buffer, BCE
- **Nested Pane Layouts** -- tmux-like recursive split tree (N panes, mixed horizontal/vertical) with drag-to-resize dividers and click/keyboard focus
- **Port Forwarding** -- Local, remote, and SOCKS proxy tunnels with visual status
- **Script Editor** -- Write, save, and execute scripts on remote hosts with variable templating
- **Server Sync** -- Side-by-side file browser, nginx/database discovery, rsync/mysqldump/pg_dump sync wizard
- **Command Palette** -- Fuzzy-filtered command search (`Ctrl+Shift+P`)
- **Session Persistence** -- Restore workspace layout and sessions across restarts
- **Search** -- In-terminal text search with match highlighting
- **URL Detection** -- Clickable URLs detected in terminal output
- **Themes** -- 13 built-in app themes (Dracula, Nord, Tokyo Night, Gruvbox, Catppuccin, …) plus terminal color themes, with live preview via the titlebar switcher and `Ctrl+P`
- **Auto-Update** -- Checks for and installs new releases automatically
- **Context Menu** -- Right-click for copy, paste, search, and URL actions
- **Git Integration** -- Branch indicator and status in the UI

## Install

Download the latest release from **[shelldeck.1clic.pro](https://shelldeck.1clic.pro)** (Linux AppImage/tarball, macOS DMG, Windows installer), or use the install script:

```bash
# Linux / macOS
curl -fsSL https://shelldeck.1clic.pro/install.sh | bash
```

```powershell
# Windows
powershell -c "irm shelldeck.1clic.pro/install.ps1 | iex"
```

ShellDeck auto-updates itself once installed. To build from source instead, see below.

## Requirements

- **Rust nightly**, pinned in `rust-toolchain.toml` (do not change to floating `nightly` — it breaks the macOS build; see `CLAUDE.md`)
- **Linux**: `libssl-dev`, `pkg-config`, `libxkbcommon-dev`, `libwayland-dev`
- **macOS**: Xcode Command Line Tools, OpenSSL (`brew install openssl`)

### Install system dependencies (Ubuntu/Debian)

```bash
sudo apt install libssl-dev pkg-config libxkbcommon-dev libwayland-dev
```

## Build & Run

```bash
# Clone
git clone https://github.com/nickarrow/shelldeck.git
cd shelldeck

# Build
cargo build

# Run
cargo run

# Run in release mode
cargo run --release
```

On Linux you may need to set `PKG_CONFIG_PATH` if OpenSSL isn't found:

```bash
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig cargo build
```

## Project Structure

```
shelldeck/
├── crates/
│   ├── shelldeck/            # Binary crate -- app entry point, keybindings
│   ├── shelldeck-core/       # Models, config, SSH config parser, keychain
│   ├── shelldeck-ssh/        # SSH client, sessions, tunnels, remote exec
│   ├── shelldeck-terminal/   # PTY, VTE parser, terminal grid
│   └── shelldeck-ui/         # GPUI views, sidebar, dashboard, forms
├── patches/
│   └── adabraka-gpui/        # Patched GPUI fork
├── Cargo.toml                # Workspace manifest
└── rust-toolchain.toml       # Nightly toolchain
```

## Configuration

ShellDeck stores its configuration in `~/.local/share/ShellDeck/`:

| File | Purpose |
|------|---------|
| `shelldeck.toml` | App settings (theme, font, keybindings) |
| `connections.json` | Saved connections, scripts, port forwards |
| `workspace.json` | Window layout and session state |

SSH credentials are stored securely in your OS keychain -- never in config files.

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+Shift+P` | Command palette |
| `Ctrl+Shift+T` | New tab |
| `Ctrl+Shift+W` | Close tab |
| `Ctrl+Shift+F` | Search in terminal |
| `Ctrl+Shift+C` | Copy selection |
| `Ctrl+Shift+V` | Paste |
| `Ctrl+Shift+Z` | Toggle zoom |
| `Ctrl+Tab` | Next tab |
| `Ctrl+Shift+Tab` | Previous tab |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

[MIT](LICENSE)
