use super::*;

/// Written from the `Status` and `PeerStatus` structures of the Tailscale
/// source (`ipn/ipnstate/ipnstate.go`); every name and address is invented.
const FIXTURE: &str = include_str!("../tests/fixtures/tailscale-status.json");

fn fixture_peers() -> Vec<TailscalePeer> {
    parse_status(FIXTURE).expect("the fixture is a status listing")
}

fn peer<'a>(peers: &'a [TailscalePeer], host_name: &str) -> &'a TailscalePeer {
    peers
        .iter()
        .find(|peer| peer.host_name == host_name)
        .unwrap_or_else(|| panic!("{host_name} is listed"))
}

fn connection_to(hostname: &str) -> Connection {
    Connection::new_manual("saved".into(), hostname.into(), "deploy".into())
}

// SDTEST-3156 (SDUC-585)
#[test]
fn sdtest_3156_a_status_listing_gives_each_machine_its_name_addresses_and_state() {
    let peers = fixture_peers();

    let build = peer(&peers, "build-box");
    assert_eq!(build.dns_name, "build-box.tail1234.ts.net");
    assert_eq!(build.addresses, ["100.64.0.2", "fd7a:115c:a1e0::2"]);
    assert_eq!(build.os, "linux");
    assert!(build.online);
    assert!(build.ssh);

    let laptop = peer(&peers, "Ana's MacBook Pro");
    assert_eq!(laptop.dns_name, "anas-macbook-pro.tail1234.ts.net");
    assert_eq!(laptop.os, "macOS");
    assert!(!laptop.online);
    assert!(!laptop.ssh);

    let windows = peer(&peers, "Atelier-PC");
    assert_eq!(windows.dns_name, "atelier-pc.tail1234.ts.net");
    assert!(windows.online);
    assert!(!windows.ssh);
}

// SDTEST-3157 (SDUC-585)
#[test]
fn sdtest_3157_this_machine_exit_nodes_and_undialable_machines_are_left_out() {
    let peers = fixture_peers();
    let mut names: Vec<&str> = peers.iter().map(|peer| peer.host_name.as_str()).collect();
    names.sort_unstable();
    // Not listed: "workstation" (this machine), the Mullvad exit node, and
    // "ghost", which has neither a name to resolve nor an address.
    assert_eq!(
        names,
        ["Ana's MacBook Pro", "Atelier-PC", "build-box", "nas"]
    );

    // A machine without a name is dialled by address.
    let nas = peer(&peers, "nas");
    assert_eq!(nas.dns_name, "");
    assert_eq!(nas.dial_host(), "100.64.0.6");

    // This machine repeated among the peers is still left out.
    let repeated = r#"{"BackendState":"Running",
        "Self":{"ID":"n1","HostName":"here","DNSName":"here.t.ts.net."},
        "Peer":{"a":{"ID":"n1","HostName":"here","DNSName":"here.t.ts.net."},
                "b":{"ID":"n2","HostName":"there","DNSName":"there.t.ts.net."}}}"#;
    let peers = parse_status(repeated).unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].host_name, "there");
}

// SDTEST-3158 (SDUC-585)
#[test]
fn sdtest_3158_a_listing_without_machines_says_why() {
    for state in ["NeedsLogin", "NeedsMachineAuth", "NoState"] {
        let json = format!(r#"{{"BackendState":"{state}","Self":null,"Peer":null}}"#);
        assert_eq!(
            parse_status(&json),
            Err(TailscaleUnavailable::NeedsLogin),
            "{state}"
        );
    }
    for state in ["Stopped", "Starting"] {
        let json = format!(r#"{{"BackendState":"{state}","Peer":{{}}}}"#);
        assert_eq!(
            parse_status(&json),
            Err(TailscaleUnavailable::Stopped),
            "{state}"
        );
    }
    for unreadable in [
        "",
        "Logged out.",
        "[]",
        "{}",
        r#"{"BackendState":"SomethingNew"}"#,
        r#"{"BackendState":"Running","Peer":"none"}"#,
    ] {
        assert_eq!(
            parse_status(unreadable),
            Err(TailscaleUnavailable::Unreadable),
            "{unreadable:?}"
        );
    }
    // Signed in and alone on the network: a list, and an empty one.
    assert_eq!(
        parse_status(r#"{"BackendState":"Running","Peer":null}"#),
        Ok(Vec::new())
    );
    // Fields this version does not know are ignored, missing ones too.
    let sparse = r#"{"BackendState":"Running","Future":1,
        "Peer":{"k":{"DNSName":"solo.t.ts.net.","Future":[1,2]}}}"#;
    let peers = parse_status(sparse).unwrap();
    assert_eq!(peers[0].dns_name, "solo.t.ts.net");
    assert_eq!(peers[0].display_name(), "solo");
    assert!(!peers[0].online);
}

// SDTEST-3159 (SDUC-585)
#[test]
fn sdtest_3159_the_list_shows_online_machines_first_then_by_name() {
    let order = |connections: &[Connection]| -> Vec<(String, bool)> {
        rows(fixture_peers(), connections)
            .into_iter()
            .map(|row| (row.peer.host_name, row.existing))
            .collect()
    };
    assert_eq!(
        order(&[]),
        [
            ("Atelier-PC".to_string(), false),
            ("build-box".to_string(), false),
            ("Ana's MacBook Pro".to_string(), false),
            ("nas".to_string(), false),
        ]
    );
    // The order does not depend on the listing's, which is a map.
    let mut reversed = fixture_peers();
    reversed.reverse();
    let names = |rows: Vec<TailscaleRow>| -> Vec<String> {
        rows.into_iter().map(|row| row.peer.host_name).collect()
    };
    assert_eq!(
        names(rows(reversed, &[])),
        names(rows(fixture_peers(), &[]))
    );
}

// SDTEST-3160 (SDUC-585)
#[test]
fn sdtest_3160_a_machine_that_already_has_a_connection_is_marked() {
    let marked = |hostname: &str| -> Vec<String> {
        rows(fixture_peers(), &[connection_to(hostname)])
            .into_iter()
            .filter(|row| row.existing)
            .map(|row| row.peer.host_name)
            .collect()
    };
    // By MagicDNS name, whatever the case and with or without the root dot.
    assert_eq!(marked("build-box.tail1234.ts.net"), ["build-box"]);
    assert_eq!(marked(" Build-Box.Tail1234.TS.net. "), ["build-box"]);
    // By the short name MagicDNS resolves, and by either address.
    assert_eq!(marked("anas-macbook-pro"), ["Ana's MacBook Pro"]);
    assert_eq!(marked("100.64.0.4"), ["Atelier-PC"]);
    assert_eq!(marked("fd7a:115c:a1e0::2"), ["build-box"]);
    assert_eq!(marked("100.64.0.6"), ["nas"]);
    // Another machine, a name that only starts alike, or no host at all.
    assert!(marked("build-box.example.com").is_empty());
    assert!(marked("build").is_empty());
    assert!(marked("").is_empty());
    // A machine without a MagicDNS name is not matched by an empty label.
    assert!(marked("nas").is_empty());
}

// SDTEST-3161 (SDUC-585)
#[test]
fn sdtest_3161_a_machine_becomes_a_normal_manual_connection() {
    let peers = fixture_peers();
    let connection = connection_from_peer(peer(&peers, "build-box"), "deploy");
    assert_eq!(connection.alias, "build-box");
    assert_eq!(connection.hostname, "build-box.tail1234.ts.net");
    assert_eq!(connection.user, "deploy");
    assert_eq!(connection.port, 22);
    assert_eq!(
        connection.source,
        crate::models::connection::ConnectionSource::Manual
    );
    assert!(connection.identity_file.is_none());
    assert!(connection.proxy_jump.is_none());
    assert!(connection.tags.is_empty());

    // Without MagicDNS the address is what gets dialled.
    let nas = connection_from_peer(peer(&peers, "nas"), "admin");
    assert_eq!(nas.alias, "nas");
    assert_eq!(nas.hostname, "100.64.0.6");

    // Once created, the machine is marked in the list.
    let again = rows(peers, &[connection]);
    assert!(again
        .iter()
        .any(|row| row.existing && row.peer.host_name == "build-box"));
}

// SDTEST-3162 (SDUC-585)
#[test]
fn sdtest_3162_the_program_is_looked_for_in_the_given_directories_only() {
    let empty = tempfile::tempdir().unwrap();
    let with = tempfile::tempdir().unwrap();
    let name = if cfg!(windows) {
        "tailscale.exe"
    } else {
        "tailscale"
    };
    assert_eq!(find_program([empty.path().to_path_buf()]), None);
    assert_eq!(find_program(Vec::new()), None);

    // A directory of that name is not the program.
    std::fs::create_dir(empty.path().join(name)).unwrap();
    assert_eq!(find_program([empty.path().to_path_buf()]), None);

    std::fs::write(with.path().join(name), "").unwrap();
    assert_eq!(
        find_program([empty.path().to_path_buf(), with.path().to_path_buf()]),
        Some(with.path().join(name))
    );

    // A program that went away between the search and the start.
    assert_eq!(
        discover_with(&empty.path().join("gone"), Duration::from_secs(1)),
        Err(TailscaleUnavailable::NotInstalled)
    );
}

// SDTEST-3170 (SDUC-585)
#[test]
fn sdtest_3170_names_that_do_not_resolve_and_sharee_machines_are_not_offered() {
    // Peers keep their `DNSName` when MagicDNS is turned off for the
    // network: the name must not become the host of a connection.
    let listing = |tailnet: &str| {
        format!(
            r#"{{"BackendState":"Running",{tailnet}
            "Peer":{{"a":{{"ID":"n2","HostName":"build-box",
                "DNSName":"build-box.tail1234.ts.net.",
                "TailscaleIPs":["100.64.0.2","fd7a:115c:a1e0::2"]}},
              "b":{{"ID":"n3","HostName":"no-address",
                "DNSName":"no-address.tail1234.ts.net."}},
              "c":{{"ID":"n4","HostName":"theirs","ShareeNode":true,
                "DNSName":"theirs.other.ts.net.","TailscaleIPs":["100.64.0.4"]}}}}}}"#
        )
    };
    let off = parse_status(&listing(
        r#""CurrentTailnet":{"Name":"example.com","MagicDNSSuffix":"tail1234.ts.net","MagicDNSEnabled":false},"#,
    ))
    .unwrap();
    // "no-address" has nothing left to dial; "theirs" is a sharee machine.
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].dns_name, "");
    assert_eq!(off[0].dial_host(), "100.64.0.2");
    let connection = connection_from_peer(&off[0], "deploy");
    assert_eq!(connection.alias, "build-box");
    assert_eq!(connection.hostname, "100.64.0.2");
    // The mark follows what resolves: the address, not the name.
    let marked = |hostname: &str| rows(off.clone(), &[connection_to(hostname)])[0].existing;
    assert!(marked("100.64.0.2"));
    assert!(!marked("build-box"));

    // MagicDNS on, or a listing that does not say: the name is the host.
    for tailnet in [
        r#""CurrentTailnet":{"MagicDNSEnabled":true},"#,
        r#""CurrentTailnet":null,"#,
        r#""CurrentTailnet":{},"#,
        "",
    ] {
        let mut on = parse_status(&listing(tailnet)).unwrap();
        on.sort_by(|a, b| a.host_name.cmp(&b.host_name));
        let hosts: Vec<&str> = on.iter().map(|peer| peer.dial_host()).collect();
        assert_eq!(
            hosts,
            ["build-box.tail1234.ts.net", "no-address.tail1234.ts.net"],
            "{tailnet:?}"
        );
    }
}

#[cfg(unix)]
mod with_a_program {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    /// A stand-in for the `tailscale` program: a shell script in a
    /// temporary directory. Nothing of the real one is run.
    fn program(directory: &Path, body: &str) -> PathBuf {
        let path = directory.join("tailscale");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Another test forking while the file was open for writing keeps
        // it "busy" for a moment; wait until it can be started.
        for _ in 0..200 {
            match Command::new(&path).arg("probe").output() {
                Err(error) if error.raw_os_error() == Some(26) => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                _ => break,
            }
        }
        path
    }

    const ARGUMENTS: &str = r#"[ "$1" = probe ] && exit 0
[ "$1" = status ] && [ "$2" = --json ] && [ $# -eq 2 ] || exit 64"#;

    // SDTEST-3163 (SDUC-585)
    #[test]
    fn sdtest_3163_the_local_program_is_asked_for_its_status_as_json() {
        let directory = tempfile::tempdir().unwrap();
        let fixture = directory.path().join("status.json");
        std::fs::write(&fixture, FIXTURE).unwrap();
        let path = program(
            directory.path(),
            &format!("{ARGUMENTS}\ncat '{}'", fixture.display()),
        );
        let peers = discover_with(&path, Duration::from_secs(10)).unwrap();
        assert_eq!(peers.len(), 4);
        assert!(peers.iter().any(|peer| peer.host_name == "build-box"));
    }

    // SDTEST-3164 (SDUC-585)
    #[test]
    fn sdtest_3164_a_program_that_fails_or_answers_something_else_is_a_reason_not_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let cases = [
            // No daemon: a message on the error output, a failure status.
            (
                "echo 'failed to connect to local tailscaled' >&2; exit 1",
                TailscaleUnavailable::NotRunning,
            ),
            ("exit 0", TailscaleUnavailable::Unreadable),
            (
                "echo 'Logged out.'; exit 1",
                TailscaleUnavailable::Unreadable,
            ),
            (
                r#"echo '{"BackendState":"NeedsLogin","Peer":null}'"#,
                TailscaleUnavailable::NeedsLogin,
            ),
            (
                r#"echo '{"BackendState":"Stopped","Peer":null}'"#,
                TailscaleUnavailable::Stopped,
            ),
            // Not text at all.
            (r"printf '\377\376\375'", TailscaleUnavailable::Unreadable),
        ];
        for (body, expected) in cases {
            let path = program(directory.path(), &format!("{ARGUMENTS}\n{body}"));
            assert_eq!(
                discover_with(&path, Duration::from_secs(10)),
                Err(expected),
                "{body}"
            );
        }
    }

    // SDTEST-3165 (SDUC-585)
    #[test]
    fn sdtest_3165_a_program_that_does_not_answer_is_stopped_after_the_timeout() {
        let directory = tempfile::tempdir().unwrap();
        // `exec` so that the kill reaches the sleeper itself.
        let path = program(directory.path(), &format!("{ARGUMENTS}\nexec sleep 30"));
        let start = Instant::now();
        assert_eq!(
            discover_with(&path, Duration::from_millis(300)),
            Err(TailscaleUnavailable::TimedOut)
        );
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );

        // A child that outlives the program and keeps its output open does
        // not hold the caller either.
        let path = program(directory.path(), &format!("{ARGUMENTS}\nsleep 3 &\nexit 1"));
        let start = Instant::now();
        assert_eq!(
            discover_with(&path, Duration::from_millis(300)),
            Err(TailscaleUnavailable::TimedOut)
        );
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
    }

    // SDTEST-3166 (SDUC-585)
    #[test]
    fn sdtest_3166_a_listing_larger_than_a_pipe_buffer_is_read_whole() {
        let directory = tempfile::tempdir().unwrap();
        let mut listing = String::from(r#"{"BackendState":"Running","Peer":{"#);
        for index in 0..3000 {
            if index > 0 {
                listing.push(',');
            }
            listing.push_str(&format!(
                r#""k{index}":{{"ID":"n{index}","HostName":"node-{index}","DNSName":"node-{index}.t.ts.net.","TailscaleIPs":["100.64.{}.{}"],"Online":true}}"#,
                index / 250,
                index % 250
            ));
        }
        listing.push_str("}}");
        assert!(listing.len() > 256 * 1024);
        let file = directory.path().join("status.json");
        std::fs::write(&file, &listing).unwrap();
        let path = program(
            directory.path(),
            &format!("{ARGUMENTS}\ncat '{}'", file.display()),
        );
        let peers = discover_with(&path, Duration::from_secs(10)).unwrap();
        assert_eq!(peers.len(), 3000);
    }
}
