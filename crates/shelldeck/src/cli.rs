//! Production launch intents. Parse before configuration, IPC or GUI startup so
//! help/version never create a session or modify desktop settings.
use shelldeck_core::config::deep_link::DeepLink;
use std::ffi::OsString;

pub(crate) const HELP: &str = "Usage: shelldeck [--agents | --main | shelldeck://URL]

  --agents       Open the focused Agents workspace
  --main         Open the full ShellDeck application
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
    DeepLink(String),
}

impl Target {
    pub(crate) fn is_agents(&self) -> bool {
        matches!(self, Self::DefaultAgents | Self::Agents)
    }

    pub(crate) fn forces_visible_agents(&self) -> bool {
        matches!(self, Self::Agents)
    }

    pub(crate) fn payload(&self) -> Option<&str> {
        match self {
            Self::Main => None,
            Self::DefaultAgents | Self::Agents => Some("shelldeck://agents"),
            Self::DeepLink(url) => Some(url),
        }
    }
}

pub(crate) fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut target = None;
    for arg in args {
        let arg = arg
            .to_str()
            .ok_or("ShellDeck launch arguments must be valid Unicode")?;
        let next = match arg {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--agents" => Target::Agents,
            "--main" => Target::Main,
            // Finder supplied process serial numbers on older macOS versions.
            // They describe the launch, not a ShellDeck navigation target.
            value if value.starts_with("-psn_") => continue,
            url if DeepLink::looks_like(url) => {
                if matches!(DeepLink::parse(url), Some(DeepLink::Agents)) {
                    Target::Agents
                } else {
                    Target::DeepLink(url.to_owned())
                }
            }
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
            assert!(target.is_agents());
            assert!(!target.forces_visible_agents());
            assert_eq!(target.payload(), Some("shelldeck://agents"));
        }
        let Command::Launch(main) = parse(&["--main"]).unwrap() else {
            panic!("expected the explicit full application intent")
        };
        assert!(!main.is_agents());
        assert!(!main.forces_visible_agents());
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
            assert!(target.is_agents());
            assert!(target.forces_visible_agents());
            assert_eq!(target.payload(), Some("shelldeck://agents"));
        }
        let url = "shelldeck://issue/Iss_MixedCase";
        let Command::Launch(target) = parse(&[url]).unwrap() else {
            panic!("expected a deep link")
        };
        assert_eq!(target.payload(), Some(url));
        assert!(!target.is_agents());
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
}
