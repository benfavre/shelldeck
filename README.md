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
- **Cloud Sync** -- Pull SSH connection profiles from the [Inklura Manage](https://manage.inklura.fr) portal into your connection store
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
git clone https://github.com/benfavre/shelldeck.git
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

## Cloud Sync (Inklura Manage)

ShellDeck can pull SSH connection profiles from the [Inklura Manage](https://manage.inklura.fr) portal so a team's server inventory stays in sync across machines. Synced connections show up alongside your `~/.ssh/config` and manual entries, tagged as **cloud**-sourced; they are refreshed on every sync and removed automatically when they disappear from the portal. Your local **manual** and **SSH-config** connections are never modified by sync.

### Sign in from the titlebar

The quickest way to connect is the **account chip in the titlebar** (top-right, next to the theme switcher). Click **Se connecter** and either:

- enter your Inklura Manage **email + password**, or
- use **single sign-on** — *SSO 1clic.pro*, *Google*, or *GitHub*. This opens your system browser to authorize the device, then hands a token back to ShellDeck automatically.
- or **Via le navigateur (mot de passe)** — opens the browser to the Manage password login page (handy when you already have a Manage session or a browser password manager), then authorizes and returns.

On success ShellDeck stores an account-bound sync token, enables Cloud Sync, and pulls your profiles. The chip then shows your name and a status dot (green = connected, gray = offline/unchecked, red = token rejected — sign in again). Use the chip's dropdown to **Synchroniser** on demand or **Se déconnecter** (which revokes the token server-side).

### Sites & Manage areas

Once signed in, a **site chip** appears in the titlebar (next to the account chip). It shows the active site — or **Tous les sites** — and its dropdown lets you:

- **Switch the active site**: the list pins the active site and sites that have connections to the top. Selecting one scopes the sidebar to that site's connections (plus your unbound manual/SSH entries); **Tous les sites** clears the filter. Connections bound to a site show a small site badge. The choice is remembered across restarts.
- **Open a Manage area** for the active site: each area (Dashboard, CMS, Helpdesk, E-commerce, Settings, …) opens in your browser, already scoped to that site.

The command palette (`Ctrl+Shift+P`) also has a **Switch Active Site** entry and, when a site is active, one **Site actif (…) : \<area\>** entry per area.

### Manual configuration

You can also configure Cloud Sync by hand — add a `[cloud_sync]` section to `~/.local/share/ShellDeck/shelldeck.toml`:

```toml
[cloud_sync]
enabled = true
base_url = "https://manage.inklura.fr"
token = "sd_..."          # get a token at manage.inklura.fr/manage/shelldeck
sync_on_startup = true     # pull profiles automatically at launch
```

- **Get a token** at [manage.inklura.fr/manage/shelldeck](https://manage.inklura.fr/manage/shelldeck).
- With `sync_on_startup = true`, ShellDeck syncs at launch (bounded by a 4s connect / 10s total timeout, so a portal outage never blocks startup).
- Trigger a sync anytime via the command palette (**Cloud Sync Now**) or the **Sync now** button under Settings → General → Cloud Sync.
- The token is stored in `shelldeck.toml`; the Settings screen only ever shows a masked hint of it.

## App modes (User / Support / Dev)

Signed in as an Inklura Manage **super-admin**, a three-segment **mode switcher** appears in the titlebar (left of the site chip):

- **Dev** (default) — the full ShellDeck workspace: terminals, SSH, port forwards, scripts, server sync, sites. This is exactly the classic app.
- **Support** — a native two-pane helpdesk console for support.inklura.fr: view filters (Tous / Non attribués / Les miens / Ouverts / En attente / SLA / Résolus) with live counts, the ticket list, and a conversation pane with a reply/note composer and an action bar (status, priority, assign, resolve). The list refreshes every ~30s while open.
- **User** — a manage-centric home: an account header plus a **Mes sites** list where each site has an **Activer** button and one-click deep links into its Manage areas (Dashboard, CMS, Helpdesk, E-commerce, Réglages, Console ShellDeck).

Switching modes never closes running terminal sessions — Dev surfaces are hidden, not destroyed. The selected mode is remembered across restarts. The command palette (`Ctrl+Shift+P`) has **Mode : Utilisateur / Support / Dev** entries too.

Non-super-admin accounts are locked to **User** mode (no dev surfaces, no switcher). When you're **not** signed in, ShellDeck runs as the classic full app (Dev), since it's a general-purpose terminal on its own.

## JeanClaude

[JeanClaude](https://github.com/benfavre/slack-claude-bot) is Ben's `#jean` Slack ticket bot, driven by headless Claude Code. ShellDeck is a **native client for it, replacing the bot's web dashboard** — the console lives in the app instead of a browser tab.

It appears only when a JeanClaude config is available (which de facto scopes it to super-admins and local overrides), sourced with this precedence:

1. A local `[jeanclaude]` section in `shelldeck.toml` (wins — e.g. to point at an SSH tunnel on `127.0.0.1`):
   ```toml
   [jeanclaude]
   url = "http://127.0.0.1:3100"
   user = "jean"
   pass = "…"
   ```
2. Otherwise the config delivered by the server in the sites feed (super-admin tokens only).

Where it shows up:

- **Dev mode** — a **JeanClaude** entry in the sidebar opens the full console: bot status (connected / paused / concurrency) with a "Dire dans #jean" input, **Aperçu** (pending confirmations with Confirmer/Rejeter + active tickets with heartbeat age and Annuler), **Historique** (status filter + detail with the per-ticket action log + Forcer/Annuler), **Cibles** (domain→server CRUD), and **Mémoire** (rules/notes CRUD). It polls every ~10s while open. The command palette has **JeanClaude : ouvrir la console** and **pause / reprendre**.
- **Support mode** — a compact JeanClaude strip (pending confirmations + active count) and an **Envoyer à Jean** action per ticket that files it through Jean's normal Slack intake.
- **User mode** — a **Demander à JeanClaude** card to send a request (a confirmer must still approve in `#jean`), with a read-only recent-activity list.

### Fleet runtime

Beyond controlling one dashboard, ShellDeck can be a **runtime for the Jean fleet** — the tenant/site-aware set of Jean instances managed from `manage.inklura.fr`. Signed in (Dev mode), a **Fleet** sidebar entry shows every instance (name, tenant/site, runtime, status dot, heartbeat age), the recent-jobs feed, and a toggle to make **this machine** a runtime.

When enabled, ShellDeck registers itself, heartbeats, and claims pending jobs for its instance, executing each by driving **headless Claude Code** (`claude -p`, subscription auth) in the configured working directory.

⚠️ **Safety.** Executing a job runs Claude Code with file/edit/command powers on this machine. It is **off by default** and gated hard:

- The runtime only runs when `[jean_runtime].enabled = true` **and** the instance's autonomy is **`auto`**.
- An instance set to **`confirm`** never auto-runs — each claimed job appears in the Fleet view with **Exécuter / Rejeter** and waits for an explicit click. New instances default to `confirm`.
- One job at a time per machine.

```toml
[jean_runtime]
enabled = false          # default — must be turned on explicitly
# instance_id = "…"      # filled in after the first registration
# workdir = "/home/you/infra"
# name = "my-machine"
```

Toggle it from the Fleet view or the command palette (**Fleet : activer / désactiver ce runtime**, **Fleet : ouvrir la flotte Jean**).

## Requests (hosted issue management)

ShellDeck has a built-in **request tracker** — per tenant/site issues that are synced to GitHub and can be dispatched to the Jean fleet.

- **User mode** — a **Mes demandes** section (below your sites): file a **Nouvelle demande** (title + details + priority), see your tenant's requests with their status and GitHub number, and open any one to read its body/comments and add your own. It's the durable, tracked path — the quick "Demander à JeanClaude" card stays for one-off Slack-style asks.
- **Support mode** — a **Demandes** tab in the console: the request queue for your scope (all tenants for staff). Open a request to see its thread and comment; staff get a triage action bar — set status, cycle priority, assign to me, **Dispatcher** to a tenant Jean instance, and **Créer sur GitHub** / refresh from GitHub. Any support ticket can be **Convertir[i] en demande** to become a tracked request.

Staff-only actions are gated server-side; the action bar only appears for staff tokens. Palette: **Nouvelle demande**, **Demandes (support)**.

## bext Cloud

A **bext Cloud** view (Dev mode, sidebar) integrates the hosted control plane at [cloud.bext.dev](https://cloud.bext.dev) and lets you manage a single bext instance directly. It has two tabs:

- **Cloud** — **Se connecter** signs in through your browser (the cloud CLI OIDC flow via auth.1clic.pro; the token is stored in `[bext_cloud]` of `shelldeck.toml`). Once connected: your identity (with a super-admin badge), a **dashboard** stat strip, and a **Sites** panel — the one-click WordPress sites with status and primary domain (open-in-browser), a **Nouveau site WordPress** form, and per-site **Mettre en ligne / Config / Détruire** (destroy asks to confirm). Super-admins also see the **bext instances** the cloud knows about with health/status. The list polls every ~15s while open.
- **Instance** — manage the sites on one bext box directly through its loopback site SDK (`/__bext/sdk/site/*`): set a target base URL + app-id, list/create sites, and go-live/destroy them.

Each SSH connection has a **bext** hover action that opens the Instance tab for that box. (v1 targets the local loopback `http://127.0.0.1` — managing a remote box over an SSH tunnel is the next step.) Palette: **bext Cloud : se connecter / nouveau site / ouvrir**.

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
