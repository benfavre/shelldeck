//! Machines of the user's Tailscale network, read from the local
//! `tailscale status --json`. Nothing is sent anywhere: the only thing run
//! is the local binary, which talks to the local daemon.
//!
//! Field names follow `ipn/ipnstate/ipnstate.go` (`Status`, `PeerStatus`)
//! of the Tailscale source, as of 2026-11.

use crate::models::connection::Connection;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long the local binary gets to answer. It only asks the local daemon,
/// so a slow answer means a stuck daemon, not a slow network.
pub const STATUS_TIMEOUT: Duration = Duration::from_secs(3);

/// A status listing is a few kilobytes per machine; anything beyond this is
/// not one.
const STATUS_LIMIT: u64 = 16 * 1024 * 1024;

/// Nodes of the Mullvad add-on are exit nodes, not machines to log in to.
const MULLVAD_TAG: &str = "tag:mullvad-exit-node";

/// One other machine of the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailscalePeer {
    /// The name the machine gave itself. Not necessarily unique.
    pub host_name: String,
    /// MagicDNS name without the final dot; empty when the network has none.
    pub dns_name: String,
    pub addresses: Vec<String>,
    pub os: String,
    pub online: bool,
    /// The machine offers Tailscale SSH.
    pub ssh: bool,
}

impl TailscalePeer {
    /// What an SSH connection dials: the MagicDNS name, or the first
    /// address on a network without MagicDNS.
    pub fn dial_host(&self) -> &str {
        if self.dns_name.is_empty() {
            self.addresses.first().map(String::as_str).unwrap_or("")
        } else {
            &self.dns_name
        }
    }

    /// The name shown in the list and given to the connection.
    pub fn display_name(&self) -> &str {
        if !self.host_name.is_empty() {
            &self.host_name
        } else if !self.dns_name.is_empty() {
            first_label(&self.dns_name)
        } else {
            self.dial_host()
        }
    }
}

/// Why there is no list of machines to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailscaleUnavailable {
    /// No `tailscale` program on this machine.
    NotInstalled,
    /// The program is there but its daemon did not answer.
    NotRunning,
    /// The daemon runs but this machine is not signed in to a network.
    NeedsLogin,
    /// Signed in, but Tailscale is turned off.
    Stopped,
    /// The program did not answer in time.
    TimedOut,
    /// The answer was not a status listing.
    Unreadable,
}

/// A peer as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailscaleRow {
    pub peer: TailscalePeer,
    /// A connection to this machine already exists.
    pub existing: bool,
}

#[derive(Deserialize)]
struct RawStatus {
    #[serde(rename = "BackendState")]
    backend_state: Option<String>,
    #[serde(rename = "CurrentTailnet")]
    tailnet: Option<RawTailnet>,
    #[serde(rename = "Self")]
    this_machine: Option<RawPeer>,
    #[serde(rename = "Peer")]
    peers: Option<HashMap<String, RawPeer>>,
}

#[derive(Deserialize)]
struct RawTailnet {
    #[serde(rename = "MagicDNSEnabled")]
    magic_dns: Option<bool>,
}

#[derive(Deserialize)]
struct RawPeer {
    #[serde(rename = "ID", default)]
    id: String,
    #[serde(rename = "HostName", default)]
    host_name: String,
    #[serde(rename = "DNSName", default)]
    dns_name: String,
    #[serde(rename = "OS", default)]
    os: String,
    #[serde(rename = "TailscaleIPs", default)]
    addresses: Option<Vec<String>>,
    #[serde(rename = "Tags", default)]
    tags: Option<Vec<String>>,
    #[serde(rename = "Online", default)]
    online: bool,
    #[serde(rename = "sshHostKeys", default)]
    ssh_host_keys: Option<Vec<String>>,
    #[serde(rename = "ShareeNode", default)]
    sharee: bool,
}

/// Reads the output of `tailscale status --json`. This machine is left out,
/// and so is a peer with nothing to dial. On a network with MagicDNS turned
/// off the names do not resolve, so the peers come without one and are
/// dialled by address. The order is the listing's, which
/// is a map: sort with [`rows`] before showing.
pub fn parse_status(json: &str) -> Result<Vec<TailscalePeer>, TailscaleUnavailable> {
    let status: RawStatus =
        serde_json::from_str(json).map_err(|_| TailscaleUnavailable::Unreadable)?;
    match status.backend_state.as_deref() {
        Some("Running") => {}
        Some("NeedsLogin" | "NeedsMachineAuth" | "NoState") => {
            return Err(TailscaleUnavailable::NeedsLogin)
        }
        Some("Stopped" | "Starting") => return Err(TailscaleUnavailable::Stopped),
        _ => return Err(TailscaleUnavailable::Unreadable),
    }
    // Every peer has a `DNSName` whether or not MagicDNS is on; only this
    // says the name resolves. A listing too old to say so is trusted.
    let magic_dns = status
        .tailnet
        .and_then(|tailnet| tailnet.magic_dns)
        .unwrap_or(true);
    let this_id = status
        .this_machine
        .map(|machine| machine.id)
        .unwrap_or_default();
    let peers = status
        .peers
        .unwrap_or_default()
        .into_values()
        // The listing keeps this machine apart, but a shared node list is
        // not ours to trust on that.
        .filter(|peer| this_id.is_empty() || peer.id != this_id)
        // Machines of the people this one is shared with: `tailscale
        // status` hides them too.
        .filter(|peer| !peer.sharee)
        .filter(|peer| {
            !peer
                .tags
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(|tag| tag == MULLVAD_TAG)
        })
        .map(|peer| TailscalePeer {
            host_name: peer.host_name.trim().to_string(),
            dns_name: if magic_dns {
                normalize_host(&peer.dns_name)
            } else {
                String::new()
            },
            addresses: peer.addresses.unwrap_or_default(),
            os: peer.os,
            online: peer.online,
            ssh: peer.ssh_host_keys.is_some_and(|keys| !keys.is_empty()),
        })
        .filter(|peer| !peer.dial_host().is_empty())
        .collect();
    Ok(peers)
}

/// The peers in the order of the list: online first, then by name, each
/// marked when one of `connections` already reaches it.
pub fn rows(peers: Vec<TailscalePeer>, connections: &[Connection]) -> Vec<TailscaleRow> {
    let known: Vec<String> = connections
        .iter()
        .map(|connection| normalize_host(&connection.hostname))
        .collect();
    let mut rows: Vec<TailscaleRow> = peers
        .into_iter()
        .map(|peer| {
            let existing = known.iter().any(|host| reaches(&peer, host));
            TailscaleRow { peer, existing }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.peer
            .online
            .cmp(&a.peer.online)
            .then_with(|| {
                a.peer
                    .display_name()
                    .to_lowercase()
                    .cmp(&b.peer.display_name().to_lowercase())
            })
            .then_with(|| a.peer.dns_name.cmp(&b.peer.dns_name))
    });
    rows
}

/// A normal manual SSH connection to the peer, on port 22.
pub fn connection_from_peer(peer: &TailscalePeer, user: &str) -> Connection {
    Connection::new_manual(
        peer.display_name().to_string(),
        peer.dial_host().to_string(),
        user.to_string(),
    )
}

/// Asks the local Tailscale for its machines. Blocks for at most
/// [`STATUS_TIMEOUT`]: call it off the UI thread.
pub fn discover() -> Result<Vec<TailscalePeer>, TailscaleUnavailable> {
    let Some(program) = find_program(search_directories()) else {
        return Err(TailscaleUnavailable::NotInstalled);
    };
    discover_with(&program, STATUS_TIMEOUT)
}

/// [`discover`] with the program and the patience given.
pub fn discover_with(
    program: &Path,
    timeout: Duration,
) -> Result<Vec<TailscalePeer>, TailscaleUnavailable> {
    let mut command = Command::new(program);
    command
        .args(["status", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(TailscaleUnavailable::NotInstalled)
        }
        Err(error) => {
            tracing::warn!("tailscale status could not be started: {error}");
            return Err(TailscaleUnavailable::NotRunning);
        }
    };

    // The listing of a large network is bigger than a pipe buffer, so it
    // is read while the program runs. The reader is never joined: after a
    // kill it ends by itself when the pipe closes.
    let (sender, receiver) = std::sync::mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut output = String::new();
            let read = stdout.take(STATUS_LIMIT).read_to_string(&mut output);
            let _ = sender.send(read.map(|_| output));
        });
    }

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() >= timeout => {
                tracing::warn!("tailscale status did not answer in {timeout:?}");
                let _ = child.kill();
                let _ = child.wait();
                return Err(TailscaleUnavailable::TimedOut);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                tracing::warn!("tailscale status could not be waited for: {error}");
                let _ = child.kill();
                return Err(TailscaleUnavailable::NotRunning);
            }
        }
    };
    let remaining = timeout.saturating_sub(start.elapsed()) + Duration::from_millis(250);
    let output = match receiver.recv_timeout(remaining) {
        Ok(Ok(output)) => output,
        Ok(Err(_)) => return Err(TailscaleUnavailable::Unreadable),
        Err(_) => return Err(TailscaleUnavailable::TimedOut),
    };
    // Without a daemon the program prints nothing on its output and fails.
    if output.trim().is_empty() {
        return Err(if status.success() {
            TailscaleUnavailable::Unreadable
        } else {
            TailscaleUnavailable::NotRunning
        });
    }
    let peers = parse_status(&output)?;
    tracing::debug!("tailscale status listed {} other machines", peers.len());
    Ok(peers)
}

/// The first `tailscale` program found in `directories`.
pub fn find_program(directories: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "tailscale.exe"
    } else {
        "tailscale"
    };
    directories
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// `PATH`, then where the installers put the program when `PATH` (often
/// short for an application started from the desktop) does not have it.
fn search_directories() -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if cfg!(target_os = "macos") {
        directories.push(PathBuf::from("/usr/local/bin"));
        directories.push(PathBuf::from("/opt/homebrew/bin"));
    } else if cfg!(windows) {
        if let Some(programs) = std::env::var_os("ProgramFiles") {
            directories.push(PathBuf::from(programs).join("Tailscale"));
        }
    } else {
        directories.push(PathBuf::from("/usr/bin"));
        directories.push(PathBuf::from("/usr/local/bin"));
    }
    directories
}

fn reaches(peer: &TailscalePeer, host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    // With MagicDNS the short name resolves too, and people type it.
    (!peer.dns_name.is_empty() && (host == peer.dns_name || host == first_label(&peer.dns_name)))
        || peer.addresses.iter().any(|address| address == host)
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_lowercase()
}

fn first_label(dns_name: &str) -> &str {
    dns_name.split('.').next().unwrap_or(dns_name)
}

#[cfg(test)]
#[path = "tailscale_tests.rs"]
mod tests;
