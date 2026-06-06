# ShellDeck - CLAUDE.md

## Project Overview

ShellDeck is a GPU-accelerated native desktop SSH & Terminal companion app built with Rust. It provides a unified control plane for managing terminal sessions, SSH connections, remote script execution, and port forwarding from a polished sidebar/dashboard UI.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| UI Framework | GPUI (via adabraka-gpui fork) |
| Component Library | adabraka-ui (85+ components) |
| SSH | russh (async, tokio-based) |
| Terminal | portable-pty + vte |
| Async Runtime | tokio |
| Config | serde + toml |
| Credentials | keyring (OS keychain) |
| File Watching | notify |

## Workspace Structure

```
shelldeck/
├── Cargo.toml              # Workspace root
├── rust-toolchain.toml     # Nightly required (GPUI dependency)
├── crates/
│   ├── shelldeck/          # Main binary crate (app entry, wiring)
│   ├── shelldeck-core/     # Models, config, SSH config parser, keychain
│   ├── shelldeck-ssh/      # SSH client, sessions, tunnels, remote exec
│   ├── shelldeck-terminal/ # PTY, VTE parser, terminal grid
│   └── shelldeck-ui/       # GPUI views, sidebar, dashboard, forms
```

## Essential Commands

```bash
# Build (requires nightly + system deps)
cargo build

# Check compilation
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig cargo check

# Run
cargo run

# Run specific example
cargo run --example hello_world
```

## System Dependencies

- **Rust nightly** (specified in rust-toolchain.toml)
- **OpenSSL dev** (`libssl-dev` on Ubuntu, `openssl-devel` on Fedora)
- **pkg-config**
- On Linux: `libxkbcommon-dev`, `libwayland-dev` (for GPUI)

## Crate Dependencies

### shelldeck-core
Models (Connection, PortForward, Script, ExecutionRecord), SSH config parser, app config (TOML), connection store (JSON), keychain wrapper, config file watcher.

### shelldeck-ssh
SSH client (russh), session management, port forwarding tunnels (local/remote/SOCKS), remote command execution with streaming output, connection pool.

### shelldeck-terminal
Terminal grid (with scrollback, alt screen buffer, scroll regions), VTE escape sequence parser (full SGR, CSI, OSC support), local PTY spawning, terminal session management.

### shelldeck-ui
GPUI views: Workspace layout, Sidebar (connection tree), Dashboard (stats cards, activity feed), Terminal view (grid renderer, tabs), Port Forward view (table + visual map), Script Editor, Settings, Status Bar, Connection Form.

### shelldeck (main)
App entry point, GPUI Application setup, theme initialization, keyboard shortcuts, app state management.

## Key Patterns

### GPUI Views
```rust
use gpui::*;
use gpui::prelude::*;

struct MyView { /* state */ }

impl Render for MyView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().size_full().child("Hello")
    }
}
```

### adabraka-ui Components
```rust
use adabraka_ui::prelude::*;

Button::new("id", "Label").variant(ButtonVariant::Default)
```

### Conditional Elements
Use Rust if/else instead of `.when()` chains:
```rust
let mut el = div().flex();
if condition {
    el = el.child(something);
} else {
    el = el.child(other);
}
```

## Config Locations

- App config: `~/.local/share/ShellDeck/shelldeck.toml`
- Connection store: `~/.local/share/ShellDeck/connections.json`
- SSH config (read-only): `~/.ssh/config`

## Releasing & Auto-Update

- **Cut a release:** bump `[workspace.package] version` in the root `Cargo.toml`, commit, then push a matching `vX.Y.Z` git tag. The tag triggers `.github/workflows/release.yml`: it builds linux/macos/windows, creates the GitHub Release with assets, and publishes the update manifest to Cloudflare KV (key `latest-release`). If any build fails, the release + manifest jobs are SKIPPED — fix and re-tag.
- **Site + update server:** a single Cloudflare Worker (`cloudflare/update-worker/`, bound to `shelldeck.1clic.pro`) serves the landing/marketing page (`/`), the update API (`/api/releases/latest?platform=…`), and install scripts (`/install.sh`, `/install.ps1`).
- **Update client:** `crates/shelldeck-update/` polls that API hourly. Platform keys are `{os}-{arch}` and use **`macos-*`, never `darwin-*`** — manifest, workflow, client, and worker must all agree.
- **Worker code deploy:** `release.yml` only updates KV *data*. Worker *code* deploys via `.github/workflows/deploy-worker.yml` on changes to `cloudflare/update-worker/**` — but that needs the `CLOUDFLARE_API_TOKEN` secret to have **Workers Scripts: Edit** permission. Otherwise deploy manually: `cd cloudflare/update-worker && wrangler login && npm run deploy`.

## Critical Rules

- NEVER write to `~/.ssh/config` - ShellDeck-specific data goes in its own config
- Rust nightly is **pinned** in `rust-toolchain.toml` (`nightly-2026-03-06`). Do NOT float it back to `nightly`: a newer nightly drops the `simd_fmin/simd_fmax` intrinsics `pathfinder_simd` needs and breaks the macOS release build (Linux/Windows are unaffected, so `cargo check` won't catch it). Bump only deliberately and verify the macOS release build.
- ALWAYS set PKG_CONFIG_PATH on Linux for OpenSSL
- Terminal grid operations must be fast - they're on the rendering hot path
- Use `parking_lot::Mutex` for thread-safe grid access, not `std::sync::Mutex`
- Terminal repaint is event-driven (PTY reader → channel → refresh task), not polled — don't reintroduce a fixed-interval poll. The main grid paint loop must keep using `shape_line`; the `paint_glyph`/`GlyphCache` fast path silently fails to render (breaks bold/colored glyphs).
