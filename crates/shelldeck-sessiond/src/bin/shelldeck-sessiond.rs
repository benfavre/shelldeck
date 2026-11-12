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
  ls [--json]                         list panes
  run [--name NAME] [--cwd DIR] [--rows N] [--cols N] [-- ARGV...]
                                      start a pane and print its id (login
                                      shell by default); starts the daemon
                                      when none runs
  send PANE [--no-enter] [TEXT...]    type TEXT into a pane, then Enter
                                      (TEXT from stdin when not given)
  read PANE [--lines N]               print the last N lines of a pane
                                      (default 40)
  wait PANE [--until STATE] [--timeout SECONDS]
                                      wait until the pane's program exits
                                      and exit with its status; with
                                      --until, until it reports STATE
                                      through OSC 7501 (idle, working, done,
                                      blocked, error). 124 on timeout
  attach PANE                         attach this terminal (Ctrl-\\ detaches)
  kill PANE [1|15|9]                  signal a pane, or remove a dead one

  PANE is a pane id, the start of one, or a name given with --name.

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
            "ls" | "run" | "send" | "read" | "wait" => return crate::panes::main(&command, args),
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
    use shelldeck_sessiond::pane_cli::{exit_status, panes_json, panes_table, resolve_pane};
    use shelldeck_sessiond::paths;
    use shelldeck_sessiond::protocol::PaneInfo;
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
        let dir = paths::resolve_run_dir(args.dir.as_deref()).map_err(|e| e.to_string())?;
        // Only `run` starts a daemon: the other commands act on panes that
        // exist, and there are none without one.
        let client = connect(&dir, command == "run").await?;
        match command {
            "ls" => {
                let panes = client.list_panes().await.map_err(|e| e.to_string())?;
                if args.json {
                    println!("{}", panes_json(&panes));
                } else {
                    print!("{}", panes_table(&panes));
                }
                Ok(0)
            }
            "run" => start(&client, &args).await,
            _ => {
                let panes = client.list_panes().await.map_err(|e| e.to_string())?;
                let selector = args.positional.first().map(String::as_str).unwrap_or("");
                let id = resolve_pane(selector, &panes)?;
                let Some(info) = panes.iter().find(|info| info.pane == id) else {
                    return Err(format!("no pane {id}"));
                };
                match command {
                    "send" => send(&client, info, &args).await,
                    "read" => read(&client, info, &args).await,
                    _ => wait(&client, info, &args).await,
                }
            }
        }
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
            if name == "COLORTERM" || name == "LANG" || name.starts_with("LC_") {
                env.push((name, value));
            }
        }
        env
    }

    async fn start(client: &Client, args: &Args) -> Result<u8, String> {
        // Like a shell: the pane starts where the caller is.
        let cwd = match &args.cwd {
            Some(cwd) => cwd.clone(),
            None => std::env::current_dir()
                .ok()
                .and_then(|dir| dir.to_str().map(str::to_string))
                .unwrap_or_default(),
        };
        let spawned = client
            .spawn(SpawnOptions {
                label: args.name.clone().unwrap_or_default(),
                rows: args.rows.unwrap_or(40),
                cols: args.cols.unwrap_or(120),
                cwd,
                argv: args.trailing.clone(),
                env: pane_env(),
                ..SpawnOptions::default()
            })
            .await
            .map_err(|e| format!("cannot start the pane: {e}"))?;
        println!("{}", spawned.pane);
        Ok(0)
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

        // Input needs an attached, writing client. The first event is the
        // screen, which also says whether the program takes pastes whole.
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
        for chunk in bytes.chunks(4096) {
            client.input(info.pane, chunk).map_err(|e| e.to_string())?;
        }
        // One round trip, so the input frames are delivered before exit.
        client.list_panes().await.map_err(|e| e.to_string())?;
        Ok(0)
    }

    async fn read(client: &Client, info: &PaneInfo, args: &Args) -> Result<u8, String> {
        let lines = args.lines.unwrap_or(DEFAULT_LINES);
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
        let text = mirror.grid().lock().tail_text(lines).join("\n");
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
        if !info.alive {
            return Ok(exit_status(info.exit_code, 0));
        }
        let deadline = args
            .timeout
            .map(|seconds| Instant::now() + Duration::from_secs(seconds));
        let (mut events, mut mirror) = attach(client, info, 0, true)?;
        loop {
            let event = match deadline {
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    match tokio::time::timeout(left, events.next()).await {
                        Ok(event) => event,
                        Err(_) => {
                            eprintln!("shelldeck-sessiond: timed out waiting for {}", info.pane);
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
                return Ok(exit_status(exit_code, signal));
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
                    return Ok(0);
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
