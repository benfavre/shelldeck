# Agents cockpit from the owner's mock-up — 2026-11-17 (SDUC-595)

Private headless GNOME Shell (Wayland, own D-Bus without service directories,
throwaway HOME, `GSETTINGS_BACKEND=memory`), French, 1400 px wide.

- [Start screen, Light](01-start-light.png): demo profile of `scripts/demo`
  (`--agents`), empty session.
- [Finished reply with Détails, Light](02-reply-details-light.png) and
  [Dark](03-reply-details-dark.png): `agent-console-preview` with
  `SHELLDECK_PREVIEW_MOCKUP=1 SHELLDECK_PREVIEW_DETAILS=1 SHELLDECK_PREVIEW_FR=1`
  (`SHELLDECK_PREVIEW_PLAIN_LIGHT=1` for Light). The « Application redémarrée »
  line comes from the fixture's reload.

Not checked on screen: windows under 840 px, macOS, Windows, a live provider
run, SSH targets.
