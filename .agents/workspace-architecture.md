# Workspace architecture

`crates/shelldeck-ui/src/workspace/mod.rs` is an **orchestrator**, not a home
for feature implementations. It was reduced from 15,288 lines to roughly
1,500 lines in July 2026 after domain logic, polling, event handling, and
rendering had accumulated in one file.

## Hard boundary

`workspace/mod.rs` owns only:

- submodule declarations and imports;
- shared `Workspace` state and small shared types;
- `Workspace::new`, entity construction, and event subscriptions;
- small pure helpers shared by several workspace modules;
- tests for those shared helpers.

Do **not** add domain workflows, network operations, polling loops, substantial
event handlers, or surface rendering to `mod.rs`.

The file must stay below **2,000 lines**. If a change would cross that limit,
the same change must extract or extend a domain module instead.

## Where new code goes

- Account/session and authentication → `workspace/account.rs`
- Cloud profile sync → `workspace/cloud_sync.rs`
- AI workflows/actions → `workspace/ai.rs`
- Requests/issues operations → `workspace/requests.rs`
- Requests rendering → `workspace/request_views.rs`
- User dashboard rendering → `workspace/user_home.rs`
- Window chrome and titlebar menus → `workspace/chrome.rs`
- Root surface composition → `workspace/render.rs`
- Navigation/session restoration → `workspace/navigation.rs`
- Settings and UI event coordination → `workspace/events.rs`
- Tray integration → `workspace/tray.rs`
- Support, Monique, Fleet, Bext, Sites → their same-named modules

For a genuinely new subsystem, create one focused
`crates/shelldeck-ui/src/workspace/<domain>.rs` module and put its
`impl Workspace` there. Use `pub(super)` only for methods called across the
workspace module boundary; keep domain-internal helpers private.

## Change checklist

Before committing a workspace change:

```bash
wc -l crates/shelldeck-ui/src/workspace/mod.rs
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig cargo check -p shelldeck-ui
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig cargo test -p shelldeck-ui
```

If `mod.rs` grows materially, stop and move the new responsibility to its
domain before continuing. Never postpone the split to a follow-up.
