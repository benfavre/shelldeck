//! Production launch intents. Parse before configuration, IPC or GUI startup so
//! help/version never create a session or modify desktop settings.
use shelldeck_core::config::deep_link::DeepLink;
use shelldeck_core::config::share_intent::ShareIntent;
use shelldeck_ui::WorkspacePresentation;
use std::ffi::OsString;
use std::path::Path;

pub(crate) const HELP: &str =
    "Usage: shelldeck [--agents | --terminal | --editor | --main | shelldeck://URL | --share FILE...]

  --agents       Open the focused Agents workspace
  --terminal     Open the focused Terminal view
  --editor       Open the focused Editor view
  --main         Open the full Inklura Desktop application
  --share FILE   Stage files in Share for review (nothing is uploaded
                 until you click Share)
  -h, --help     Show this help
  -V, --version  Show the application version

With no arguments, open Agents (respecting the start-hidden preference).
An existing instance receives the launch request and retains its sessions.";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Launch(Target),
    Help,
    Version,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) enum Target {
    #[default]
    DefaultAgents,
    Main,
    Agents,
    Terminal,
    Editor,
    DeepLink(String),
    /// Encoded [`ShareIntent`] payload for the single-instance hand-off.
    Share(String),
}

impl Target {
    /// An explicitly requested focused view opens its window even when the
    /// companion is set to start hidden in the tray.
    pub(crate) fn forces_visible(&self) -> bool {
        matches!(self, Self::Agents | Self::Terminal | Self::Editor)
    }

    /// The presentation the first window opens in. Deep links and share
    /// hand-offs start on the full app and route themselves afterwards.
    pub(crate) fn presentation(&self) -> WorkspacePresentation {
        match self {
            Self::DefaultAgents | Self::Agents => WorkspacePresentation::Agents,
            Self::Terminal => WorkspacePresentation::Terminal,
            Self::Editor => WorkspacePresentation::Editor,
            Self::Main | Self::DeepLink(_) | Self::Share(_) => WorkspacePresentation::Main,
        }
    }

    pub(crate) fn payload(&self) -> Option<&str> {
        match self {
            Self::Main => None,
            Self::DefaultAgents | Self::Agents => Some("shelldeck://agents"),
            Self::Terminal => Some("shelldeck://terminal"),
            Self::Editor => Some("shelldeck://editor"),
            Self::DeepLink(url) | Self::Share(url) => Some(url),
        }
    }
}

pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let cwd = std::env::current_dir().unwrap_or_default();
    parse_in(args, &cwd)
}

/// `parse` with an explicit working directory for relative `--share` paths.
pub(crate) fn parse_in(
    args: impl IntoIterator<Item = OsString>,
    cwd: &Path,
) -> Result<Command, String> {
    let mut target = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let arg = arg
            .to_str()
            .ok_or("Inklura Desktop launch arguments must be valid Unicode")?;
        let next = match arg {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--agents" => Target::Agents,
            "--terminal" => Target::Terminal,
            "--editor" => Target::Editor,
            "--main" => Target::Main,
            // File managers append the selected files after the flag, so the
            // rest of the command line is the file list.
            "--share" => {
                let mut files = Vec::new();
                for file in args.by_ref() {
                    files.push(
                        file.into_string().map_err(|_| {
                            "Inklura Desktop launch arguments must be valid Unicode"
                        })?,
                    );
                }
                if files.is_empty() {
                    return Err(format!("--share needs at least one file\n\n{HELP}"));
                }
                let files = shelldeck_core::config::share_intent::resolve_cli_paths(&files, cwd)
                    .map_err(|file| format!("Not a readable file: {file}"))?;
                let payload = ShareIntent { files }
                    .encode()
                    .ok_or_else(|| "Too many files or a non-Unicode path".to_string())?;
                Target::Share(payload)
            }
            // Finder supplied process serial numbers on older macOS versions.
            // They describe the launch, not a ShellDeck navigation target.
            value if value.starts_with("-psn_") => continue,
            url if DeepLink::looks_like(url) => match DeepLink::parse(url) {
                Some(DeepLink::Agents) => Target::Agents,
                Some(DeepLink::Terminal) => Target::Terminal,
                Some(DeepLink::Editor) => Target::Editor,
                _ => Target::DeepLink(url.to_owned()),
            },
            _ => return Err(format!("Unknown argument: {arg}\n\n{HELP}")),
        };
        if target.as_ref().is_some_and(|previous| previous != &next) {
            return Err(format!("Choose one launch target\n\n{HELP}"));
        }
        target = Some(next);
    }
    Ok(Command::Launch(target.unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // SDTEST-2633 / SDUC-558: the CLI must preserve a nonempty Agents intent
    // for primary/secondary launches and refuse ambiguous targets before I/O.
    #[test]
    fn sdtest_2633_launch_intents_preserve_agents_and_reject_ambiguous_targets() {
        let parse = |args: &[&str]| super::parse(args.iter().map(|arg| OsString::from(*arg)));
        for args in [vec![], vec!["-psn_0_42"]] {
            let Command::Launch(target) = parse(&args).unwrap() else {
                panic!("expected the default Agents intent")
            };
            assert_eq!(target.presentation(), WorkspacePresentation::Agents);
            assert!(!target.forces_visible());
            assert_eq!(target.payload(), Some("shelldeck://agents"));
        }
        let Command::Launch(main) = parse(&["--main"]).unwrap() else {
            panic!("expected the explicit full application intent")
        };
        assert_eq!(main.presentation(), WorkspacePresentation::Main);
        assert!(!main.forces_visible());
        assert_eq!(main.payload(), None);
        for args in [
            vec!["--agents"],
            vec!["--agents", "--agents"],
            vec!["shelldeck://agents"],
            vec!["ShellDeck://AGENTS/"],
            vec!["-psn_0_42", "--agents"],
        ] {
            let Command::Launch(target) = parse(&args).unwrap() else {
                panic!("expected a launch intent")
            };
            assert_eq!(target.presentation(), WorkspacePresentation::Agents);
            assert!(target.forces_visible());
            assert_eq!(target.payload(), Some("shelldeck://agents"));
        }
        let url = "shelldeck://issue/Iss_MixedCase";
        let Command::Launch(target) = parse(&[url]).unwrap() else {
            panic!("expected a deep link")
        };
        assert_eq!(target.payload(), Some(url));
        assert_eq!(target.presentation(), WorkspacePresentation::Main);
        assert_eq!(parse(&["--help"]), Ok(Command::Help));
        assert_eq!(parse(&["--agents", "-h"]), Ok(Command::Help));
        assert_eq!(parse(&["--version"]), Ok(Command::Version));
        assert_eq!(parse(&["-V"]), Ok(Command::Version));
        assert!(parse(&["--unknown"]).is_err());
        assert!(parse(&["日本語の引数です"]).is_err());
        assert!(parse(&["--agents", url]).is_err());
        assert!(parse(&[url, "--agents"]).is_err());
        assert!(parse(&["--main", "--agents"]).is_err());
        assert!(parse(&["--main", url]).is_err());
    }

    // SDTEST-3086 / SDUC-575: the flag and the URL of a focused view are one
    // intent, so a second launch forwards the same payload to the running
    // instance and the first window opens directly in that view.
    #[test]
    fn sdtest_3086_terminal_and_editor_flags_launch_their_focused_view() {
        let parse = |args: &[&str]| super::parse(args.iter().map(|arg| OsString::from(*arg)));
        for (args, presentation, payload) in [
            (
                vec!["--terminal"],
                WorkspacePresentation::Terminal,
                "shelldeck://terminal",
            ),
            (
                vec!["ShellDeck://Terminal/"],
                WorkspacePresentation::Terminal,
                "shelldeck://terminal",
            ),
            (
                vec!["--editor"],
                WorkspacePresentation::Editor,
                "shelldeck://editor",
            ),
            (
                vec!["shelldeck://editor"],
                WorkspacePresentation::Editor,
                "shelldeck://editor",
            ),
        ] {
            let Command::Launch(target) = parse(&args).unwrap() else {
                panic!("expected a launch intent")
            };
            assert_eq!(target.presentation(), presentation, "{args:?}");
            assert_eq!(target.payload(), Some(payload), "{args:?}");
            assert!(target.forces_visible(), "{args:?}");
        }
        // Other intents start on the full app; only the default opens Agents.
        let Command::Launch(link) = parse(&["shelldeck://issue/Iss_1"]).unwrap() else {
            panic!("expected a deep link")
        };
        assert_eq!(link.presentation(), WorkspacePresentation::Main);
        assert_eq!(
            Target::default().presentation(),
            WorkspacePresentation::Agents
        );
        assert_eq!(Target::Main.presentation(), WorkspacePresentation::Main);
        assert!(parse(&["--terminal", "--editor"]).is_err());
        assert!(parse(&["--agents", "--terminal"]).is_err());
        assert!(parse(&["--main", "--editor"]).is_err());
    }

    // "Share with Inklura" from a file manager: relative paths resolve
    // against the launching directory, the payload is the share intent
    // (never a browser-reachable deep link) and bad input fails before I/O.
    #[test]
    fn share_flag_resolves_files_into_a_share_intent() {
        let dir = std::env::temp_dir().join(format!("inklura-cli-share-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), b"a").unwrap();
        std::fs::write(dir.join("b c.pdf"), b"b").unwrap();
        let parse = |args: &[&str]| parse_in(args.iter().map(|arg| OsString::from(*arg)), &dir);
        let Command::Launch(target) = parse(&["--share", "a.png", "b c.pdf"]).unwrap() else {
            panic!("expected a share launch")
        };
        assert_eq!(target.presentation(), WorkspacePresentation::Main);
        let intent = ShareIntent::decode(target.payload().unwrap()).expect("share payload");
        assert_eq!(intent.files, vec![dir.join("a.png"), dir.join("b c.pdf")]);
        // Everything after --share is a file, even flag-looking names.
        assert!(parse(&["--share", "--main"]).is_err());
        assert!(parse(&["--share"]).is_err());
        assert!(parse(&["--share", "missing.png"]).is_err());
        assert!(parse(&["--main", "--share", "a.png"]).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
