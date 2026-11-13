# Pane commands for scripts and agents

`shelldeck-sessiond` is the session daemon of Inklura Desktop: it owns
terminals (panes) that keep running when the application closes. The same
binary has commands to use those panes from a shell, so a script or a coding
agent can start a program in a pane that outlives it, type into it, read what
it shows and wait for it.

```bash
id=$(shelldeck-sessiond run --name reviewer -- claude)   # start a pane
shelldeck-sessiond send reviewer "Review the branch and list the risks."
shelldeck-sessiond wait reviewer --until done --timeout 900
shelldeck-sessiond read reviewer --lines 80

shelldeck-sessiond split --right -- npm run dev          # a pane beside this one
shelldeck-sessiond skill setup                           # teach all this to coding agents
```

The binary is installed beside `shelldeck` (Linux and macOS). The commands
talk to the daemon of the current user through its private socket; nothing is
reachable from another account or from the network.

## Inside a pane

Every pane the daemon starts is told who it is:

| Variable | Value |
|---|---|
| `SHELLDECK_PANE` | The id of the pane. `split` uses it when no pane is named. |
| `SHELLDECK_SESSIOND` | The path of this binary. |
| `PATH` | Ends with the directory of this binary, so `shelldeck-sessiond` is found. It is added last and shadows nothing. |

A shell that rebuilds `PATH` from scratch in its start files loses the last
entry; `$SHELLDECK_SESSIOND` still works. Panes of a daemon started by an
older version have none of the three until that daemon has ended.

## Commands

| Command | What it does |
|---|---|
| `ls [--json]` | List the panes: id, name, state, the status its program reported, size, attached clients, title. `--json` prints an array, one pane per line, for programs. |
| `run [--name NAME] [--cwd DIR] [--rows N] [--cols N] [--json] [-- ARGV...]` | Start a pane and print its id. The login shell by default; the program after `--` otherwise. Starts in the caller's directory. Default size 40 x 120. Starts the daemon when none runs. |
| `split [PANE] [--right\|--down] [--ratio R] [--name NAME] [--cwd DIR] [--json] [-- ARGV...]` | Start a pane and show it in the application beside PANE: at its right (default) or below it. Without PANE, beside the pane the command runs in. `R` is the share of the space the new pane takes, 0.1 to 0.9 (default 0.5). Prints the id of the new pane. |
| `send PANE [--no-enter] [--force] [TEXT...]` | Type TEXT into the pane, then Enter. Without TEXT the text is read from stdin. Put `--` before text that starts with a dash. Refused while the pane's program waits for a permission, see [Notes](#notes). |
| `send-key PANE [--force] KEY...` | Press keys, in order. See [Keys](#keys). Refused like `send`. |
| `read PANE [--lines N] [--format text\|html]` | Print the last N lines of the pane (40 by default), history included, ending at the last line that has text. `html` prints one `<pre>` that keeps colours, bold, italic, underline, inverse video and links. |
| `status PANE [--json]` | Print what the pane's program last reported about itself (`working 40%`, `blocked (permission): ...`), or `none`. |
| `wait PANE [--until STATE] [--timeout SECONDS] [--json]` | Wait until the pane's program exits and exit with its status. With `--until`, return 0 as soon as the program reports STATE (`idle`, `working`, `done`, `blocked`, `error`), at once when that is already its status. Exit status 124 when the timeout runs out. |
| `focus [PANE]` | Select the pane's tab in the application and give the pane the keyboard. |
| `rename [PANE] NAME` | Name the pane's tab. Inside a pane, `rename NAME` names its own tab. |
| `zoom [PANE]`, `unzoom [PANE]` | Show the pane alone in its tab, or the whole split again. |
| `events [--timeout SECONDS]` | Print one JSON object per line for everything that exists, then for every change, until interrupted. See [Events](#events). |
| `attach PANE` | Attach this terminal to the pane. `Ctrl-\` detaches. |
| `kill PANE [1\|15\|9]` | Signal the pane's program (15 by default), or remove a pane that already ended. |
| `skill setup [--agent NAME]...` | Write the skill that teaches these commands to coding agents. See [Agents](#agents). `skill show` prints it. |

`PANE` is a pane id, the start of one (at least 4 characters) or a name given
with `--name`. A selector that matches several panes is refused: these
commands type into panes and signal them, so they never guess.

Every command takes `--dir DIR` to use another run directory (tests).

## Keys

`send-key` takes `enter`, `tab`, `esc`, `space`, `backspace`, `delete`,
`insert`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`,
`f1` to `f12`, or one character. Each can have `ctrl-`, `alt-` and `shift-`
in front: `ctrl-c`, `alt-b`, `shift-tab`, `ctrl-right`. A name that is not a
key is refused and nothing is typed.

The arrows, Home and End are sent the way the program in the pane asked for
them (application cursor keys), as a terminal does.

## Splits

`split` starts its pane through the daemon, like `run`, then asks the
application to show it. The application places the pane beside the one
named, in whichever tab that one is, without changing the active tab or the
keyboard focus: the request usually comes from a program working in the
background.

The command waits about three seconds for the answer and says on stderr
when the pane is not shown: no application runs, persistent sessions are off
in Settings, or no tab shows the pane to split. The pane runs all the same,
and `--json` reports it as `"shown": false`. Local panes only: a pane on an
SSH server is started there, where no application reads the request.

## Focus, rename, zoom

These act on what the application shows, so the application does them: the
command leaves the same kind of request as `split` and waits about three
seconds for the answer. Without PANE they act on the pane the command runs
in. Exit status 0 when done; 1 with the reason otherwise (no application
runs, persistent sessions are off, no tab shows that pane, the pane is alone
in its tab for `zoom`).

- `focus` is the only one that changes the active tab. It moves the user's
  keyboard: use it when asked to show something, not as a side effect.
- `rename` names the tab, whichever pane of it is named. The name is kept
  like one typed in the application: the program's own title no longer
  replaces it. One line, 80 characters at most.
- `zoom` on a pane of the active tab also gives it the keyboard there,
  since the pane shown alone is the focused one. In a background tab
  nothing visible changes until the user goes to it.

## Events

`events` prints JSON lines. Each has `event` and `pane`:

| Event | Other members |
|---|---|
| `pane-started` | `name`, `title`, `cwd`, `rows`, `cols`, `alive`, `clients` |
| `pane-exited` | `exit_code` |
| `pane-removed` | |
| `title` | `title` |
| `cwd` | `cwd` |
| `resized` | `rows`, `cols` |
| `clients` | `clients` (how many viewers are attached) |
| `status` | `state` (null when cleared), `kind`, `progress`, `app`, `text` |

The first lines describe every pane that exists. The command looks at the
daemon four times a second: a status that came and went between two looks
is not reported.

## Agents

`skill setup` writes `inklura-panes/SKILL.md` into the skills directory of
each coding agent set up for the user:

| Agent | Directory |
|---|---|
| Claude Code | `~/.claude/skills` |
| Codex | `~/.codex/skills` |
| OpenCode | `~/.config/opencode/skills` |
| Pi | `~/.pi/agent/skills` |

`--agent NAME` (`claude`, `codex`, `opencode`, `pi`) writes it for that agent
whether its directory exists or not. Running it again replaces the file. No
hook is installed: the agents load the skill by its description.

## Notes

- **Several lines.** `send` passes several lines as one paste when the
  program in the pane accepts pastes (bracketed paste, which agent interfaces
  enable). Otherwise each line break is typed as Enter.
- **Sizes.** `send`, `send-key`, `read`, `status` and `wait` attach without
  asking for a size: a pane keeps the size its viewers gave it.
- **Status.** The status is what the program reports through OSC 7501 (see
  `docs/testing/USE_CASES.md`, SDUC-576). The daemon keeps the last report,
  so `status`, `ls` and `wait --until` know a status reached before they
  started. A program that reports nothing has the status `none` and can only
  be waited for until it exits. A daemon started by a version before this
  one does not answer: `status` says so, `ls` shows no status and
  `wait --until` only sees reports made after it started.
- **Exit status.** `wait` exits with the pane's exit code, or 128 plus the
  signal number when a signal ended it. For a pane that already ended it
  answers at once.
- **Permission prompts.** A program that reported `blocked` with the kind
  `permission` (a coding agent asking to run a command) is waiting for its
  user. `send` and `send-key` refuse to type into it and exit with 1;
  `--force` types all the same. A daemon of an older version reports no
  status, so nothing is refused there.
- **In the application.** A pane started with `run` is a pane like the
  others: with persistent sessions enabled in Settings, it comes back as a
  tab the next time the application starts. A running application does not
  show it until then; `split` is how a pane is shown at once.

## What is not there

- No scripting language. The application actions are `split`, `focus`,
  `rename`, `zoom` and `unzoom`; closing a tab, moving a pane and the rest
  are not there.
- `events` polls; it is not pushed by the daemon.
- Windows: the daemon is Unix only.
