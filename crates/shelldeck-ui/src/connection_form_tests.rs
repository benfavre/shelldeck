use super::*;
use gpui::TestAppContext;
use std::cell::RefCell;
use std::rc::Rc;

fn machine(name: &str, online: bool) -> TailscalePeer {
    TailscalePeer {
        host_name: name.to_string(),
        dns_name: format!("{name}.tail1234.ts.net"),
        addresses: vec!["100.64.0.9".to_string()],
        os: "linux".to_string(),
        online,
        ssh: false,
    }
}

fn listed(form: &ConnectionForm) -> Vec<(String, bool)> {
    match &form.tailscale {
        TailscalePanel::Machines(rows) => rows
            .iter()
            .map(|row| (row.peer.host_name.clone(), row.existing))
            .collect(),
        _ => panic!("the list of machines is not shown"),
    }
}

// SDTEST-3167 (SDUC-585): the answers are given by hand, the local
// `tailscale` program is never run.
#[::core::prelude::v1::test]
fn sdtest_3167_the_tailscale_list_shows_the_machines_or_the_reason() {
    let mut app = TestAppContext::single();
    let form = app.update(|cx| {
        cx.new(|cx| {
            let mut form = ConnectionForm::new(cx);
            form.set_existing_connections(vec![Connection::new_manual(
                "saved".into(),
                "beta.tail1234.ts.net".into(),
                "deploy".into(),
            )]);
            form
        })
    });
    form.update(&mut app, |form, cx| {
        assert!(matches!(form.tailscale, TailscalePanel::Closed));
        let request = form.begin_tailscale_lookup();
        assert!(matches!(form.tailscale, TailscalePanel::Loading));
        form.show_tailscale_result(
            request,
            Ok(vec![
                machine("offline", false),
                machine("beta", true),
                machine("alpha", true),
            ]),
            cx,
        );
        assert_eq!(
            listed(form),
            [
                ("alpha".to_string(), false),
                ("beta".to_string(), true),
                ("offline".to_string(), false),
            ]
        );

        // Asked again: the answer to the first question is dropped.
        let second = form.begin_tailscale_lookup();
        form.show_tailscale_result(request, Ok(vec![machine("late", true)]), cx);
        assert!(matches!(form.tailscale, TailscalePanel::Loading));
        form.show_tailscale_result(second, Err(TailscaleUnavailable::NotInstalled), cx);
        assert!(matches!(
            form.tailscale,
            TailscalePanel::Unavailable(TailscaleUnavailable::NotInstalled)
        ));

        // Closed while the program runs: its answer does not reopen it.
        let third = form.begin_tailscale_lookup();
        form.toggle_tailscale(cx);
        form.show_tailscale_result(third, Ok(vec![machine("late", true)]), cx);
        assert!(matches!(form.tailscale, TailscalePanel::Closed));
    });
}

// SDTEST-3168 (SDUC-585)
#[::core::prelude::v1::test]
fn sdtest_3168_choosing_a_tailscale_machine_saves_a_normal_connection() {
    let mut app = TestAppContext::single();
    let form = app.update(|cx| {
        cx.new(|cx| {
            let mut form = ConnectionForm::new(cx);
            form.set_existing_connections(vec![Connection::new_manual(
                "saved".into(),
                "beta".into(),
                "deploy".into(),
            )]);
            form
        })
    });
    let saved: Rc<RefCell<Vec<Connection>>> = Rc::default();
    let _subscription = app.update(|cx| {
        let saved = saved.clone();
        cx.subscribe(&form, move |_, event: &ConnectionFormEvent, _| {
            if let ConnectionFormEvent::Save(connection) = event {
                saved.borrow_mut().push(connection.clone());
            }
        })
    });

    form.update(&mut app, |form, cx| {
        // Nothing listed yet: nothing to choose.
        form.create_from_tailscale(0, cx);
        let request = form.begin_tailscale_lookup();
        form.create_from_tailscale(0, cx);
        form.show_tailscale_result(
            request,
            Ok(vec![machine("beta", true), machine("alpha", true)]),
            cx,
        );
        // "beta" already has a connection, and there is no third row.
        form.create_from_tailscale(1, cx);
        form.create_from_tailscale(2, cx);
    });
    assert!(saved.borrow().is_empty());

    form.update(&mut app, |form, cx| {
        form.user_state.update(cx, |state, _| {
            state.content = "deploy".into();
        });
        form.create_from_tailscale(0, cx);
    });
    {
        let saved = saved.borrow();
        assert_eq!(saved.len(), 1);
        let connection = &saved[0];
        assert_eq!(connection.alias, "alpha");
        assert_eq!(connection.hostname, "alpha.tail1234.ts.net");
        assert_eq!(connection.user, "deploy");
        assert_eq!(connection.port, 22);
        assert_eq!(
            connection.source,
            shelldeck_core::models::connection::ConnectionSource::Manual
        );
    }

    // An emptied user field falls back to a name, never to nothing.
    form.update(&mut app, |form, cx| {
        form.user_state.update(cx, |state, _| {
            state.content = "  ".into();
        });
        form.create_from_tailscale(0, cx);
    });
    assert!(!saved.borrow()[1].user.trim().is_empty());
}

// SDTEST-3169 (SDUC-585)
#[::core::prelude::v1::test]
fn sdtest_3169_every_tailscale_reason_has_its_own_sentence_in_both_languages() {
    let reasons = [
        TailscaleUnavailable::NotInstalled,
        TailscaleUnavailable::NotRunning,
        TailscaleUnavailable::NeedsLogin,
        TailscaleUnavailable::Stopped,
        TailscaleUnavailable::TimedOut,
        TailscaleUnavailable::Unreadable,
    ];
    let other = [
        "connection_form.tailscale.open",
        "connection_form.tailscale.loading",
        "connection_form.tailscale.hint",
        "connection_form.tailscale.empty",
        "connection_form.tailscale.offline",
        "connection_form.tailscale.existing",
        "connection_form.tailscale.ssh",
    ];
    for locale in ["fr", "en"] {
        let mut seen = std::collections::HashSet::new();
        let keys = reasons
            .iter()
            .map(|reason| tailscale_unavailable_key(*reason))
            .chain(other);
        for key in keys {
            let text = t!(key, locale = locale).to_string();
            assert!(!text.contains("connection_form."), "{locale}: {key}");
            assert!(seen.insert(text), "{locale}: {key} repeats a sentence");
        }
    }
}
