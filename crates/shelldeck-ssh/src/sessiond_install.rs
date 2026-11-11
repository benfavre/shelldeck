//! Remote binary of the session daemon: probe, upload and the fallback
//! rules (docs/design/session-daemon.md, section 8, "Binary delivery and
//! version matching").
//!
//! Everything sent to the remote host is a fixed script. The only values
//! placed in it are checked against a strict alphabet first: the app
//! version (`0-9 a-z . -`), a random nonce (32 hex characters) and a
//! SHA-256 digest (64 hex characters). Nothing from a host entry, a
//! session or the remote side is interpolated.
//!
//! What an install writes on the remote host: one file,
//! `$HOME/.shelldeck/bin/shelldeck-sessiond-<version>` (mode 0700), through
//! a temporary file in the same directory. It never uses `sudo` and never
//! writes outside `$HOME/.shelldeck`. It runs only when the caller says
//! the user agreed for this host ([`InstallConsent::Allow`]).

use crate::session::SshSession;
use crate::sessiond::SessiondConnectError;
use crate::SshError;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

/// Static builds bundled with the app, by remote platform.
pub const TARGET_LINUX_X86_64: &str = "x86_64-unknown-linux-musl";
pub const TARGET_LINUX_AARCH64: &str = "aarch64-unknown-linux-musl";

/// Largest artifact that is ever uploaded.
pub const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;
/// Remote stdout and stderr kept from a probe or an install.
/// How many installed builds stay on a host after an install (design
/// section 8).
pub const KEPT_REMOTE_BUILDS: usize = 3;

const MAX_SCRIPT_OUTPUT: usize = 64 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(180);

/// `0-9 a-z . -`, 1 to 32 bytes: the alphabet of the versioned file name
/// (same rule as `protocol::proxy_command`).
fn version_is_valid(version: &str) -> bool {
    (1..=32).contains(&version.len())
        && version
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || b == b'.' || b == b'-')
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn nonce() -> String {
    Uuid::new_v4().simple().to_string()
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

// ---------------------------------------------------------------------------
// Probe
// ---------------------------------------------------------------------------

/// State of `$HOME/.shelldeck/bin/shelldeck-sessiond-<version>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteBinaryState {
    /// A regular file with the execute bit.
    Present,
    Absent,
    /// A regular file without the execute bit.
    NotExecutable,
}

/// What one probe says about a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteProbe {
    pub binary: RemoteBinaryState,
    /// `uname -s`, for example `Linux`.
    pub os: String,
    /// `uname -m`, for example `x86_64`.
    pub arch: String,
    /// `$SSH_CONNECTION` of the connection the probe ran on ("client
    /// address, client port, server address, server port"), when the host
    /// gave one in that form. For the panes spawned over that connection.
    pub ssh_connection: Option<String>,
}

impl RemoteProbe {
    /// The bundled build that runs on this host, if there is one.
    pub fn target(&self) -> Option<&'static str> {
        target_triple(&self.os, &self.arch)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("the app version cannot name a remote binary")]
    InvalidVersion,
    /// The exec was refused, or the host answered something else than the
    /// probe line (restricted shell, forced command).
    #[error("the remote host did not answer the session daemon probe")]
    NoAnswer,
    #[error("the session daemon probe did not finish in time")]
    Timeout,
    #[error(transparent)]
    Ssh(#[from] SshError),
}

/// The fixed probe script. `None` when version or nonce is not acceptable.
pub fn probe_command(version: &str, nonce: &str) -> Option<String> {
    (version_is_valid(version) && is_lower_hex(nonce, 32)).then(|| {
        format!(
            "b=\"$HOME/.shelldeck/bin/shelldeck-sessiond-{version}\"; s=absent; \
             if [ -f \"$b\" ]; then if [ -x \"$b\" ]; then s=present; else s=noexec; fi; fi; \
             printf 'SDSD-PROBE-{nonce} %s %s %s\\n' \"$s\" \"$(uname -s)\" \"$(uname -m)\"; \
             printf 'SDSD-CONN-{nonce} %s\\n' \"${{SSH_CONNECTION:-}}\""
        )
    })
}

/// The probe line among whatever else the remote shell printed (rc noise).
fn parse_probe(stdout: &[u8], nonce: &str) -> Option<RemoteProbe> {
    let prefix = format!("SDSD-PROBE-{nonce} ");
    let text = String::from_utf8_lossy(stdout);
    let line = text
        .lines()
        .find_map(|line| line.trim_end_matches('\r').strip_prefix(&prefix))?;
    let mut fields = line.split(' ');
    let binary = match fields.next()? {
        "present" => RemoteBinaryState::Present,
        "absent" => RemoteBinaryState::Absent,
        "noexec" => RemoteBinaryState::NotExecutable,
        _ => return None,
    };
    let os = fields.next()?.to_string();
    let arch = fields.next()?.to_string();
    let plain = |value: &str| {
        !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    };
    (fields.next().is_none() && plain(&os) && plain(&arch)).then_some(RemoteProbe {
        binary,
        os,
        arch,
        ssh_connection: parse_ssh_connection(&text, nonce),
    })
}

/// The `SDSD-CONN-<nonce>` line of a probe answer: an `SSH_CONNECTION`
/// value of exactly four fields, two addresses and two ports. Anything
/// else (no line, an empty value, a host that prints something odd) is no
/// value: the panes then have no `SSH_CONNECTION`, as before.
fn parse_ssh_connection(text: &str, nonce: &str) -> Option<String> {
    let prefix = format!("SDSD-CONN-{nonce} ");
    let value = text
        .lines()
        .find_map(|line| line.trim_end_matches('\r').strip_prefix(&prefix))?;
    let fields: Vec<&str> = value.split(' ').collect();
    let address = |field: &str| {
        !field.is_empty()
            && field.len() <= 64
            && field
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b':' | b'%'))
    };
    let port = |field: &str| field.parse::<u16>().is_ok_and(|port| port != 0);
    (fields.len() == 4
        && address(fields[0])
        && port(fields[1])
        && address(fields[2])
        && port(fields[3]))
    .then(|| value.to_string())
}

/// The `SSH_CLIENT` that goes with an `SSH_CONNECTION`: client address,
/// client port, server port.
pub fn ssh_client_of(ssh_connection: &str) -> Option<String> {
    let fields: Vec<&str> = ssh_connection.split(' ').collect();
    (fields.len() == 4).then(|| format!("{} {} {}", fields[0], fields[1], fields[3]))
}

/// One exec on `session`: is the binary of `version` installed, and what
/// platform is the host. Nothing is written on the remote host.
pub async fn probe(session: &SshSession, version: &str) -> Result<RemoteProbe, ProbeError> {
    let nonce = nonce();
    let script = probe_command(version, &nonce).ok_or(ProbeError::InvalidVersion)?;
    // The account's login shell may not be a POSIX shell.
    let command = format!("sh -c {}", crate::tmux::shell_quote(&script));
    let result = tokio::time::timeout(
        PROBE_TIMEOUT,
        session.exec_with_input_bounded(&command, &[], 0, MAX_SCRIPT_OUTPUT, MAX_SCRIPT_OUTPUT),
    )
    .await
    .map_err(|_| ProbeError::Timeout)??;
    parse_probe(&result.stdout, &nonce).ok_or(ProbeError::NoAnswer)
}

// ---------------------------------------------------------------------------
// Artifact
// ---------------------------------------------------------------------------

/// The bundled build for a host, from its `uname -s` and `uname -m`.
pub fn target_triple(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("Linux", "x86_64" | "amd64") => Some(TARGET_LINUX_X86_64),
        ("Linux", "aarch64" | "arm64") => Some(TARGET_LINUX_AARCH64),
        _ => None,
    }
}

/// File name of the bundled build for `triple`.
pub fn artifact_file_name(triple: &str) -> String {
    format!("shelldeck-sessiond-{triple}")
}

/// The first regular file named after `triple` in `dirs` (the directory of
/// the app binary, then the app data directory). Found by name only: see
/// [`load_artifact`] for what makes it usable.
pub fn find_artifact(dirs: &[PathBuf], triple: &str) -> Option<PathBuf> {
    let name = artifact_file_name(triple);
    dirs.iter()
        .map(|dir| dir.join(&name))
        .find(|path| path.is_file())
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("no session daemon build for {0} is bundled with the app")]
    Missing(String),
    #[error("the bundled session daemon build is not a {0} executable")]
    WrongFormat(String),
    #[error("the bundled session daemon build is larger than the upload limit")]
    TooLarge,
    /// The file was built for another release of the app (left beside a
    /// newer app by an update that did not carry a new one). Uploading it
    /// under this release's name would misname what runs on the server.
    #[error("the bundled session daemon build is not the build of version {0}")]
    WrongVersion(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Whether `bytes` is a little-endian 64-bit ELF executable for `triple`.
/// Guards against uploading a file that merely has the right name.
pub fn artifact_matches(bytes: &[u8], triple: &str) -> bool {
    let machine: u16 = match triple {
        TARGET_LINUX_X86_64 => 62,
        TARGET_LINUX_AARCH64 => 183,
        _ => return false,
    };
    // ELF magic, 64-bit class, little endian.
    bytes.get(..6) == Some(b"\x7fELF\x02\x01")
        && bytes.get(18..20) == Some(machine.to_le_bytes().as_slice())
}

/// Whether `bytes` is the daemon build of `version`: it carries that
/// release's build marker (`shelldeck_sessiond::BUILD_MARKER`). The file
/// name on the server is `shelldeck-sessiond-<version>`, and an installed
/// file is never replaced, so what is uploaded under that name must be
/// that release's build and nothing older.
pub fn artifact_is_version(bytes: &[u8], version: &str) -> bool {
    let marker = shelldeck_sessiond::build_marker(version);
    let marker = marker.as_bytes();
    version_is_valid(version) && bytes.windows(marker.len()).any(|window| window == marker)
}

/// Read the bundled build for `triple` and the release `version` from
/// `dirs`: the first file with the right name that is an executable for
/// that platform and carries the build marker of `version`. A file that
/// fails a check is skipped (a later directory may hold a good one); with
/// none usable, the error is that of the first file found.
pub fn load_artifact(
    dirs: &[PathBuf],
    triple: &str,
    version: &str,
) -> Result<Vec<u8>, ArtifactError> {
    let name = artifact_file_name(triple);
    let mut first_error = None;
    for path in dirs.iter().map(|dir| dir.join(&name)) {
        if !path.is_file() {
            continue;
        }
        match read_artifact(&path, triple, version) {
            Ok(bytes) => return Ok(bytes),
            Err(error) => {
                tracing::warn!(path = %path.display(), "session daemon build not usable: {error}");
                first_error.get_or_insert(error);
            }
        }
    }
    Err(first_error.unwrap_or_else(|| ArtifactError::Missing(triple.into())))
}

fn read_artifact(path: &Path, triple: &str, version: &str) -> Result<Vec<u8>, ArtifactError> {
    if std::fs::metadata(path)?.len() > MAX_ARTIFACT_BYTES as u64 {
        return Err(ArtifactError::TooLarge);
    }
    let bytes = std::fs::read(path)?;
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(ArtifactError::TooLarge);
    }
    if !artifact_matches(&bytes, triple) {
        return Err(ArtifactError::WrongFormat(triple.into()));
    }
    if !artifact_is_version(&bytes, version) {
        return Err(ArtifactError::WrongVersion(version.into()));
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("the app version cannot name a remote binary")]
    InvalidVersion,
    #[error("the session daemon build is empty or larger than the upload limit")]
    BadArtifact,
    /// The uploaded file does not have the digest of the local artifact.
    /// The temporary file was removed and nothing was installed.
    #[error("the uploaded session daemon failed its checksum on the remote host")]
    ChecksumMismatch,
    /// Neither `sha256sum`, `shasum` nor `openssl` exists on the host, so
    /// the upload cannot be verified. Nothing was installed.
    #[error("the remote host has no tool to verify the uploaded session daemon")]
    NoChecksumTool,
    /// The uploaded file cannot be executed where it was written (a home
    /// directory mounted `noexec`). Nothing was installed, and uploading
    /// again cannot help.
    #[error("the remote host cannot execute files in its home directory")]
    CannotExecute,
    #[error("installing the session daemon failed on the remote host (status {exit_status:?}): {detail}")]
    Failed {
        exit_status: Option<u32>,
        detail: String,
    },
    #[error("installing the session daemon did not finish in time")]
    Timeout,
    #[error(transparent)]
    Ssh(#[from] SshError),
}

/// The fixed install script. It reads the binary on stdin. `None` when a
/// value is not acceptable.
///
/// Steps: private umask, `mkdir -p "$HOME/.shelldeck/bin"`, write stdin to
/// a temporary file there, compare its SHA-256 with `sha256`, `chmod 700`,
/// check that the file can be executed there (a `noexec` home cannot),
/// rename over the final name. The temporary file is removed on every
/// failure. The answer is one line, `SDSD-INSTALL-<nonce> ok <digest>`,
/// `... mismatch <digest>`, `... nosha` or `... noexec`.
///
/// After a successful install the builds of older releases are pruned:
/// the [`KEPT_REMOTE_BUILDS`] most recently installed
/// `shelldeck-sessiond-<version>` files stay, the one just installed
/// always among them. Only regular files whose name is exactly that
/// prefix and a valid version are touched. A daemon still running from a
/// removed file keeps it (the file is only unlinked), and an app of that
/// release installs its build again when it connects. A failure here
/// never fails the install.
pub fn install_command(version: &str, sha256: &str, nonce: &str) -> Option<String> {
    (version_is_valid(version) && is_lower_hex(sha256, 64) && is_lower_hex(nonce, 32)).then(|| {
        format!(
            r#"set -eu
umask 077
d="$HOME/.shelldeck/bin"
mkdir -p "$d"
t="$d/.shelldeck-sessiond-{version}.{nonce}.tmp"
trap 'rm -f "$t"' EXIT
cat > "$t"
if command -v sha256sum >/dev/null 2>&1; then sum=$(sha256sum "$t")
elif command -v shasum >/dev/null 2>&1; then sum=$(shasum -a 256 "$t")
elif command -v openssl >/dev/null 2>&1; then sum=$(openssl dgst -sha256 -r "$t")
else echo "SDSD-INSTALL-{nonce} nosha"; exit 4
fi
sum=${{sum%% *}}
if [ "$sum" != "{sha256}" ]; then echo "SDSD-INSTALL-{nonce} mismatch $sum"; exit 3; fi
chmod 700 "$t"
if [ ! -x "$t" ]; then echo "SDSD-INSTALL-{nonce} noexec"; exit 5; fi
mv -f "$t" "$d/shelldeck-sessiond-{version}"
trap - EXIT
(
  cd "$d" || exit 0
  n=0
  ls -t 2>/dev/null | while IFS= read -r f; do
    case "$f" in shelldeck-sessiond-*) ;; *) continue ;; esac
    v=${{f#shelldeck-sessiond-}}
    case "$v" in ""|*[!0-9a-z.-]*) continue ;; esac
    if [ ! -f "$f" ] || [ -L "$f" ]; then continue; fi
    n=$((n+1))
    if [ "$n" -gt {KEPT_REMOTE_BUILDS} ] && [ "$f" != "shelldeck-sessiond-{version}" ]; then rm -f -- "$f"; fi
  done
) || true
echo "SDSD-INSTALL-{nonce} ok $sum"
"#
        )
    })
}

/// Upload `artifact` as the binary of `version` on the host of `session`
/// and verify it. Returns the SHA-256 the remote host computed, which is
/// also compared here with the digest of `artifact`.
///
/// The caller is responsible for the user's consent.
pub async fn install(
    session: &SshSession,
    version: &str,
    artifact: &[u8],
) -> Result<String, InstallError> {
    if artifact.is_empty() || artifact.len() > MAX_ARTIFACT_BYTES {
        return Err(InstallError::BadArtifact);
    }
    let expected = sha256_hex(artifact);
    let nonce = nonce();
    let script = install_command(version, &expected, &nonce).ok_or(InstallError::InvalidVersion)?;
    let command = format!("sh -c {}", crate::tmux::shell_quote(&script));
    let result = tokio::time::timeout(
        INSTALL_TIMEOUT,
        session.exec_with_input_bounded(
            &command,
            artifact,
            MAX_ARTIFACT_BYTES,
            MAX_SCRIPT_OUTPUT,
            MAX_SCRIPT_OUTPUT,
        ),
    )
    .await
    .map_err(|_| InstallError::Timeout)??;

    let prefix = format!("SDSD-INSTALL-{nonce} ");
    let stdout = String::from_utf8_lossy(&result.stdout);
    let answer = stdout
        .lines()
        .find_map(|line| line.trim_end_matches('\r').strip_prefix(&prefix));
    match answer.map(|answer| answer.split(' ').collect::<Vec<_>>()) {
        Some(fields) if fields.first() == Some(&"ok") && result.exit_code == Some(0) => {
            // The remote digest is checked here as well: the decision does
            // not rest on the remote script alone.
            match fields.get(1) {
                Some(remote) if *remote == expected => Ok(expected),
                _ => Err(InstallError::ChecksumMismatch),
            }
        }
        Some(fields) if fields.first() == Some(&"mismatch") => Err(InstallError::ChecksumMismatch),
        Some(fields) if fields.first() == Some(&"nosha") => Err(InstallError::NoChecksumTool),
        Some(fields) if fields.first() == Some(&"noexec") => Err(InstallError::CannotExecute),
        _ => Err(InstallError::Failed {
            exit_status: result.exit_code,
            detail: String::from_utf8_lossy(&result.stderr)
                .trim()
                .chars()
                .take(300)
                .collect(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Decision table
// ---------------------------------------------------------------------------

/// The user's recorded answer for one host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallConsent {
    /// Never asked, or asked and left undecided.
    Ask,
    Allow,
    /// "Never for this host".
    Never,
}

/// Why a host does not get the session daemon for this connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    /// The probe got no usable answer (restricted shell, forced command).
    ProbeFailed,
    /// No bundled build runs on the host's OS or architecture.
    UnsupportedPlatform,
    /// The build for the host's platform is not shipped with this app.
    NoArtifact,
    /// The binary is missing and the user has not answered yet.
    ConsentPending,
    /// The binary is missing and the user said never for this host.
    ConsentDeclined,
    /// The upload or its verification failed.
    InstallFailed,
    /// The host cannot run the binary (missing after all, not executable,
    /// exec refused): `SessiondConnectError::is_unavailable`.
    Unavailable,
    /// A daemon left by an incompatible release runs on the host. It is
    /// not stopped: its panes belong to the user.
    VersionMismatch,
    /// The proxy or the daemon failed in another way.
    ConnectFailed,
}

/// What to do for one connection, from the probe and the user's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The binary is there: connect.
    Connect,
    /// Upload the build for this target, then connect.
    Install { target: &'static str },
    /// Use the fallback backend. `ask`: offer the install to the user (it
    /// is possible and nobody answered yet).
    Fallback { reason: FallbackReason, ask: bool },
}

/// The rules of section 8. `has_artifact(target)`: is the bundled build
/// for that target available locally.
pub fn decide(
    probe: Option<&RemoteProbe>,
    consent: InstallConsent,
    has_artifact: impl Fn(&str) -> bool,
) -> Decision {
    let fallback = |reason| Decision::Fallback { reason, ask: false };
    let Some(probe) = probe else {
        return fallback(FallbackReason::ProbeFailed);
    };
    if probe.binary == RemoteBinaryState::Present {
        return Decision::Connect;
    }
    let Some(target) = probe.target() else {
        return fallback(FallbackReason::UnsupportedPlatform);
    };
    if !has_artifact(target) {
        return fallback(FallbackReason::NoArtifact);
    }
    match consent {
        InstallConsent::Allow => Decision::Install { target },
        InstallConsent::Never => fallback(FallbackReason::ConsentDeclined),
        InstallConsent::Ask => Decision::Fallback {
            reason: FallbackReason::ConsentPending,
            ask: true,
        },
    }
}

/// Why a failed [`crate::sessiond::connect`] ends the daemon attempt.
pub fn connect_fallback_reason(error: &SessiondConnectError) -> FallbackReason {
    match error {
        error if error.is_unavailable() => FallbackReason::Unavailable,
        SessiondConnectError::VersionMismatch => FallbackReason::VersionMismatch,
        _ => FallbackReason::ConnectFailed,
    }
}

/// The backend of an SSH terminal that does not use the session daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackBackend {
    /// `open_tmux_shell`: the existing tmux automatic attachment.
    Tmux,
    /// A plain shell on an SSH PTY.
    PlainShell,
}

/// Same choice as with the session daemon setting off.
pub fn fallback_backend(auto_attach_tmux: bool) -> FallbackBackend {
    if auto_attach_tmux {
        FallbackBackend::Tmux
    } else {
        FallbackBackend::PlainShell
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe_of(binary: RemoteBinaryState, os: &str, arch: &str) -> RemoteProbe {
        RemoteProbe {
            binary,
            os: os.into(),
            arch: arch.into(),
            ssh_connection: None,
        }
    }

    // SDTEST-3015 (SDUC-574): the fallback decision table. Nothing is
    // uploaded without a recorded "allow", and every other case names its
    // reason.
    #[test]
    fn sdtest_3015_decision_table_uploads_only_with_recorded_consent() {
        use Decision::{Connect, Fallback, Install};
        use FallbackReason::*;
        use InstallConsent::{Allow, Ask, Never};
        use RemoteBinaryState::{Absent, NotExecutable, Present};
        let all = |_: &str| true;
        let none = |_: &str| false;
        let quiet = |reason| Fallback { reason, ask: false };

        // An installed binary is used whatever the answer was, and no
        // artifact is needed for it, on any platform.
        for consent in [Ask, Allow, Never] {
            assert_eq!(
                decide(Some(&probe_of(Present, "Linux", "x86_64")), consent, none),
                Connect
            );
            assert_eq!(
                decide(
                    Some(&probe_of(Present, "FreeBSD", "riscv64")),
                    consent,
                    none
                ),
                Connect
            );
        }

        // Missing (or present without the execute bit): the answer decides.
        for state in [Absent, NotExecutable] {
            let linux = probe_of(state, "Linux", "x86_64");
            assert_eq!(
                decide(Some(&linux), Allow, all),
                Install {
                    target: TARGET_LINUX_X86_64
                }
            );
            assert_eq!(
                decide(Some(&linux), Ask, all),
                Fallback {
                    reason: ConsentPending,
                    ask: true
                }
            );
            assert_eq!(decide(Some(&linux), Never, all), quiet(ConsentDeclined));
        }
        assert_eq!(
            decide(Some(&probe_of(Absent, "Linux", "aarch64")), Allow, all),
            Install {
                target: TARGET_LINUX_AARCH64
            }
        );

        // Nothing to upload: never an install and never a question, even
        // with a recorded "allow".
        for consent in [Ask, Allow, Never] {
            assert_eq!(
                decide(Some(&probe_of(Absent, "Linux", "x86_64")), consent, none),
                quiet(NoArtifact)
            );
            assert_eq!(
                decide(Some(&probe_of(Absent, "Darwin", "arm64")), consent, all),
                quiet(UnsupportedPlatform)
            );
            assert_eq!(
                decide(Some(&probe_of(Absent, "Linux", "armv7l")), consent, all),
                quiet(UnsupportedPlatform)
            );
            assert_eq!(decide(None, consent, all), quiet(ProbeFailed));
        }
        // The artifact asked for is the one of the host's platform.
        let only_arm = |target: &str| target == TARGET_LINUX_AARCH64;
        assert_eq!(
            decide(Some(&probe_of(Absent, "Linux", "x86_64")), Allow, only_arm),
            quiet(NoArtifact)
        );

        // A failed connect: `is_unavailable` and a version mismatch are
        // fallbacks with their own reason.
        for error in [
            SessiondConnectError::BinaryMissing,
            SessiondConnectError::BinaryNotExecutable,
            SessiondConnectError::ExecRefused,
        ] {
            assert!(error.is_unavailable());
            assert_eq!(connect_fallback_reason(&error), Unavailable);
        }
        assert_eq!(
            connect_fallback_reason(&SessiondConnectError::VersionMismatch),
            VersionMismatch
        );
        for error in [
            SessiondConnectError::Timeout,
            SessiondConnectError::Prelude,
            SessiondConnectError::ProxyExited {
                exit_status: Some(1),
            },
        ] {
            assert_eq!(connect_fallback_reason(&error), ConnectFailed);
        }

        // The fallback backend is the choice made with the setting off.
        assert_eq!(fallback_backend(true), FallbackBackend::Tmux);
        assert_eq!(fallback_backend(false), FallbackBackend::PlainShell);
    }

    // SDTEST-3016 (SDUC-574): what may reach a remote shell, what a probe
    // answer may contain, and which local file counts as an artifact.
    #[test]
    fn sdtest_3016_scripts_take_only_checked_values_and_artifacts_are_checked() {
        let nonce = "0123456789abcdef0123456789abcdef";
        let sha = "a".repeat(64);
        let probe = probe_command("0.20.0", nonce).expect("probe command");
        assert!(probe.contains("\"$HOME/.shelldeck/bin/shelldeck-sessiond-0.20.0\""));
        let install = install_command("0.20.0", &sha, nonce).expect("install command");
        assert!(install.contains("d=\"$HOME/.shelldeck/bin\""));
        assert!(install.contains("\"$d/shelldeck-sessiond-0.20.0\""));
        assert!(!install.contains("sudo"));
        for bad in ["", "1.0;id", "1.0 x", "$(id)", "1.0'", "A", &"1".repeat(33)] {
            assert_eq!(probe_command(bad, nonce), None, "{bad:?}");
            assert_eq!(install_command(bad, &sha, nonce), None, "{bad:?}");
        }
        for bad in ["", "xyz", "0123456789ABCDEF0123456789ABCDEF", "$(id)"] {
            assert_eq!(probe_command("0.20.0", bad), None, "{bad:?}");
            assert_eq!(install_command("0.20.0", &sha, bad), None, "{bad:?}");
        }
        for bad in ["", "abc", &"g".repeat(64), &"A".repeat(64)] {
            assert_eq!(install_command("0.20.0", bad, nonce), None, "{bad:?}");
        }

        // The probe line is found among rc noise, only with this nonce.
        let answer = format!("motd\r\nSDSD-PROBE-{nonce} absent Linux x86_64\nbye\n");
        assert_eq!(
            parse_probe(answer.as_bytes(), nonce),
            Some(probe_of(RemoteBinaryState::Absent, "Linux", "x86_64"))
        );
        let other = "f".repeat(32);
        assert_eq!(parse_probe(answer.as_bytes(), &other), None);

        // The connection the probe ran on, when the host gives one in
        // the form sshd uses; `SSH_CLIENT` follows from it.
        let with = |conn: &str| {
            let answer =
                format!("SDSD-PROBE-{nonce} present Linux x86_64\nSDSD-CONN-{nonce} {conn}\r\n");
            parse_probe(answer.as_bytes(), nonce)
                .expect("the probe line is valid")
                .ssh_connection
        };
        for good in [
            "203.0.113.7 52114 198.51.100.9 22",
            "2001:db8::7 52114 2001:db8::9 2222",
            "fe80::1%eth0 40000 fe80::2%eth0 22",
        ] {
            assert_eq!(with(good).as_deref(), Some(good));
        }
        assert_eq!(
            ssh_client_of("203.0.113.7 52114 198.51.100.9 22").as_deref(),
            Some("203.0.113.7 52114 22")
        );
        for bad in [
            "",
            "203.0.113.7 52114 198.51.100.9",
            "203.0.113.7 52114 198.51.100.9 22 extra",
            "203.0.113.7 0 198.51.100.9 22",
            "203.0.113.7 99999 198.51.100.9 22",
            "$(id) 52114 198.51.100.9 22",
            "a;b 52114 198.51.100.9 22",
        ] {
            assert_eq!(with(bad), None, "{bad:?}");
        }
        // A line with another nonce is not this probe's.
        let foreign = format!(
            "SDSD-PROBE-{nonce} present Linux x86_64\nSDSD-CONN-{other} 203.0.113.7 1 198.51.100.9 22\n"
        );
        assert_eq!(
            parse_probe(foreign.as_bytes(), nonce)
                .expect("valid")
                .ssh_connection,
            None
        );
        for bad in [
            format!("SDSD-PROBE-{nonce} maybe Linux x86_64\n"),
            format!("SDSD-PROBE-{nonce} present Linux\n"),
            format!("SDSD-PROBE-{nonce} present Linux x86_64 extra\n"),
            format!("SDSD-PROBE-{nonce} present Lin$ux x86_64\n"),
            format!(" SDSD-PROBE-{nonce} present Linux x86_64\n"),
        ] {
            assert_eq!(parse_probe(bad.as_bytes(), nonce), None, "{bad:?}");
        }

        // Platforms with a bundled build.
        assert_eq!(target_triple("Linux", "x86_64"), Some(TARGET_LINUX_X86_64));
        assert_eq!(
            target_triple("Linux", "aarch64"),
            Some(TARGET_LINUX_AARCH64)
        );
        assert_eq!(target_triple("Linux", "arm64"), Some(TARGET_LINUX_AARCH64));
        assert_eq!(target_triple("Darwin", "x86_64"), None);
        assert_eq!(target_triple("Linux", "i686"), None);

        // An artifact is found by name, first directory first, and must be
        // an ELF executable of the right machine, built for this release.
        let elf_of = |machine: u16, version: &str| {
            let mut bytes = b"\x7fELF\x02\x01\x01\x00".to_vec();
            bytes.resize(18, 0);
            bytes.extend_from_slice(&machine.to_le_bytes());
            bytes.resize(64, 0);
            bytes.extend_from_slice(shelldeck_sessiond::build_marker(version).as_bytes());
            bytes.resize(160, 0);
            bytes
        };
        let elf = |machine: u16| elf_of(machine, "0.20.0");
        assert!(artifact_matches(&elf(62), TARGET_LINUX_X86_64));
        assert!(artifact_matches(&elf(183), TARGET_LINUX_AARCH64));
        assert!(!artifact_matches(&elf(183), TARGET_LINUX_X86_64));
        assert!(!artifact_matches(
            b"#!/bin/sh\necho no\n",
            TARGET_LINUX_X86_64
        ));
        assert!(!artifact_matches(&elf(62), "x86_64-apple-darwin"));

        let first = tempfile::tempdir().expect("temp dir");
        let second = tempfile::tempdir().expect("temp dir");
        let dirs = [first.path().to_path_buf(), second.path().to_path_buf()];
        assert!(matches!(
            load_artifact(&dirs, TARGET_LINUX_X86_64, "0.20.0"),
            Err(ArtifactError::Missing(_))
        ));
        let name = artifact_file_name(TARGET_LINUX_X86_64);
        assert_eq!(name, "shelldeck-sessiond-x86_64-unknown-linux-musl");
        std::fs::write(second.path().join(&name), elf(62)).expect("write");
        assert_eq!(
            find_artifact(&dirs, TARGET_LINUX_X86_64),
            Some(second.path().join(&name))
        );
        assert_eq!(find_artifact(&dirs, TARGET_LINUX_AARCH64), None);
        assert_eq!(
            load_artifact(&dirs, TARGET_LINUX_X86_64, "0.20.0").expect("load"),
            elf(62)
        );

        // SDTEST-3044: the build must be the one of this release. A build
        // of an earlier release left beside a newer app (an update whose
        // archive carried no remote build) is never uploaded under the
        // new release's name.
        assert!(artifact_is_version(&elf(62), "0.20.0"));
        for other in [
            "0.20.1", "0.20", "0.20.00", "0.2", "1.0.20.0", "", "0.20.0\n",
        ] {
            assert!(!artifact_is_version(&elf(62), other), "{other:?}");
        }
        assert!(!artifact_is_version(b"\x7fELF no marker at all", "0.20.0"));
        // The marker is the one the daemon binary is built with.
        assert_eq!(
            shelldeck_sessiond::BUILD_MARKER,
            shelldeck_sessiond::build_marker(env!("CARGO_PKG_VERSION"))
        );
        assert!(matches!(
            load_artifact(&dirs, TARGET_LINUX_X86_64, "0.21.0"),
            Err(ArtifactError::WrongVersion(_))
        ));
        // A stale build in the first directory does not hide the build
        // of this release in a later one, and the other way round.
        std::fs::write(first.path().join(&name), elf_of(62, "0.19.0")).expect("write");
        assert_eq!(
            load_artifact(&dirs, TARGET_LINUX_X86_64, "0.20.0").expect("load"),
            elf(62)
        );
        assert_eq!(
            load_artifact(&dirs, TARGET_LINUX_X86_64, "0.19.0").expect("load"),
            elf_of(62, "0.19.0")
        );

        // A file with the right name and the wrong content is refused.
        std::fs::remove_file(second.path().join(&name)).expect("remove");
        std::fs::write(first.path().join(&name), b"not an executable").expect("write");
        assert!(matches!(
            load_artifact(&dirs, TARGET_LINUX_X86_64, "0.20.0"),
            Err(ArtifactError::WrongFormat(_))
        ));
    }
}
