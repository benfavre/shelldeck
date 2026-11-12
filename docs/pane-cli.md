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
```

The binary is installed beside `shelldeck` (Linux and macOS). The commands
talk to the daemon of the current user through its private socket; nothing is
reachable from another account or from the network.

## Commands

| Command | What it does |
|---|---|
| `ls [--json]` | List the panes: id, name, state, size, attached clients, title. `--json` prints an array, one pane per line, for programs. |
| `run [--name NAME] [--cwd DIR] [--rows N] [--cols N] [-- ARGV...]` | Start a pane and print its id. The login shell by default; the program after `--` otherwise. Starts in the caller's directory. Default size 40 x 120. Starts the daemon when none runs. |
| `send PANE [--no-enter] [TEXT...]` | Type TEXT into the pane, then Enter. Without TEXT the text is read from stdin. Put `--` before text that starts with a dash. |
| `read PANE [--lines N]` | Print the last N lines of the pane (40 by default), history included, ending at the last line that has text. |
| `wait PANE [--until STATE] [--timeout SECONDS]` | Wait until the pane's program exits and exit with its status. With `--until`, return 0 as soon as the program reports STATE (`idle`, `working`, `done`, `blocked`, `error`). Exit status 124 when the timeout runs out. |
| `attach PANE` | Attach this terminal to the pane. `Ctrl-\` detaches. |
| `kill PANE [1\|15\|9]` | Signal the pane's program (15 by default), or remove a pane that already ended. |

`PANE` is a pane id, the start of one (at least 4 characters) or a name given
with `--name`. A selector that matches several panes is refused: these
commands type into panes and signal them, so they never guess.

Every command takes `--dir DIR` to use another run directory (tests).

## Notes

- **Several lines.** `send` passes several lines as one paste when the
  program in the pane accepts pastes (bracketed paste, which agent interfaces
  enable). Otherwise each line break is typed as Enter.
- **Sizes.** `send`, `read` and `wait` attach without asking for a size: a
  pane keeps the size its viewers gave it.
- **Status.** `wait --until` relies on the program reporting its status
  through OSC 7501 (see `docs/testing/USE_CASES.md`, SDUC-576). It sees
  reports made after `wait` started: a status reached before is not known,
  because the status is not kept by the daemon. A program that reports
  nothing can only be waited for until it exits.
- **Exit status.** `wait` exits with the pane's exit code, or 128 plus the
  signal number when a signal ended it. For a pane that already ended it
  answers at once.
- **In the application.** A pane started here is a pane like the others: with
  the session daemon enabled in Settings, it comes back as a tab the next
  time the application starts. A running application does not show it until
  then.

## What is not there

- A pane does not know its own id: there is no variable for a helper to
  report back to the pane that started it. Pass the id or the name along.
- `send` does not refuse to type into a program that reported `blocked`
  on a permission question. Do not use it to answer one on the user's behalf.
- Windows: the daemon is Unix only.
