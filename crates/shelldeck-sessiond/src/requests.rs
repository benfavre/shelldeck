//! Requests the pane commands leave for the running application.
//!
//! The daemon owns the panes; where a pane is shown (which tab, beside
//! which pane) is the application's. `shelldeck-sessiond split` therefore
//! starts its pane through the daemon like `run`, then asks the
//! application to show it beside another one by leaving a small file in
//! `requests/` under the run directory, which only the user can read or
//! write. The application takes the file, does it and leaves an answer the
//! command waits for. No application, or one that does not show session
//! daemon panes: nobody answers, and the pane runs all the same.
//!
//! The other requests act on a pane the application already shows: give
//! it the keyboard, name it, show it alone ([`ActRequest`]).
//!
//! A request is a few `key=value` lines: the crate has no JSON parser and
//! the values are ids, words, a number and one line of text.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

/// A request nobody took for this long was left by a command that has
/// stopped waiting: it is removed, not done late.
pub const REQUEST_TTL: Duration = Duration::from_secs(15);

/// Which side of the pane it splits the new pane takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Right,
    Down,
}

impl Side {
    pub fn word(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Down => "down",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "right" => Some(Self::Right),
            "down" => Some(Self::Down),
            _ => None,
        }
    }
}

/// Show `pane` beside `beside`, which keeps `1 - ratio` of the space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitRequest {
    pub pane: Uuid,
    pub beside: Uuid,
    pub side: Side,
    /// Share of the space the new pane takes, 0.1 to 0.9.
    pub ratio: f32,
}

impl SplitRequest {
    pub fn encode(&self) -> String {
        format!(
            "action=split\npane={}\nbeside={}\nside={}\nratio={}\n",
            self.pane,
            self.beside,
            self.side.word(),
            self.ratio
        )
    }

    /// `None` for anything that is not a complete split request: the file
    /// comes from the user's own directory, but is still only data.
    pub fn decode(text: &str) -> Option<Self> {
        let value = |key: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
                .map(str::trim)
        };
        if value("action")? != "split" {
            return None;
        }
        let ratio: f32 = value("ratio")?.parse().ok()?;
        if !ratio.is_finite() {
            return None;
        }
        Some(Self {
            pane: value("pane")?.parse().ok()?,
            beside: value("beside")?.parse().ok()?,
            side: Side::parse(value("side")?)?,
            ratio: ratio.clamp(0.1, 0.9),
        })
    }
}

/// Longest name `rename` gives, in characters.
pub const MAX_NAME: usize = 80;

/// What to do with a pane the application shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Select its tab and give it the keyboard.
    Focus,
    /// Name its tab.
    Rename,
    /// Show it alone in its tab.
    Zoom,
    /// Show the whole split of its tab again.
    Unzoom,
}

impl Action {
    pub fn word(self) -> &'static str {
        match self {
            Self::Focus => "focus",
            Self::Rename => "rename",
            Self::Zoom => "zoom",
            Self::Unzoom => "unzoom",
        }
    }

    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "focus" => Some(Self::Focus),
            "rename" => Some(Self::Rename),
            "zoom" => Some(Self::Zoom),
            "unzoom" => Some(Self::Unzoom),
            _ => None,
        }
    }
}

/// Do `action` on `pane`. `text` is the name for [`Action::Rename`] and
/// empty otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActRequest {
    pub action: Action,
    pub pane: Uuid,
    pub text: String,
}

/// One line, no control characters, at most [`MAX_NAME`] characters.
pub fn clean_name(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_NAME)
        .collect()
}

impl ActRequest {
    pub fn encode(&self) -> String {
        format!(
            "action={}\npane={}\ntext={}\n",
            self.action.word(),
            self.pane,
            clean_name(&self.text)
        )
    }

    pub fn decode(text: &str) -> Option<Self> {
        let value = |key: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
        };
        let action = Action::parse(value("action")?.trim())?;
        let name = clean_name(value("text").unwrap_or(""));
        if action == Action::Rename && name.is_empty() {
            return None;
        }
        Some(Self {
            action,
            pane: value("pane")?.trim().parse().ok()?,
            text: name,
        })
    }
}

/// Anything a command asks of the application.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Split(SplitRequest),
    Act(ActRequest),
}

impl Request {
    pub fn encode(&self) -> String {
        match self {
            Self::Split(request) => request.encode(),
            Self::Act(request) => request.encode(),
        }
    }

    pub fn decode(text: &str) -> Option<Self> {
        SplitRequest::decode(text)
            .map(Self::Split)
            .or_else(|| ActRequest::decode(text).map(Self::Act))
    }
}

pub fn requests_dir(run_dir: &Path) -> PathBuf {
    run_dir.join("requests")
}

fn request_path(run_dir: &Path, id: Uuid) -> PathBuf {
    requests_dir(run_dir).join(format!("{id}.req"))
}

fn answer_path(run_dir: &Path, id: Uuid) -> PathBuf {
    requests_dir(run_dir).join(format!("{id}.done"))
}

/// Write `text` so a reader sees all of it or no file.
fn write_whole(path: &Path, text: &str) -> std::io::Result<()> {
    let partial = path.with_extension("tmp");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, path)
}

/// Leave a request for the application. Returns its id.
pub fn submit(run_dir: &Path, request: &Request) -> std::io::Result<Uuid> {
    let dir = requests_dir(run_dir);
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let id = Uuid::new_v4();
    write_whole(&request_path(run_dir, id), &request.encode())?;
    Ok(id)
}

/// Take every request that waits, oldest first. A taken request is gone:
/// one application does it. Files that are not requests, and requests
/// older than [`REQUEST_TTL`], are removed without being returned.
pub fn take(run_dir: &Path) -> Vec<(Uuid, Request)> {
    let Ok(entries) = std::fs::read_dir(requests_dir(run_dir)) else {
        return Vec::new();
    };
    let now = SystemTime::now();
    let mut taken = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let modified = entry.metadata().and_then(|meta| meta.modified()).ok();
        let old = modified
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age > REQUEST_TTL);
        let is_request = path.extension().is_some_and(|ext| ext == "req");
        if !is_request {
            // Answers nobody read and partial files of a writer that died.
            if old {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        let id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<Uuid>().ok());
        let text = std::fs::read_to_string(&path).ok();
        // Removing it is taking it: whoever fails to remove it lost the
        // race to another application and leaves the request alone.
        if std::fs::remove_file(&path).is_err() || old {
            continue;
        }
        if let (Some(id), Some(request)) = (id, text.as_deref().and_then(Request::decode)) {
            taken.push((modified, id, request));
        }
    }
    taken.sort_by_key(|(modified, id, _)| (*modified, *id));
    taken
        .into_iter()
        .map(|(_, id, request)| (id, request))
        .collect()
}

/// Leave the answer to a request: `Ok` when it was done, else why not.
pub fn answer(run_dir: &Path, id: Uuid, result: Result<(), &str>) {
    let text = match result {
        Ok(()) => "ok\n".to_string(),
        Err(reason) => format!("error {}\n", reason.replace('\n', " ")),
    };
    let _ = write_whole(&answer_path(run_dir, id), &text);
}

/// The answer to a request, once: `None` while there is none.
pub fn read_answer(run_dir: &Path, id: Uuid) -> Option<Result<(), String>> {
    let path = answer_path(run_dir, id);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(match text.trim().strip_prefix("error") {
        Some(reason) => Err(reason.trim().to_string()),
        None => Ok(()),
    })
}

/// Withdraw a request nobody took. `true`: it was still there.
pub fn withdraw(run_dir: &Path, id: Uuid) -> bool {
    std::fs::remove_file(request_path(run_dir, id)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-3116 / SDUC-581. A split request goes from the command to the
    // application through the run directory: taken once, answered once,
    // and what is not a fresh, complete request is never done.
    #[test]
    fn sdtest_3116_a_split_request_is_taken_once_and_answered() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run = dir.path();
        assert!(take(run).is_empty(), "no directory yet");

        let split = SplitRequest {
            pane: Uuid::from_u128(1),
            beside: Uuid::from_u128(2),
            side: Side::Down,
            ratio: 0.3,
        };
        let request = Request::Split(split);
        let id = submit(run, &request).expect("submit");
        assert_eq!(read_answer(run, id), None);

        // Not requests: a truncated one, another action, a stray file.
        let requests = requests_dir(run);
        let stray = Uuid::from_u128(9);
        std::fs::write(
            requests.join(format!("{stray}.req")),
            "action=split\npane=x\n",
        )
        .unwrap();
        std::fs::write(
            requests.join(format!("{}.req", Uuid::from_u128(10))),
            "action=quit\n",
        )
        .unwrap();
        std::fs::write(requests.join("notes.txt"), "hello").unwrap();

        assert_eq!(take(run), vec![(id, request.clone())]);
        assert!(take(run).is_empty(), "a request is taken once");
        assert!(!withdraw(run, id), "taken, so nothing to withdraw");
        assert!(!requests.join(format!("{stray}.req")).exists());

        answer(run, id, Err("no such pane\nshown"));
        assert_eq!(read_answer(run, id), Some(Err("no such pane shown".into())));
        assert_eq!(read_answer(run, id), None, "an answer is read once");
        answer(run, id, Ok(()));
        assert_eq!(read_answer(run, id), Some(Ok(())));

        // Out-of-range ratios are brought back into range, a ratio that is
        // not a number is refused.
        let wide = SplitRequest::decode(&split.encode().replace("0.3", "7")).unwrap();
        assert_eq!(wide.ratio, 0.9);
        assert_eq!(
            SplitRequest::decode(&split.encode().replace("0.3", "NaN")),
            None
        );

        // Withdrawn before anyone took it: nobody does it later.
        let late = submit(run, &request).expect("submit");
        assert!(withdraw(run, late));
        assert!(take(run).is_empty());
    }

    // SDTEST-3122 / SDUC-582. Focus, rename, zoom and unzoom travel the
    // same way as a split. A name is one line of at most MAX_NAME
    // characters, and a rename without a name is not a request.
    #[test]
    fn sdtest_3122_pane_actions_are_requests_with_a_clean_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run = dir.path();
        let pane = Uuid::from_u128(7);
        for action in [Action::Focus, Action::Rename, Action::Zoom, Action::Unzoom] {
            assert_eq!(Action::parse(action.word()), Some(action));
            let request = Request::Act(ActRequest {
                action,
                pane,
                text: if action == Action::Rename {
                    "build".into()
                } else {
                    String::new()
                },
            });
            let id = submit(run, &request).expect("submit");
            assert_eq!(take(run), vec![(id, request)]);
        }

        let messy = ActRequest {
            action: Action::Rename,
            pane,
            text: format!("  api\nserver\x1b[31m {}", "x".repeat(200)),
        };
        let decoded = ActRequest::decode(&messy.encode()).expect("decodes");
        assert!(decoded.text.starts_with("api server [31m x"));
        assert_eq!(decoded.text.chars().count(), MAX_NAME);
        assert!(!decoded.text.chars().any(char::is_control));

        assert_eq!(
            ActRequest::decode(&format!("action=rename\npane={pane}\ntext=  \n")),
            None
        );
        assert_eq!(ActRequest::decode("action=focus\npane=nope\n"), None);
        assert_eq!(
            Request::decode(&format!("action=quit\npane={pane}\n")),
            None
        );
    }
}
