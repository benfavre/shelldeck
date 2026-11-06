use super::*;
use crate::theme::ShellDeckColors;
use shelldeck_core::config::app_config::ThemePreference;

fn snapshot() -> ShareSnapshot {
    serde_json::from_value(serde_json::json!({
        "owner":{"email":"preview@example.invalid","name":"Preview"},
        "plan":{"id":"free","label":"Gratuit","max_file_bytes":104857600,"storage_bytes":1073741824,"max_files":1000,"retention_days":[1,7,30],"residency":"global","storage_ready":true},
        "usage":{"bytes":1234,"files":1},"default_retention_seconds":604800,"list_limit":500,
        "files":[{"id":"abcdef","filename":"Rapport de visite.pdf","bytes":1234,"content_type":"application/pdf","kind":"pdf","created_at":"2026-11-06T12:00:00Z","expires_at":null,"views":0,"url":"https://share.inklura.fr/r/abcdef.pdf","viewer_url":"https://share.inklura.fr/s/abcdef"}]
    })).unwrap()
}
fn redraw(window: &mut VisualTestContext) {
    window.run_until_parked();
    for _ in 0..2 {
        window.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear();
        });
        window.run_until_parked();
    }
}
fn click(window: &mut VisualTestContext, selector: &str) {
    let bounds = window
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing {selector}"));
    let viewport = window.update(|window, _| window.viewport_size());
    assert!(
        Bounds::new(point(gpui::px(0.0), gpui::px(0.0)), viewport).contains(&bounds.center()),
        "Clipped {selector}: {bounds:?}"
    );
    window.simulate_mouse_move(bounds.center(), None, Modifiers::none());
    window.simulate_click(bounds.center(), Modifiers::none());
    redraw(window);
}
// SDTEST-2893 / SDUC-568 — production controls, layout, clipboard and stale-job fence.
#[::core::prelude::v1::test]
fn sdtest_2893_small_share_batches_keep_actions_reachable_and_account_switch_rejects_old_results() {
    for (width, height) in [(480.0, 420.0), (620.0, 560.0), (1100.0, 800.0)] {
        let mut app = TestAppContext::single();
        app.update(|cx| {
            adabraka_ui::init(cx);
            adabraka_ui::components::input::init(cx);
            ShellDeckColors::set_theme(&ThemePreference::Dark);
            adabraka_ui::theme::install_theme(cx, crate::theme::adabraka_theme_from_palette());
        });
        let handle = app.add_window(|_, cx| {
            ShareView::preview(
                snapshot(),
                (0..20)
                    .map(|n| PathBuf::from(format!("a-very-long-local-file-to-share-{n}.pdf")))
                    .collect(),
                cx,
            )
        });
        let view = handle.root(&mut app).unwrap();
        let mut window = VisualTestContext::from_window(*handle, &app);
        window.simulate_resize(size(gpui::px(width), gpui::px(height)));
        redraw(&mut window);
        let drafts = window.debug_bounds("share-drafts").unwrap();
        assert!(drafts.size.height <= gpui::px(100.0));
        let intake = window.debug_bounds("share-intake").unwrap();
        let send = window.debug_bounds("share-send").unwrap();
        assert!(
            send.origin.y >= intake.bottom(),
            "Send stays outside the scrolling intake"
        );
        let viewport = window.update(|window, _| window.viewport_size());
        assert!(
            send.bottom() <= viewport.height,
            "Send is visible without scrolling"
        );
        click(&mut window, "share-review");
        click(&mut window, "share-review-remove-0");
        assert!(view.read_with(&app, |view, _| view.drafts.len() == 19
            && view.drafts[0].upload.filename.ends_with("-1.pdf")));
        click(&mut window, "share-review-close");
        assert!(!view.read_with(&app, |view, _| view.review_drafts));
        click(&mut window, "share-copy-abcdef");
        assert_eq!(
            app.read_from_clipboard().unwrap().text().unwrap(),
            "https://share.inklura.fr/s/abcdef"
        );
        click(&mut window, "share-delete-abcdef");
        assert!(view.read_with(&app, |view, _| view.confirm_delete.is_some()));
        click(&mut window, "share-delete-cancel");
        assert!(view.read_with(&app, |view, _| view.confirm_delete.is_none()
            && view.snapshot.as_ref().unwrap().files.len() == 1));
        let cancelled = view.read_with(&app, |view, _| view.cancelled.clone());
        view.update(&mut app, |view, cx| {
            view.manage = "https://manage.inklura.fr".into();
            view.device = "synthetic".into();
            view.run(|_| Ok(Outcome::snapshot(snapshot())), cx);
            view.configure(
                "https://manage.inklura.fr".into(),
                "different-synthetic-device".into(),
                cx,
            );
        });
        redraw(&mut window);
        assert!(cancelled.load(Ordering::Acquire));
        assert!(view.read_with(&app, |view, _| view.snapshot.is_none()
            && view.drafts.is_empty()
            && !view.busy));
        app.quit();
    }
}

// SDTEST-2896 / SDUC-568 — real Stop control, partial completion and retry state.
#[::core::prelude::v1::test]
fn sdtest_2896_stop_preserves_unshared_files_and_partial_success_restores_a_usable_session() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        adabraka_ui::init(cx);
        adabraka_ui::components::input::init(cx);
        ShellDeckColors::set_theme(&ThemePreference::Dark);
        adabraka_ui::theme::install_theme(cx, crate::theme::adabraka_theme_from_palette());
    });
    let handle = app.add_window(|_, cx| {
        ShareView::preview(
            snapshot(),
            vec!["first.pdf".into(), "second.pdf".into(), "third.pdf".into()],
            cx,
        )
    });
    let view = handle.root(&mut app).unwrap();
    let mut window = VisualTestContext::from_window(*handle, &app);
    window.simulate_resize(size(gpui::px(480.0), gpui::px(420.0)));
    let progress = Arc::new(UploadProgress::default());
    progress.begin(1);
    progress.update(8 * 1024 * 1024, 16 * 1024 * 1024);
    let cancellation = view.read_with(&app, |view, _| view.cancelled.clone());
    view.update(&mut app, |view, cx| {
        view.busy = true;
        view.upload_progress = Some(progress);
        cx.notify();
    });
    redraw(&mut window);
    assert!(window.debug_bounds("share-transfer-progress").is_some());
    click(&mut window, "share-stop");
    assert!(cancellation.load(Ordering::Acquire));
    assert!(view.read_with(&app, |view, _| view.drafts.len() == 3 && view.busy));
    // One initiated upload completed; the remaining files must survive Stop.
    view.update(&mut app, |view, cx| {
        view.busy = false;
        view.manage = "https://manage.inklura.fr".into();
        view.device = "synthetic".into();
        view.run(
            |_| {
                Ok(Outcome {
                    snapshot: None,
                    uploads: vec![SharedUpload {
                        id: "fedcba".into(),
                        url: "https://share.inklura.fr/r/fedcba.pdf".into(),
                        viewer: "https://share.inklura.fr/s/fedcba".into(),
                    }],
                    completed: 1,
                    warning: Some(ShareError::Cancelled),
                })
            },
            cx,
        );
    });
    redraw(&mut window);
    assert_eq!(
        app.read_from_clipboard().unwrap().text().unwrap(),
        "https://share.inklura.fr/s/fedcba"
    );
    assert!(view.read_with(&app, |view, _| view
        .drafts
        .iter()
        .map(|draft| draft.upload.filename.as_str())
        .collect::<Vec<_>>()
        == vec!["second.pdf", "third.pdf"]
        && !view.busy
        && view.upload_progress.is_none()
        && !view.cancelled.load(Ordering::Acquire)
        && view.device == "synthetic"));
    click(&mut window, "share-review");
    click(&mut window, "share-review-clear");
    assert!(view.read_with(&app, |view, _| view.drafts.is_empty()
        && !view.review_drafts));
    app.quit();
}
