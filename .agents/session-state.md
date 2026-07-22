# Session & config sync

ShellDeck has **one authoritative `AppConfig` in `Workspace::app_config`**, but
`SettingsView` keeps a **second copy** (`settings.config`) for persisting
General/Terminal prefs. If those copies diverge, innocent UI actions resurrect
dead session state.

**Incident (2026-07):** logout cleared `workspace.app_config`, user clicked
**Hide nav** → settings saved its stale snapshot → full `ConfigChanged` replaced
`app_config` → Support mode + expired token + 401 toasts returned. Hide nav was
innocent; the stale snapshot was the bug.

## Architecture

```
Workspace.app_config          ← authoritative (login, logout, cloud_sync, account)
        ↑ merge slices only
SettingsView.config           ← snapshot for general/terminal persistence
        ↓ save_config()
   disk (shelldeck.toml)
```

**Who owns which fields**

| Field(s) | Owner | Persisted via |
|----------|-------|---------------|
| `general.*` | Settings (merged into workspace) | `settings.save_config()` |
| `terminal.*`, `editor.*`, `tray.*`, `companion.*`, `ai.*` | Settings (merged into workspace) | `settings.save_config()` |
| `theme` | Workspace + Settings (`ThemeChanged`) | both paths |
| `account`, `cloud_sync`, `jeanclaude`, `bext_cloud`, … | **Workspace only** | `app_config.save()` in workspace handlers |

## Hard rules

### 1. Never wholesale-replace `app_config` from Settings

```rust
// ❌ FORBIDDEN — resurrects stale account/token
self.app_config = config.clone();

// ✅ merge only what Settings owns
self.app_config.general = config.general.clone();
self.app_config.terminal = config.terminal.clone();
```

Handler: `Workspace::handle_settings_event` → `SettingsEvent::ConfigChanged`.

### 2. Sync settings snapshot after workspace mutates session

Call `Workspace::sync_settings_config(cx)` whenever `app_config.account` or
`app_config.cloud_sync` (or any non-settings slice) changes in the workspace:

| Event | Function |
|-------|----------|
| Login OK | `apply_login` |
| Logout | `invalidate_cloud_session` (via `logout_account`) |
| whoami 401 | `invalidate_cloud_session` + `account_status = Rejected` |
| Any future token revoke | `invalidate_cloud_session` |

```rust
fn sync_settings_config(&mut self, cx: &mut Context<Self>) {
    let snapshot = self.app_config.clone();
    self.settings.update(cx, |settings, cx| {
        settings.config = snapshot;
        cx.notify();
    });
}
```

### 3. Use `invalidate_cloud_session` for every local sign-out

Do not duplicate ad-hoc clears. One function:

- Clears `account`, token, `enabled`, active site
- Saves to disk + `sync_settings_config`
- Stops `_support_poll_task` / `_issues_poll`
- `sidebar.set_site_filter(None)`
- `activate_current_mode(cx)` → Dev surface when logged out

Server-side logout (`cloud_account::logout`) is best-effort **before**
`invalidate_cloud_session`.

### 4. API 401 ≠ “still logged in”

If `whoami` / Manage client returns auth rejected:

- **Do not** leave `account` in config while the token is dead.
- Invalidate locally and toast once — avoid polling Support/issues with a bad token.

`signed_in()` = `cloud_sync.is_configured() && account.is_some()` — both must
clear on 401.

## PR checklist (config / auth / settings touch)

- [ ] Did I change `account`, `cloud_sync.token`, or `cloud_sync.enabled`?
      → call `sync_settings_config` (or go through `invalidate_cloud_session`).
- [ ] Did I add a new `SettingsEvent::ConfigChanged` consumer?
      → merge slices only, never full `AppConfig`.
- [ ] Did I add a new view that holds its own `AppConfig` copy?
      → **don't** — read from workspace or merge explicitly like Settings.
- [ ] Did I add background poll gated on `is_configured()`?
      → ensure poll stops when `invalidate_cloud_session` runs.
- [ ] Manual test: login → logout → toggle sidebar **Hide nav** → must stay Dev,
      titlebar **Se connecter**, no 401 spam.

## Debugging “ghost login”

Symptoms: logged out but Support/User returns, or Karim chip with 401 toasts.

1. Grep `app_config = config.clone()` — should be **zero** hits outside tests.
2. Check `settings.config.account` vs `workspace.app_config.account` mentally —
   any settings save without prior `sync_settings_config` after logout?
3. Hide nav / font size / theme are **not** auth actions — look at config sync.

## Related code

- `crates/shelldeck-ui/src/workspace/mod.rs` — `sync_settings_config`,
  `invalidate_cloud_session`, `handle_settings_event`, `logout_account`,
  `check_account_on_startup`, `apply_login`
- `crates/shelldeck-ui/src/settings.rs` — `save_config`, `set_sidebar_nav_collapsed`
- `crates/shelldeck-core/src/config/cloud_sync.rs` — `is_configured()`
