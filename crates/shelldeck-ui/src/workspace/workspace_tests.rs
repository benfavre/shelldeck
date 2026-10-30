use super::{
    post_login_display_progress, post_login_simulated_progress, post_login_splash_opacity,
    post_login_splash_remaining, shortcut_failure_toasts, shortcut_status_is_failure,
    ShortcutToastKind, POST_LOGIN_SPLASH_MIN_MS,
};
use crate::settings::{CompanionShortcutStatuses, ShortcutRegistrationStatus};

// SDTEST-2743 — SDUC-153: fixture account/config writes must never target
// the desktop profile, including relative or parent-traversing overrides.
#[cfg(target_os = "linux")]
#[test]
fn workspace_test_profile_rejects_desktop_and_incomplete_overrides() {
    use std::path::{Path, PathBuf};
    let home = Path::new("/home/operator");
    let roots = [
        "/tmp/fixture/config",
        "/tmp/fixture/data",
        "/tmp/fixture/cache",
    ]
    .map(|path| Some(PathBuf::from(path)));
    assert!(super::ui_test_profile_is_isolated(home, &roots));
    for index in 0..3 {
        for unsafe_root in [
            None,
            Some("relative"),
            Some("/home/operator"),
            Some("/home/operator/.config"),
            Some("/home/operator/.local/share"),
            Some("/home/operator/.cache"),
            Some("/tmp/fixture/../../home/operator/.config"),
        ] {
            let mut changed = roots.clone();
            changed[index] = unsafe_root.map(PathBuf::from);
            assert!(!super::ui_test_profile_is_isolated(home, &changed));
        }
    }
}

fn statuses(
    ai_dock: ShortcutRegistrationStatus,
    command_palette: ShortcutRegistrationStatus,
) -> CompanionShortcutStatuses {
    CompanionShortcutStatuses {
        ai_dock,
        command_palette,
    }
}

#[test]
fn post_login_splash_has_a_minimum_visible_duration() {
    let early = post_login_splash_remaining(std::time::Duration::from_millis(200))
        .expect("a fast sync should keep the splash visible");
    assert_eq!(
        early,
        std::time::Duration::from_millis(POST_LOGIN_SPLASH_MIN_MS - 200)
    );
    assert!(
        post_login_splash_remaining(std::time::Duration::from_millis(POST_LOGIN_SPLASH_MIN_MS))
            .is_none()
    );
}

#[test]
fn post_login_simulated_progress_is_staged_and_monotonic() {
    let checkpoints: Vec<f32> = (0..=100)
        .map(|step| post_login_simulated_progress(step as f32 / 100.0))
        .collect();
    assert_eq!(checkpoints[0], 0.0);
    assert_eq!(checkpoints[100], 1.0);
    assert!(
        checkpoints.windows(2).all(|pair| pair[1] >= pair[0]),
        "simulated progress must never move backwards"
    );
    assert!(post_login_simulated_progress(0.1) >= 0.17);
    assert!(post_login_simulated_progress(0.86) < 0.95);
}

// SDTEST-1709 — changing the splash animation key for the fade-out causes
// GPUI to rebuild its nested animations. The display reducer must pin both
// the bar and percentage at completion throughout that final repaint.
#[test]
fn post_login_progress_stays_complete_while_the_splash_fades() {
    assert_eq!(post_login_display_progress(true, 0.0), 1.0);
    assert_eq!(post_login_display_progress(true, 0.5), 1.0);
    assert_eq!(post_login_display_progress(true, 1.0), 1.0);
    assert_eq!(post_login_display_progress(false, 0.0), 0.0);
}

#[test]
fn post_login_splash_only_fades_when_dismissing() {
    assert_eq!(post_login_splash_opacity(false, 0.75), 1.0);
    assert_eq!(post_login_splash_opacity(true, 0.0), 1.0);
    assert_eq!(post_login_splash_opacity(true, 0.5), 0.5);
    assert_eq!(post_login_splash_opacity(true, 1.0), 0.0);
}

// SDTEST-1415 — only a genuinely failed registration counts. `Applying`
// and `PendingPortal` are in-flight: the Wayland GlobalShortcuts portal
// answers asynchronously, so treating either as a failure would toast on
// every single launch before the compositor has replied. `Disabled` is the
// user's own choice and never a failure.
#[test]
fn only_conflict_and_error_count_as_failures() {
    assert!(shortcut_status_is_failure(
        &ShortcutRegistrationStatus::Conflict
    ));
    assert!(shortcut_status_is_failure(
        &ShortcutRegistrationStatus::Error("BadAccess".into())
    ));

    for in_flight in [
        ShortcutRegistrationStatus::Disabled,
        ShortcutRegistrationStatus::Applying,
        ShortcutRegistrationStatus::Registered,
        ShortcutRegistrationStatus::PendingPortal,
    ] {
        assert!(
            !shortcut_status_is_failure(&in_flight),
            "{in_flight:?} must not toast"
        );
    }
}

// SDTEST-1416 — the companion config channel republishes statuses on every
// settings save, so an unchanged failure must stay silent. Only the
// transition *into* a failure is announced, or a user with a permanently
// conflicting shortcut gets a toast every time they touch Settings.
#[test]
fn only_the_transition_into_failure_toasts() {
    let ok = statuses(
        ShortcutRegistrationStatus::Registered,
        ShortcutRegistrationStatus::Registered,
    );
    let failed = statuses(
        ShortcutRegistrationStatus::Error("BadAccess".into()),
        ShortcutRegistrationStatus::Registered,
    );

    // Entering the failure announces it once.
    let toasts = shortcut_failure_toasts(&ok, &failed);
    assert_eq!(toasts.len(), 1);
    assert_eq!(toasts[0].0, ShortcutToastKind::AiDock);
    // The platform's own reason must reach the user, not be swallowed.
    assert!(toasts[0].1.contains("BadAccess"));

    // Republishing the same failure is silent.
    assert!(shortcut_failure_toasts(&failed, &failed).is_empty());
    // Recovering is silent too — nothing to warn about.
    assert!(shortcut_failure_toasts(&failed, &ok).is_empty());
}

// SDTEST-1417 — the two shortcuts are independent; one failing must not
// mask or duplicate the other, and both failing at once reports both.
#[test]
fn each_shortcut_reports_independently() {
    let ok = statuses(
        ShortcutRegistrationStatus::Registered,
        ShortcutRegistrationStatus::Registered,
    );
    let both = statuses(
        ShortcutRegistrationStatus::Error("BadAccess".into()),
        ShortcutRegistrationStatus::Conflict,
    );

    let toasts = shortcut_failure_toasts(&ok, &both);
    assert_eq!(toasts.len(), 2);
    let kinds: Vec<_> = toasts.iter().map(|(kind, _)| *kind).collect();
    assert!(kinds.contains(&ShortcutToastKind::AiDock));
    assert!(kinds.contains(&ShortcutToastKind::CommandPalette));

    // Palette-only failure does not mention the dock.
    let palette_only = statuses(
        ShortcutRegistrationStatus::Registered,
        ShortcutRegistrationStatus::Conflict,
    );
    let toasts = shortcut_failure_toasts(&ok, &palette_only);
    assert_eq!(toasts.len(), 1);
    assert_eq!(toasts[0].0, ShortcutToastKind::CommandPalette);
}

// SDTEST-1418 — absent desktop capability stays in Settings rather than
// producing duplicate launch toasts; actionable failures still surface.
#[test]
fn portal_absence_stays_in_settings_but_other_errors_pass_through() {
    let ok = statuses(
        ShortcutRegistrationStatus::Registered,
        ShortcutRegistrationStatus::Registered,
    );
    let portal_missing = statuses(
        ShortcutRegistrationStatus::Error(
            "A portal frontend implementing `org.freedesktop.portal.GlobalShortcuts` \
                 was not found"
                .into(),
        ),
        ShortcutRegistrationStatus::Registered,
    );

    let toasts = shortcut_failure_toasts(&ok, &portal_missing);
    assert!(toasts.is_empty());
    let both_missing = statuses(
        portal_missing.ai_dock.clone(),
        portal_missing.ai_dock.clone(),
    );
    assert!(shortcut_failure_toasts(&ok, &both_missing).is_empty());
    let mixed = statuses(
        portal_missing.ai_dock.clone(),
        ShortcutRegistrationStatus::Conflict,
    );
    let actionable = shortcut_failure_toasts(&ok, &mixed);
    assert_eq!(actionable.len(), 1);
    assert_eq!(actionable[0].0, ShortcutToastKind::CommandPalette);

    let other = statuses(
        ShortcutRegistrationStatus::Error("Could not resolve keycode for key: nosuchkey".into()),
        ShortcutRegistrationStatus::Registered,
    );
    let toasts = shortcut_failure_toasts(&ok, &other);
    assert!(toasts[0].1.contains("Could not resolve keycode"));
}
