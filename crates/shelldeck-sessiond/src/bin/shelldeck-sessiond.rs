//! `shelldeck-sessiond`: the session daemon, its SSH stdio proxy and a few
//! debug commands for manual testing. See docs/design/session-daemon.md.

#[cfg(not(unix))]
fn main() -> std::process::ExitCode {
    eprintln!("shelldeck-sessiond is only available on Unix");
    std::process::ExitCode::FAILURE
}

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    unix::main()
}

#[cfg(unix)]
mod unix {
    use shelldeck_sessiond::client::{
        connect_unix, AttachOptions, Client, ClientOptions, MirrorOutcome, PaneEvent, PaneMirror,
        SpawnOptions,
    };
    use shelldeck_sessiond::server::{self, launch, PaneLauncher, Server, ServerConfig};
    use shelldeck_sessiond::{paths, proxy};
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::ExitCode;
    use std::sync::atomic::{AtomicBool, Ordering};

    const USAGE: &str = "\
usage: shelldeck-sessiond <command>

  serve [--dir DIR] [--detach] [--history-budget BYTES]
                                      run the session daemon (the budget
                                      takes a K, M or G suffix)
  proxy --hello NONCE [--dir DIR]     stdio bridge to the daemon (SSH transport)
  --version

pane commands, for scripts and agents (each takes --dir DIR):
  ls [--json]                         list panes, with the status their
                                      program reported
  run [--name NAME] [--cwd DIR] [--rows N] [--cols N] [--json] [-- ARGV...]
                                      start a pane and print its id (login
                                      shell by default); starts the daemon
                                      when none runs
  split [PANE] [--right|--down] [--ratio R] [--name NAME] [--cwd DIR]
        [--json] [-- ARGV...]         start a pane and show it beside PANE
                                      in the application (beside the pane
                                      this runs in when PANE is not given);
                                      R is the share the new pane takes
                                      (0.1 to 0.9, default 0.5)
  send PANE [--no-enter] [--force] [TEXT...]
                                      type TEXT into a pane, then Enter
                                      (TEXT from stdin when not given).
                                      Refused while the pane's program waits
                                      for a permission, unless --force
  send-key PANE [--force] KEY...      press keys: enter, esc, tab, space,
                                      backspace, delete, up, down, left,
                                      right, home, end, pageup, pagedown,
                                      f1..f12 or one character, with
                                      ctrl-, alt-, shift- (ctrl-c, alt-b)
  read PANE [--lines N] [--format text|html]
                                      print the last N lines of a pane
                                      (default 40); html keeps the colours
  status PANE [--json]                what the pane's program last reported
                                      through OSC 7501, or none
  wait PANE [--until STATE] [--timeout SECONDS] [--json]
                                      wait until the pane's program exits
                                      and exit with its status; with
                                      --until, until it reports STATE
                                      (idle, working, done, blocked,
                                      error). 124 on timeout
  focus [PANE]                        select the pane's tab in the application
                                      and give it the keyboard
  rename [PANE] NAME                  name the pane's tab
  zoom [PANE], unzoom [PANE]          show the pane alone in its tab, or
                                      the whole split again
  events [--timeout SECONDS]          print one JSON line per change (panes
                                      started, ended, renamed, resized,
                                      status reported) until interrupted
  attach PANE                         attach this terminal (Ctrl-\\ detaches)
  kill PANE [1|15|9]                  signal a pane, or remove a dead one
  skill setup [--agent NAME]...       teach these commands to the coding
                                      agents found (claude, codex, opencode,
                                      pi); `skill show` prints the skill

  PANE is a pane id, the start of one, or a name given with --name. Inside
  a pane, $SHELLDECK_PANE is its own id.

debug commands:
  list [--dir DIR]                    list panes with stream offsets
  spawn [--dir DIR] [--rows N] [--cols N] [-- ARGV...]
                                      start a pane (login shell by default)
";

    static STOP: AtomicBool = AtomicBool::new(false);

    extern "C" fn on_terminate(_signal: libc::c_int) {
        STOP.store(true, Ordering::Relaxed);
    }

    extern "C" fn on_hangup(_signal: libc::c_int) {}

    #[derive(Default)]
    pub(crate) struct Args {
        pub(crate) dir: Option<PathBuf>,
        detach: bool,
        hello: Option<String>,
        pub(crate) rows: Option<u16>,
        pub(crate) cols: Option<u16>,
        nofile: Option<u64>,
        program: Option<String>,
        history_budget: Option<u64>,
        term_grace_ms: Option<u64>,
        idle_exit_ms: Option<u64>,
        log_cap: Option<u64>,
        seed: u64,
        bytes: u64,
        pace_us: u64,
        pub(crate) name: Option<String>,
        pub(crate) cwd: Option<String>,
        pub(crate) lines: Option<usize>,
        pub(crate) until: Option<String>,
        pub(crate) timeout: Option<u64>,
        pub(crate) json: bool,
        pub(crate) no_enter: bool,
        pub(crate) force: bool,
        pub(crate) format: Option<String>,
        pub(crate) ratio: Option<f32>,
        pub(crate) down: bool,
        pub(crate) agents: Vec<String>,
        pub(crate) interval_ms: Option<u64>,
        pub(crate) positional: Vec<String>,
        pub(crate) trailing: Vec<String>,
    }

    fn parse(args: Vec<String>) -> Result<Args, String> {
        let mut parsed = Args::default();
        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            let mut value = |name: &str| iter.next().ok_or(format!("{name} needs a value"));
            match arg.as_str() {
                "--dir" => parsed.dir = Some(PathBuf::from(value("--dir")?)),
                "--detach" => parsed.detach = true,
                "--hello" => parsed.hello = Some(value("--hello")?),
                "--rows" => parsed.rows = Some(number(&value("--rows")?)?),
                "--cols" => parsed.cols = Some(number(&value("--cols")?)?),
                "--nofile" => parsed.nofile = Some(number(&value("--nofile")?)?),
                "--program" => parsed.program = Some(value("--program")?),
                "--history-budget" => {
                    parsed.history_budget = Some(byte_size(&value("--history-budget")?)?)
                }
                // Not in the usage text: for tests, which cannot wait for
                // the production timeouts.
                "--term-grace-ms" => {
                    parsed.term_grace_ms = Some(number(&value("--term-grace-ms")?)?)
                }
                "--idle-exit-ms" => parsed.idle_exit_ms = Some(number(&value("--idle-exit-ms")?)?),
                "--log-cap" => parsed.log_cap = Some(byte_size(&value("--log-cap")?)?),
                "--seed" => parsed.seed = number(&value("--seed")?)?,
                "--bytes" => parsed.bytes = number(&value("--bytes")?)?,
                "--pace-us" => parsed.pace_us = number(&value("--pace-us")?)?,
                "--name" => parsed.name = Some(value("--name")?),
                "--cwd" => parsed.cwd = Some(value("--cwd")?),
                "--lines" => parsed.lines = Some(number(&value("--lines")?)?),
                "--until" => parsed.until = Some(value("--until")?),
                "--timeout" => parsed.timeout = Some(number(&value("--timeout")?)?),
                "--json" => parsed.json = true,
                "--no-enter" => parsed.no_enter = true,
                "--force" => parsed.force = true,
                "--format" => parsed.format = Some(value("--format")?),
                "--ratio" => parsed.ratio = Some(number(&value("--ratio")?)?),
                "--right" => parsed.down = false,
                "--down" => parsed.down = true,
                "--agent" => parsed.agents.push(value("--agent")?),
                // Not in the usage text: for tests of `events`.
                "--interval-ms" => parsed.interval_ms = Some(number(&value("--interval-ms")?)?),
                "--" => {
                    parsed.trailing = iter.collect();
                    break;
                }
                other if other.starts_with('-') => return Err(format!("unknown option {other}")),
                other => parsed.positional.push(other.to_string()),
            }
        }
        Ok(parsed)
    }

    fn number<T: std::str::FromStr>(text: &str) -> Result<T, String> {
        text.parse().map_err(|_| format!("invalid number {text}"))
    }

    /// A byte count with an optional binary suffix: `512M`, `2G`, `65536`.
    fn byte_size(text: &str) -> Result<u64, String> {
        let (digits, shift) = match text.as_bytes().last() {
            Some(b'k' | b'K') => (&text[..text.len() - 1], 10),
            Some(b'm' | b'M') => (&text[..text.len() - 1], 20),
            Some(b'g' | b'G') => (&text[..text.len() - 1], 30),
            _ => (text, 0),
        };
        let value: u64 = number(digits)?;
        value
            .checked_mul(1 << shift)
            .ok_or_else(|| format!("{text} is too large"))
    }

    pub fn main() -> ExitCode {
        let mut argv = std::env::args().skip(1);
        let Some(command) = argv.next() else {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        };
        if command == "--version" {
            // Printed from the marker itself, so the marker is part of
            // every build (see `shelldeck_sessiond::BUILD_MARKER`).
            print!("{}", &shelldeck_sessiond::BUILD_MARKER[4..]);
            return ExitCode::SUCCESS;
        }
        let args = match parse(argv.collect()) {
            Ok(args) => args,
            Err(message) => {
                eprintln!("shelldeck-sessiond: {message}");
                return ExitCode::from(2);
            }
        };
        let result = match command.as_str() {
            "serve" => serve(args),
            "proxy" => {
                let Some(nonce) = args.hello.as_deref() else {
                    eprintln!("shelldeck-sessiond: proxy needs --hello");
                    return ExitCode::from(2);
                };
                return ExitCode::from(proxy::run(args.dir.as_deref(), nonce));
            }
            // Started by the daemon for each pane, never by hand: restore
            // the descriptor limit, then become the pane program.
            "exec-pane" => Err(format!(
                "cannot run the pane program: {}",
                launch::exec_pane(args.nofile, args.program.as_deref(), &args.trailing)
            )),
            "debug-flood" => crate::flood::run(args.seed, args.bytes, args.pace_us),
            "ls" | "run" | "split" | "send" | "send-key" | "read" | "status" | "wait"
            | "events" | "skill" | "focus" | "rename" | "zoom" | "unzoom" => {
                return crate::panes::main(&command, args)
            }
            "list" | "spawn" | "attach" | "kill" => debug_command(&command, args),
            _ => {
                eprint!("{USAGE}");
                return ExitCode::from(2);
            }
        };
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("shelldeck-sessiond: {message}");
                ExitCode::FAILURE
            }
        }
    }

    // -- serve --------------------------------------------------------------

    /// Detach from whoever started us. Runs single-threaded, before anything
    /// else: fork (the starter never waits on the daemon), new session,
    /// stdio to /dev/null, every other inherited descriptor closed (an
    /// inherited SSH exec pipe would keep the channel open forever).
    fn daemonize() -> Result<(), String> {
        // SAFETY: called first thing in `main`, before any thread exists, so
        // the child may run arbitrary code after `fork`.
        unsafe {
            match libc::fork() {
                -1 => return Err("fork failed".into()),
                0 => {}
                _ => libc::_exit(0),
            }
            if libc::setsid() == -1 {
                return Err("setsid failed".into());
            }
            let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
            if null < 0 {
                return Err("cannot open /dev/null".into());
            }
            for fd in 0..=2 {
                libc::dup2(null, fd);
            }
            let limit = match libc::sysconf(libc::_SC_OPEN_MAX) {
                n if n > 0 => (n as i32).min(65_536),
                _ => 1024,
            };
            for fd in 3..limit {
                libc::close(fd);
            }
        }
        let home = paths::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        std::env::set_current_dir(home).map_err(|e| format!("chdir: {e}"))
    }

    /// Panes must not inherit the state of whichever client happened to
    /// start the daemon.
    fn scrub_environment() {
        const NAMES: [&str; 11] = [
            "SSH_AUTH_SOCK",
            "SSH_CONNECTION",
            "SSH_CLIENT",
            "SSH_TTY",
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XAUTHORITY",
            "TMUX",
            "TMUX_PANE",
            "TERM",
            "COLORTERM",
        ];
        let prefixed: Vec<_> = std::env::vars_os()
            .map(|(name, _)| name)
            .filter(|name| name.to_string_lossy().starts_with("SHELLDECK_"))
            .collect();
        // Before `APPDIR` itself goes: nothing of an AppImage mount stays
        // in what the terminals inherit.
        for (name, value) in
            paths::appimage_env_changes(std::env::vars_os(), std::env::var_os("APPDIR").as_deref())
        {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        for name in NAMES.iter().map(std::ffi::OsString::from).chain(prefixed) {
            std::env::remove_var(name);
        }
    }

    fn serve(args: Args) -> Result<(), String> {
        if args.detach {
            daemonize()?;
        }
        scrub_environment();
        // SAFETY: the handlers only store to an atomic or do nothing, which
        // is async-signal-safe. SIGHUP gets a no-op handler and is never set
        // to SIG_IGN, because an ignored disposition survives `exec` and
        // pane children would inherit it.
        unsafe {
            libc::signal(
                libc::SIGTERM,
                on_terminate as *const () as libc::sighandler_t,
            );
            libc::signal(
                libc::SIGINT,
                on_terminate as *const () as libc::sighandler_t,
            );
            libc::signal(libc::SIGHUP, on_hangup as *const () as libc::sighandler_t);
        }
        let dir = paths::resolve_run_dir(args.dir.as_deref()).map_err(|e| e.to_string())?;
        let mut config = ServerConfig::default();
        if let Some(budget) = args.history_budget {
            config.history_budget = budget;
        }
        if let Some(ms) = args.term_grace_ms {
            config.term_grace = std::time::Duration::from_millis(ms);
        }
        if let Some(ms) = args.idle_exit_ms {
            config.idle_exit = Some(std::time::Duration::from_millis(ms));
        }
        // The daemon needs more descriptors than a login session gets, the
        // pane programs must not: they are started through this executable
        // again, which puts the limit back. Without a way to do that the
        // limit stays as it is.
        let nofile_before = launch::nofile_soft();
        if let Some(exe) = launcher_exe() {
            if let Some(nofile_soft) = launch::raise_nofile_limit(launch::NOFILE_TARGET) {
                config.pane_launcher = Some(PaneLauncher { exe, nofile_soft });
            }
        }
        config.cli_exe = std::env::current_exe().ok().filter(|exe| exe.is_absolute());
        // Only the daemon that holds the instance lock writes the log, and
        // it opens it before a client can connect.
        let log_cap = args.log_cap.unwrap_or(server::log::DEFAULT_LOG_CAP);
        let bound = Server::bind_with(&dir, config, || {
            server::log::install(&dir, log_cap);
        });
        let Some(server) = bound.map_err(|e| e.to_string())? else {
            // A second `serve` exits 0: the first keeps serving.
            return Ok(());
        };
        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            pid = std::process::id(),
            dir = %dir.display(),
            nofile_before = ?nofile_before,
            nofile = ?launch::nofile_soft(),
            "session daemon started"
        );
        server.run(&STOP);
        Ok(())
    }

    /// The path pane children are started through. On Linux the running
    /// image itself, which stays valid when an app update replaces or
    /// removes the file the daemon was started from.
    fn launcher_exe() -> Option<PathBuf> {
        #[cfg(target_os = "linux")]
        {
            let image = PathBuf::from("/proc/self/exe");
            if image.exists() {
                return Some(image);
            }
        }
        std::env::current_exe().ok().filter(|exe| exe.is_absolute())
    }

    // -- debug commands -----------------------------------------------------

    fn debug_command(command: &str, args: Args) -> Result<(), String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        runtime.block_on(async {
            let dir = paths::resolve_run_dir(args.dir.as_deref()).map_err(|e| e.to_string())?;
            let client = connect_unix(&dir, ClientOptions::default())
                .await
                .map_err(|e| format!("cannot connect to {}: {e}", dir.display()))?;
            // An id, the start of one, or a name (`run --name`).
            let selector = args.positional.first().cloned().unwrap_or_default();
            let pane = || async {
                let panes = client.list_panes().await.map_err(|e| e.to_string())?;
                shelldeck_sessiond::pane_cli::resolve_pane(&selector, &panes)
            };
            match command {
                "list" => {
                    for info in client.list_panes().await.map_err(|e| e.to_string())? {
                        println!(
                            "{} {}x{} {} clients={} offset={} {:?}",
                            info.pane,
                            info.rows,
                            info.cols,
                            if info.alive {
                                "alive".to_string()
                            } else {
                                format!("exited({})", info.exit_code)
                            },
                            info.clients,
                            info.offset,
                            info.title,
                        );
                    }
                    Ok(())
                }
                "spawn" => {
                    let spawned = client
                        .spawn(SpawnOptions {
                            rows: args.rows.unwrap_or(24),
                            cols: args.cols.unwrap_or(80),
                            argv: args.trailing.clone(),
                            ..SpawnOptions::default()
                        })
                        .await
                        .map_err(|e| e.to_string())?;
                    println!("{} pid={} cwd={}", spawned.pane, spawned.pid, spawned.cwd);
                    Ok(())
                }
                "kill" => {
                    let signal = match args.positional.get(1) {
                        Some(text) => number(text)?,
                        None => 15,
                    };
                    client
                        .kill(pane().await?, signal)
                        .map_err(|e| e.to_string())?;
                    // One round trip, so the frame is delivered before exit.
                    client.list_panes().await.map_err(|e| e.to_string())?;
                    Ok(())
                }
                _ => attach(&client, pane().await?).await,
            }
        })
    }

    struct RawMode(libc::termios);

    impl RawMode {
        fn enter() -> Option<Self> {
            // SAFETY: termios is plain data; tcgetattr fills it on success.
            unsafe {
                let mut saved: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(0, &mut saved) != 0 {
                    return None;
                }
                let mut raw = saved;
                libc::cfmakeraw(&mut raw);
                libc::tcsetattr(0, libc::TCSANOW, &raw);
                Some(Self(saved))
            }
        }
    }

    impl Drop for RawMode {
        fn drop(&mut self) {
            // SAFETY: restores the settings read in `enter`.
            unsafe {
                libc::tcsetattr(0, libc::TCSANOW, &self.0);
            }
        }
    }

    fn terminal_size() -> (u16, u16) {
        // SAFETY: winsize is plain data; the ioctl fills it on success.
        unsafe {
            let mut size: libc::winsize = std::mem::zeroed();
            if libc::ioctl(0, libc::TIOCGWINSZ, &mut size) == 0
                && size.ws_row > 0
                && size.ws_col > 0
            {
                return (size.ws_row.min(1000), size.ws_col.min(1000));
            }
        }
        (24, 80)
    }

    /// Paint a restored snapshot on this terminal as plain text, then leave
    /// the cursor where the pane has it.
    fn repaint(mirror: &PaneMirror, out: &mut impl Write) -> std::io::Result<()> {
        let grid = mirror.grid().lock();
        write!(out, "\x1b[0m\x1b[2J")?;
        for (index, row) in grid.cells.iter().take(grid.rows).enumerate() {
            let text: String = row
                .iter()
                .filter(|cell| cell.wide != shelldeck_terminal::grid::CellWidth::Spacer)
                .flat_map(|cell| std::iter::once(cell.c).chain(cell.combining.iter().copied()))
                .collect();
            write!(out, "\x1b[{};1H{}", index + 1, text.trim_end())?;
        }
        write!(out, "\x1b[{};{}H", grid.cursor.row + 1, grid.cursor.col + 1)?;
        out.flush()
    }

    async fn attach(client: &Client, pane: uuid::Uuid) -> Result<(), String> {
        use tokio::io::AsyncReadExt;
        let (rows, cols) = terminal_size();
        let mut events = client
            .attach(
                pane,
                AttachOptions {
                    rows,
                    cols,
                    scrollback_rows: 0,
                    read_only: false,
                },
            )
            .map_err(|e| e.to_string())?;
        let _raw = RawMode::enter();
        let mut mirror = PaneMirror::new(rows, cols, 1, client.hello().emulator_rev);
        let mut stdin = tokio::io::stdin();
        let mut stdout = std::io::stdout();
        let mut buf = [0u8; 4096];
        let message = loop {
            tokio::select! {
                event = events.next() => {
                    let Some(event) = event else { break "connection closed".to_string() };
                    match &event {
                        PaneEvent::Restore { .. } => {
                            mirror.apply(&event).map_err(|e| e.to_string())?;
                            repaint(&mirror, &mut stdout).map_err(|e| e.to_string())?;
                        }
                        // The raw stream goes straight to this terminal.
                        PaneEvent::Bytes { data, .. } => {
                            stdout.write_all(data).and_then(|_| stdout.flush()).map_err(|e| e.to_string())?;
                        }
                        PaneEvent::Resize { .. } => {
                            if mirror.apply(&event) != Ok(MirrorOutcome::Applied) {
                                let _ = client.reattach(pane, true);
                            }
                        }
                        PaneEvent::Exited { exit_code, .. } => break format!("pane exited ({exit_code})"),
                        PaneEvent::Error(code) => break format!("daemon error {code:?}"),
                        PaneEvent::Closed => break "connection closed".to_string(),
                        PaneEvent::History { .. } | PaneEvent::Resyncing => {}
                    }
                }
                read = stdin.read(&mut buf) => {
                    match read {
                        Ok(0) | Err(_) => break "stdin closed".to_string(),
                        // Ctrl-\ detaches.
                        Ok(n) if buf[..n].contains(&0x1c) => break "detached".to_string(),
                        Ok(n) => client.input(pane, &buf[..n]).map_err(|e| e.to_string())?,
                    }
                }
            }
        };
        drop(_raw);
        println!("\r\n[shelldeck-sessiond: {message}]");
        Ok(())
    }
}

/// The pane commands: what a script or an agent uses to start programs in
/// panes that outlive it, type into them, read them and wait for them.
/// The pure parts are in `shelldeck_sessiond::pane_cli`. See
/// `docs/pane-cli.md`.
#[cfg(unix)]
mod panes {
    use crate::unix::Args;
    use shelldeck_sessiond::client::{
        connect_unix, AttachOptions, Client, ClientOptions, MirrorOutcome, PaneEvent, PaneMirror,
        SpawnOptions,
    };
    use shelldeck_sessiond::pane_cli::{
        exit_status, json_string, key_bytes, observed_events, panes_json, panes_table,
        resolve_pane, skill_markdown, status_json, status_line, Observed, SKILL_AGENTS, SKILL_NAME,
    };
    use shelldeck_sessiond::paths;
    use shelldeck_sessiond::protocol::{env_name_allowed, PaneInfo};
    use shelldeck_sessiond::requests::{self, ActRequest, Action, Request, Side, SplitRequest};
    use shelldeck_terminal::program_status::ProgramState;
    use std::io::{Read, Write};
    use std::path::Path;
    use std::process::ExitCode;
    use std::time::{Duration, Instant};

    /// Exit status of `wait` when the time given with `--timeout` runs out,
    /// as `timeout(1)` reports it.
    const TIMED_OUT: u8 = 124;
    /// How long `read` waits for the history after the screen arrived. The
    /// daemon marks the last history frame; this only bounds a daemon that
    /// has none to send.
    const HISTORY_GRACE: Duration = Duration::from_millis(1500);
    const DEFAULT_LINES: usize = 40;
    /// How long `split` waits for the application to show the pane.
    const SPLIT_ANSWER: Duration = Duration::from_secs(3);
    /// Label of the panes the application itself starts
    /// (`shelldeck-ui/src/terminal_view/daemon.rs`): a split pane without a
    /// name is one of them, so it comes back with the others.
    const APP_PANE_LABEL: &str = "shelldeck-local";

    pub fn main(command: &str, args: Args) -> ExitCode {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("shelldeck-sessiond: {error}");
                return ExitCode::FAILURE;
            }
        };
        match runtime.block_on(run(command, args)) {
            Ok(status) => ExitCode::from(status),
            Err(message) => {
                eprintln!("shelldeck-sessiond: {message}");
                ExitCode::FAILURE
            }
        }
    }

    async fn run(command: &str, args: Args) -> Result<u8, String> {
        // Nothing of the daemon is needed to write a file for an agent.
        if command == "skill" {
            return skill(&args);
        }
        let dir = paths::resolve_run_dir(args.dir.as_deref()).map_err(|e| e.to_string())?;
        // Only `run` starts a daemon: the other commands act on panes that
        // exist, and there are none without one.
        let client = connect(&dir, command == "run").await?;
        match command {
            "ls" => {
                let panes = client.list_panes().await.map_err(|e| e.to_string())?;
                let statuses = statuses(&client).await?.unwrap_or_default();
                if args.json {
                    println!("{}", panes_json(&panes, &statuses));
                } else {
                    print!("{}", panes_table(&panes, &statuses));
                }
                Ok(0)
            }
            "run" => {
                let label = args.name.clone().unwrap_or_default();
                let spawned = start(&client, &args, label, (40, 120)).await?;
                print_started(&args, &spawned, None);
                Ok(0)
            }
            "split" => split(&client, &dir, &args).await,
            "events" => events(&client, &args).await,
            "focus" | "rename" | "zoom" | "unzoom" => {
                let action = Action::parse(command).expect("matched above");
                act(&client, &dir, action, &args).await
            }
            _ => {
                let panes = client.list_panes().await.map_err(|e| e.to_string())?;
                let selector = args.positional.first().map(String::as_str).unwrap_or("");
                let id = resolve_pane(selector, &panes)?;
                let Some(info) = panes.iter().find(|info| info.pane == id) else {
                    return Err(format!("no pane {id}"));
                };
                match command {
                    "send" => send(&client, info, &args).await,
                    "send-key" => send_key(&client, info, &args).await,
                    "read" => read(&client, info, &args).await,
                    "status" => status(&client, info, &args).await,
                    _ => wait(&client, info, &args).await,
                }
            }
        }
    }

    /// What the programs reported, or `None` from a daemon that is older
    /// than this request.
    async fn statuses(
        client: &Client,
    ) -> Result<Option<Vec<shelldeck_sessiond::protocol::PaneStatus>>, String> {
        client.list_status().await.map_err(|e| e.to_string())
    }

    async fn connect(dir: &Path, start_daemon: bool) -> Result<Client, String> {
        let first = match connect_unix(dir, ClientOptions::default()).await {
            Ok(client) => return Ok(client),
            Err(error) => error,
        };
        if !start_daemon {
            return Err(format!(
                "no session daemon at {} ({first}). `run` starts one.",
                dir.display()
            ));
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        // `serve --detach` forks and returns at once; a second daemon next
        // to a running one exits by itself.
        std::process::Command::new(exe)
            .arg("serve")
            .arg("--detach")
            .arg("--dir")
            .arg(dir)
            .stdin(std::process::Stdio::null())
            .status()
            .map_err(|e| format!("cannot start the session daemon: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match connect_unix(dir, ClientOptions::default()).await {
                Ok(client) => return Ok(client),
                Err(error) if Instant::now() >= deadline => {
                    return Err(format!(
                        "the session daemon did not start at {}: {error}",
                        dir.display()
                    ))
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    }

    /// What a pane gets from the caller's environment, within what the
    /// daemon lets a client set: the terminal type and the locale.
    fn pane_env() -> Vec<(String, String)> {
        let mut env = vec![("TERM".to_string(), "xterm-256color".to_string())];
        for (name, value) in std::env::vars() {
            // The display and the agents of the caller, so a pane started
            // from a pane can do what its parent can.
            let wanted = name == "COLORTERM"
                || name == "LANG"
                || name.starts_with("LC_")
                || matches!(
                    name.as_str(),
                    "DISPLAY" | "WAYLAND_DISPLAY" | "XAUTHORITY" | "SSH_AUTH_SOCK"
                );
            if wanted && env_name_allowed(&name) && !value.is_empty() && value.len() <= 1024 {
                env.push((name, value));
            }
        }
        env
    }

    /// `run` and `split` print the pane id, or with `--json` an object.
    /// `shown`: what the application answered to `split`.
    fn print_started(
        args: &Args,
        spawned: &shelldeck_sessiond::client::SpawnedPane,
        shown: Option<bool>,
    ) {
        if !args.json {
            println!("{}", spawned.pane);
            return;
        }
        let shown = match shown {
            Some(shown) => format!(",\"shown\":{shown}"),
            None => String::new(),
        };
        println!(
            "{{\"id\":{},\"pid\":{},\"cwd\":{}{shown}}}",
            json_string(&spawned.pane.to_string()),
            spawned.pid,
            json_string(&spawned.cwd),
        );
    }

    async fn split(client: &Client, dir: &Path, args: &Args) -> Result<u8, String> {
        let own = std::env::var("SHELLDECK_PANE").unwrap_or_default();
        let selector = match args.positional.first() {
            Some(selector) => selector.as_str(),
            None if !own.is_empty() => own.as_str(),
            None => {
                return Err(
                    "split needs a pane: name one, or run it inside a pane (SHELLDECK_PANE)".into(),
                )
            }
        };
        let ratio = args.ratio.unwrap_or(0.5);
        if !(0.1..=0.9).contains(&ratio) {
            return Err(format!("--ratio takes 0.1 to 0.9, not {ratio}"));
        }
        let panes = client.list_panes().await.map_err(|e| e.to_string())?;
        let beside = resolve_pane(selector, &panes)?;
        let Some(info) = panes.iter().find(|info| info.pane == beside) else {
            return Err(format!("no pane {beside}"));
        };
        if !info.alive {
            return Err(format!("pane {beside} has exited"));
        }
        // About the size it will have; the application gives the real one.
        let (rows, cols) = if args.down {
            ((info.rows / 2).max(2), info.cols)
        } else {
            (info.rows, (info.cols / 2).max(10))
        };
        let label = args
            .name
            .clone()
            .unwrap_or_else(|| APP_PANE_LABEL.to_string());
        let spawned = start(client, args, label, (rows, cols)).await?;
        let request = Request::Split(SplitRequest {
            pane: spawned.pane,
            beside,
            side: if args.down { Side::Down } else { Side::Right },
            ratio,
        });
        let shown = match ask_application(dir, &request).await? {
            Some(Ok(())) => true,
            Some(Err(reason)) => {
                eprintln!(
                    "shelldeck-sessiond: the pane runs but is not shown beside {beside}: {reason}"
                );
                false
            }
            None => {
                eprintln!(
                    "shelldeck-sessiond: the pane runs, but no application showed it \
                     (Inklura Desktop is not running, or persistent sessions are off)"
                );
                false
            }
        };
        print_started(args, &spawned, Some(shown));
        Ok(0)
    }

    /// Leave `request` for the application and wait for its answer.
    /// `None`: no application took it.
    async fn ask_application(
        dir: &Path,
        request: &Request,
    ) -> Result<Option<Result<(), String>>, String> {
        let id = requests::submit(dir, request)
            .map_err(|e| format!("cannot ask the application: {e}"))?;
        let deadline = Instant::now() + SPLIT_ANSWER;
        loop {
            if let Some(answer) = requests::read_answer(dir, id) {
                return Ok(Some(answer));
            }
            if Instant::now() >= deadline {
                // Taken but not answered yet: the answer may still come,
                // and nobody waits for it.
                requests::withdraw(dir, id);
                return Ok(requests::read_answer(dir, id));
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }

    /// `focus`, `rename`, `zoom`, `unzoom`: things only the application
    /// can do with a pane it shows. The pane is the first argument, or the
    /// pane the command runs in.
    async fn act(client: &Client, dir: &Path, action: Action, args: &Args) -> Result<u8, String> {
        let own = std::env::var("SHELLDECK_PANE").unwrap_or_default();
        let words = &args.positional;
        // `rename NAME` inside a pane names that pane.
        let own_pane = !own.is_empty()
            && match action {
                Action::Rename => words.len() == 1,
                _ => words.is_empty(),
            };
        let (selector, rest) = if own_pane {
            (own.as_str(), &words[..])
        } else {
            match words.split_first() {
                Some((selector, rest)) => (selector.as_str(), rest),
                None => {
                    return Err(format!(
                        "{} needs a pane: name one, or run it inside a pane (SHELLDECK_PANE)",
                        action.word()
                    ))
                }
            }
        };
        let text = if action == Action::Rename {
            let name = requests::clean_name(
                &rest
                    .iter()
                    .chain(&args.trailing)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            if name.is_empty() {
                return Err("rename needs a name".into());
            }
            name
        } else {
            String::new()
        };
        let panes = client.list_panes().await.map_err(|e| e.to_string())?;
        let pane = resolve_pane(selector, &panes)?;
        let request = Request::Act(ActRequest { action, pane, text });
        match ask_application(dir, &request).await? {
            Some(Ok(())) => Ok(0),
            Some(Err(reason)) => Err(format!("{} {pane}: {reason}", action.word())),
            None => Err(
                "no application answered (Inklura Desktop is not running, or persistent \
                 sessions are off)"
                    .into(),
            ),
        }
    }

    /// Refuse to type into a pane whose program waits for its user to
    /// allow something: that answer is the user's to give. `--force`
    /// types all the same.
    async fn refuse_permission_prompt(
        client: &Client,
        info: &PaneInfo,
        args: &Args,
    ) -> Result<(), String> {
        if args.force {
            return Ok(());
        }
        let statuses = statuses(client).await?.unwrap_or_default();
        let waits = statuses
            .iter()
            .any(|s| s.pane == info.pane && s.state == "blocked" && s.kind == "permission");
        if waits {
            return Err(format!(
                "pane {} waits for its user to allow something; --force types anyway",
                info.pane
            ));
        }
        Ok(())
    }

    async fn events(client: &Client, args: &Args) -> Result<u8, String> {
        let interval = Duration::from_millis(args.interval_ms.unwrap_or(250).max(10));
        let deadline = args
            .timeout
            .map(|seconds| Instant::now() + Duration::from_secs(seconds));
        let mut before = Observed::default();
        let mut out = std::io::stdout().lock();
        loop {
            let after = Observed {
                panes: client.list_panes().await.map_err(|e| e.to_string())?,
                statuses: statuses(client).await?.unwrap_or_default(),
            };
            for event in observed_events(&before, &after) {
                // The reader went away: that is how this command ends.
                if writeln!(out, "{event}").and_then(|_| out.flush()).is_err() {
                    return Ok(0);
                }
            }
            before = after;
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Ok(0);
            }
            tokio::time::sleep(interval).await;
        }
    }

    /// `skill setup`: write the skill that teaches these commands into the
    /// skill directory of each coding agent, `skill show`: print it.
    fn skill(args: &Args) -> Result<u8, String> {
        match args.positional.first().map(String::as_str) {
            Some("show") => {
                print!("{}", skill_markdown());
                return Ok(0);
            }
            Some("setup") => {}
            _ => return Err("skill takes `setup` or `show`".into()),
        }
        for name in &args.agents {
            if !SKILL_AGENTS.iter().any(|(agent, _)| agent == name) {
                let known: Vec<&str> = SKILL_AGENTS.iter().map(|(agent, _)| *agent).collect();
                return Err(format!(
                    "unknown agent {name:?}: {} are known",
                    known.join(", ")
                ));
            }
        }
        // `--dir` stands for the home directory here (tests).
        let home = match &args.dir {
            Some(dir) => dir.clone(),
            None => paths::home_dir().ok_or("no home directory")?,
        };
        let mut written = 0;
        for (agent, skills) in SKILL_AGENTS {
            let skills = home.join(skills);
            let asked = args.agents.iter().any(|name| name == agent);
            // Without --agent: the agents that are set up for this user,
            // told by the directory that holds their skills directory.
            let present = skills.parent().is_some_and(Path::is_dir);
            if !(asked || (args.agents.is_empty() && present)) {
                continue;
            }
            let dir = skills.join(SKILL_NAME);
            let file = dir.join("SKILL.md");
            std::fs::create_dir_all(&dir)
                .and_then(|_| std::fs::write(&file, skill_markdown()))
                .map_err(|e| format!("cannot write {}: {e}", file.display()))?;
            println!("{agent}: {}", file.display());
            written += 1;
        }
        if written == 0 {
            return Err(
                "no coding agent found (claude, codex, opencode, pi): name one with --agent".into(),
            );
        }
        Ok(0)
    }

    async fn start(
        client: &Client,
        args: &Args,
        label: String,
        default_size: (u16, u16),
    ) -> Result<shelldeck_sessiond::client::SpawnedPane, String> {
        // Like a shell: the pane starts where the caller is.
        let cwd = match &args.cwd {
            Some(cwd) => cwd.clone(),
            None => std::env::current_dir()
                .ok()
                .and_then(|dir| dir.to_str().map(str::to_string))
                .unwrap_or_default(),
        };
        client
            .spawn(SpawnOptions {
                label,
                rows: args.rows.unwrap_or(default_size.0),
                cols: args.cols.unwrap_or(default_size.1),
                cwd,
                argv: args.trailing.clone(),
                env: pane_env(),
                ..SpawnOptions::default()
            })
            .await
            .map_err(|e| format!("cannot start the pane: {e}"))
    }

    /// Attach without asking for a size, so the pane keeps the size its
    /// viewers gave it.
    fn attach(
        client: &Client,
        info: &PaneInfo,
        scrollback_rows: u32,
        read_only: bool,
    ) -> Result<(shelldeck_sessiond::client::PaneEvents, PaneMirror), String> {
        let events = client
            .attach(
                info.pane,
                AttachOptions {
                    rows: 0,
                    cols: 0,
                    scrollback_rows,
                    read_only,
                },
            )
            .map_err(|e| e.to_string())?;
        let mirror = PaneMirror::new(
            info.rows.max(1),
            info.cols.max(1),
            (scrollback_rows as usize).max(1),
            client.hello().emulator_rev,
        );
        Ok((events, mirror))
    }

    /// Apply one event of `pane` to the mirror. `Err` carries why the pane
    /// cannot be followed any further.
    fn follow(
        client: &Client,
        pane: uuid::Uuid,
        mirror: &mut PaneMirror,
        event: &PaneEvent,
    ) -> Result<(), String> {
        match event {
            PaneEvent::Error(code) => return Err(format!("daemon error {code:?}")),
            PaneEvent::Closed => return Err("the connection to the daemon closed".into()),
            _ => {}
        }
        if mirror.apply(event).map_err(|e| e.to_string())? == MirrorOutcome::NeedsReattach {
            client.reattach(pane, true).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    async fn send(client: &Client, info: &PaneInfo, args: &Args) -> Result<u8, String> {
        if !info.alive {
            return Err(format!("pane {} has exited", info.pane));
        }
        refuse_permission_prompt(client, info, args).await?;
        let words: Vec<&str> = args.positional[1..]
            .iter()
            .chain(&args.trailing)
            .map(String::as_str)
            .collect();
        let text = if words.is_empty() {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|e| format!("cannot read the text from stdin: {e}"))?;
            text.strip_suffix('\n').map(str::to_string).unwrap_or(text)
        } else {
            words.join(" ")
        };

        // The screen says whether the program takes pastes whole.
        let (_events, mirror) = attach_for_input(client, info).await?;

        let mut bytes = Vec::with_capacity(text.len() + 16);
        if text.contains('\n') && mirror.grid().lock().bracketed_paste() {
            // Several lines go in as one paste, or the program would take
            // each line break as Enter.
            bytes.extend_from_slice(b"\x1b[200~");
            bytes.extend_from_slice(text.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
        } else {
            bytes.extend_from_slice(text.replace('\n', "\r").as_bytes());
        }
        if !args.no_enter {
            bytes.push(b'\r');
        }
        type_bytes(client, info, &bytes).await
    }

    /// Input needs an attached, writing client: attach and wait for the
    /// screen, which also carries the modes the program set.
    async fn attach_for_input(
        client: &Client,
        info: &PaneInfo,
    ) -> Result<(shelldeck_sessiond::client::PaneEvents, PaneMirror), String> {
        let (mut events, mut mirror) = attach(client, info, 0, false)?;
        let restored = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = events.next().await {
                follow(client, info.pane, &mut mirror, &event)?;
                if mirror.in_sync() {
                    return Ok(());
                }
            }
            Err("the connection to the daemon closed".to_string())
        })
        .await
        .map_err(|_| "the daemon did not send the pane".to_string())?;
        restored?;
        Ok((events, mirror))
    }

    async fn type_bytes(client: &Client, info: &PaneInfo, bytes: &[u8]) -> Result<u8, String> {
        for chunk in bytes.chunks(4096) {
            client.input(info.pane, chunk).map_err(|e| e.to_string())?;
        }
        // One round trip, so the input frames are delivered before exit.
        client.list_panes().await.map_err(|e| e.to_string())?;
        Ok(0)
    }

    async fn send_key(client: &Client, info: &PaneInfo, args: &Args) -> Result<u8, String> {
        if !info.alive {
            return Err(format!("pane {} has exited", info.pane));
        }
        let keys: Vec<&String> = args.positional[1..].iter().chain(&args.trailing).collect();
        if keys.is_empty() {
            return Err("send-key needs a key (enter, esc, ctrl-c, up, f5, ...)".into());
        }
        refuse_permission_prompt(client, info, args).await?;
        // The arrows depend on a mode the program set: read it off the screen.
        let (_events, mirror) = attach_for_input(client, info).await?;
        let application_cursor = mirror.grid().lock().application_cursor_keys();
        let mut bytes = Vec::new();
        for key in keys {
            bytes.extend(key_bytes(key, application_cursor)?);
        }
        type_bytes(client, info, &bytes).await
    }

    async fn status(client: &Client, info: &PaneInfo, args: &Args) -> Result<u8, String> {
        let Some(statuses) = statuses(client).await? else {
            return Err(
                "this session daemon was started by an older version and does not report \
                 status; a new one starts once its panes have ended"
                    .into(),
            );
        };
        let status = statuses.iter().find(|status| status.pane == info.pane);
        if args.json {
            println!("{}", status_json(info, status));
        } else {
            println!("{}", status_line(status));
        }
        Ok(0)
    }

    async fn read(client: &Client, info: &PaneInfo, args: &Args) -> Result<u8, String> {
        let lines = args.lines.unwrap_or(DEFAULT_LINES);
        let html = match args.format.as_deref() {
            None | Some("text") => false,
            Some("html") => true,
            Some(other) => return Err(format!("--format takes text or html, not {other:?}")),
        };
        // The screen comes first, then history: ask for what the screen
        // alone cannot give.
        let history = lines.saturating_sub(1).min(100_000) as u32;
        let (mut events, mut mirror) = attach(client, info, history, true)?;
        let mut deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let Ok(event) = tokio::time::timeout(left, events.next()).await else {
                break;
            };
            let Some(event) = event else { break };
            let last_history = matches!(event, PaneEvent::History { last: true, .. });
            let screen = matches!(event, PaneEvent::Restore { .. });
            follow(client, info.pane, &mut mirror, &event)?;
            if last_history || (screen && history == 0) {
                break;
            }
            if screen {
                deadline = Instant::now() + HISTORY_GRACE;
            }
        }
        if !mirror.in_sync() {
            return Err("the daemon did not send the pane".into());
        }
        let text = if html {
            mirror.grid().lock().tail_html(lines)
        } else {
            mirror.grid().lock().tail_text(lines).join("\n")
        };
        let mut out = std::io::stdout().lock();
        if !text.is_empty() {
            writeln!(out, "{text}").map_err(|e| e.to_string())?;
        }
        Ok(0)
    }

    async fn wait(client: &Client, info: &PaneInfo, args: &Args) -> Result<u8, String> {
        let until = match args.until.as_deref() {
            Some(word) => Some(ProgramState::parse(word).ok_or_else(|| {
                format!("--until takes idle, working, done, blocked or error, not {word:?}")
            })?),
            None => None,
        };
        let pane = json_string(&info.pane.to_string());
        let exited = |code: u8| {
            if args.json {
                println!("{{\"id\":{pane},\"result\":\"exited\",\"exit_code\":{code}}}");
            }
            code
        };
        let reported = |state: ProgramState| {
            if args.json {
                println!(
                    "{{\"id\":{pane},\"result\":\"state\",\"state\":\"{}\"}}",
                    state.word()
                );
            }
            0u8
        };
        if !info.alive {
            return Ok(exited(exit_status(info.exit_code, 0)));
        }
        let deadline = args
            .timeout
            .map(|seconds| Instant::now() + Duration::from_secs(seconds));
        let (mut events, mut mirror) = attach(client, info, 0, true)?;
        // Attached first, asked second: a report made in between is seen
        // by one of the two. A status reached before `wait` started is
        // known to the daemon, not to this new viewer.
        if let (Some(until), Some(statuses)) = (until, statuses(client).await?) {
            let now = statuses
                .iter()
                .find(|status| status.pane == info.pane)
                .and_then(|status| ProgramState::parse(&status.state));
            if now == Some(until) {
                return Ok(reported(until));
            }
        }
        loop {
            let event = match deadline {
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    match tokio::time::timeout(left, events.next()).await {
                        Ok(event) => event,
                        Err(_) => {
                            eprintln!("shelldeck-sessiond: timed out waiting for {}", info.pane);
                            if args.json {
                                println!("{{\"id\":{pane},\"result\":\"timeout\"}}");
                            }
                            return Ok(TIMED_OUT);
                        }
                    }
                }
                None => events.next().await,
            };
            let Some(event) = event else {
                return Err("the connection to the daemon closed".into());
            };
            if let PaneEvent::Exited {
                exit_code, signal, ..
            } = event
            {
                return Ok(exited(exit_status(exit_code, signal)));
            }
            follow(client, info.pane, &mut mirror, &event)?;
            if let Some(until) = until {
                // The program's own record when it has one, else whichever
                // of its records matters most.
                let grid = mirror.grid().lock();
                let state = grid
                    .program_status
                    .get("")
                    .map(|record| record.state)
                    .or_else(|| grid.program_status.headline().map(|h| h.state));
                if state == Some(until) {
                    drop(grid);
                    return Ok(reported(until));
                }
            }
        }
    }
}

/// Deterministic terminal output generator used by the attach race test: a
/// seeded mix of text, SGR, cursor movement, wide characters, scrolling and
/// alt-screen switches. It never queries the terminal. It prints without a
/// break (`pace_us` only spreads the output: that many microseconds
/// between slices). After `bytes` bytes it prints a marker and waits for one line on
/// stdin.
#[cfg(unix)]
mod flood {
    use std::io::{BufRead, Write};

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, bound: u64) -> u64 {
            self.next() % bound
        }
    }

    pub fn run(seed: u64, bytes: u64, pace_us: u64) -> Result<(), String> {
        let mut rng = Rng(seed | 1);
        // Written in fixed slices that ignore sequence boundaries, so the
        // reader on the other side of the PTY keeps getting chunks that end
        // inside an escape sequence or a multi-byte character.
        const SLICE: usize = 4099;
        let mut out = std::io::stdout().lock();
        let mut pending: Vec<u8> = Vec::with_capacity(2 * SLICE);
        let mut written = 0u64;
        let mut alt = false;
        let mut chunk = String::new();
        while written < bytes {
            chunk.clear();
            match rng.below(100) {
                0..=39 => {
                    let words = 1 + rng.below(8);
                    for _ in 0..words {
                        let len = 1 + rng.below(9);
                        for _ in 0..len {
                            chunk.push((b'a' + rng.below(26) as u8) as char);
                        }
                        chunk.push(' ');
                    }
                }
                40..=54 => chunk.push_str("\r\n"),
                55..=64 => {
                    use std::fmt::Write as _;
                    let _ = match rng.below(4) {
                        0 => write!(chunk, "\x1b[{}m", 30 + rng.below(8)),
                        1 => write!(chunk, "\x1b[1;4:3;38;5;{}m", rng.below(256)),
                        2 => write!(
                            chunk,
                            "\x1b[48;2;{};{};{}m",
                            rng.below(256),
                            rng.below(256),
                            rng.below(256)
                        ),
                        _ => write!(chunk, "\x1b[0m"),
                    };
                }
                65..=74 => {
                    use std::fmt::Write as _;
                    let _ = write!(chunk, "\x1b[{};{}H", 1 + rng.below(24), 1 + rng.below(80));
                }
                75..=82 => {
                    for _ in 0..1 + rng.below(6) {
                        let c = char::from_u32(0x4e00 + rng.below(0x500) as u32).unwrap_or('?');
                        chunk.push(c);
                    }
                }
                83..=86 => chunk
                    .push_str(["\x1b[K", "\x1b[2K", "\x1b[J", "\x1b[3L"][rng.below(4) as usize]),
                87..=89 => chunk.push_str("e\u{301}\u{1f600} \u{e9}\u{fc}"),
                90..=91 => chunk.push_str("\x1b(0lqqk\x1b(B"),
                92..=93 => {
                    use std::fmt::Write as _;
                    let _ = write!(chunk, "\x1b]0;flood {}\x07", rng.below(1000));
                }
                94..=95 => {
                    use std::fmt::Write as _;
                    let top = 1 + rng.below(10);
                    let _ = write!(chunk, "\x1b[{};{}r", top, top + 2 + rng.below(12));
                }
                96 => chunk.push_str("\x1b[r"),
                97..=98 => {
                    alt = !alt;
                    chunk.push_str(if alt { "\x1b[?1049h" } else { "\x1b[?1049l" });
                }
                _ => chunk.push_str("\x1b7\x1b[5;5Hmark\x1b8\tT"),
            }
            pending.extend_from_slice(chunk.as_bytes());
            written += chunk.len() as u64;
            while pending.len() >= SLICE {
                out.write_all(&pending[..SLICE])
                    .and_then(|_| out.flush())
                    .map_err(|e| e.to_string())?;
                pending.drain(..SLICE);
                if pace_us > 0 {
                    std::thread::sleep(std::time::Duration::from_micros(pace_us));
                }
            }
        }
        if alt {
            pending.extend_from_slice(b"\x1b[?1049l");
        }
        pending.extend_from_slice(b"\x1b[r\x1b[0m\r\nFLOOD-DONE\r\n");
        out.write_all(&pending)
            .and_then(|_| out.flush())
            .map_err(|e| e.to_string())?;
        drop(out);
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        Ok(())
    }
}
