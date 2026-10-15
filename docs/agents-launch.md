# Launch Agents directly

The installed ShellDeck application can open its Agents interface directly:

```bash
shelldeck --agents
```

This uses the production `shelldeck` executable, the same account, settings,
conversation library and native browser runtime as the full application.
The normal welcome, login and Dev capability checks still apply.

If ShellDeck is already running, the launch request is handed to the existing
application through its single-instance routing. It does not start a second
agent runtime. Launching `shelldeck` normally returns to the full ShellDeck
interface; retained agent conversations and running sessions keep their state.

## Installed launchers

- **Linux:** choose **Open Agents** from the ShellDeck desktop launcher's actions
  when supported by the desktop environment. The default action still opens
  ShellDeck, and `shelldeck://` URL handling remains registered to the normal
  application. An AppImage also accepts `./ShellDeck-x86_64.AppImage --agents`.
- **Windows:** choose **ShellDeck Agents** in the ShellDeck Start Menu folder.
  The NSIS installer and per-user PowerShell installer both create this shortcut
  with `--agents` as an argument to the main executable. The standard ShellDeck
  shortcut remains available.
- **macOS:** use the installed app bundle:

  ```bash
  open -na ShellDeck --args --agents
  ```

  `-n` makes Launch Services pass the command-line arguments even when ShellDeck
  is already open. ShellDeck's own single-instance handoff routes the request
  to its existing window. Run `open -na ShellDeck` to return to the full app.
  A separate wrapper app or second bundle is not required.

If the installer has just added ShellDeck to `PATH`, reopen the terminal first.
For a manually unpacked release, invoke the executable at its installed path.
The launch option requires an application build that includes this feature.

## Development and packaging

Run the actual application from the repository with:

```bash
PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig cargo run -p shelldeck -- --agents
```

The `PKG_CONFIG_PATH` prefix is for Linux; use `cargo run -p shelldeck -- --agents`
on macOS and Windows. Development still needs the native browser helper/runtime
when browser tools are used. See [native-browser.md](native-browser.md) for
runtime bundling and [the README](../README.md#build--run) for prerequisites.

Release archives/installers contain the same main executable. Linux AppImage
packaging also preserves its matching versioned browser payload, copies the
Agents desktop action and retains all AppRun arguments. Browser distribution
permissions are normalized before publication: directories and executable files
are accessible to installed application users, data is readable, and macOS
framework symlinks remain intact. Private browser profiles are separate and
retain their runtime permissions. Packaging refuses missing browser resources
and failed shared-library deployment.

Offline packaging checks:

```bash
python3 scripts/test-agents-packaging.py
python3 scripts/test-browser-packaging.py
bash -n packaging/linux/build-appimage.sh
bash -n packaging/windows/build-nsis.sh
desktop-file-validate packaging/linux/shelldeck.desktop
```

The Python tests use temporary payloads and fake AppImage tools.
The installer-rendering test needs Node with TypeScript stripping and NSIS;
Windows native shortcut activation and macOS Launch Services behavior still
need their platform release checks.

The `agents-packaging` Linux CI job installs Python 3.12, Node 24, NSIS and
`desktop-file-utils`, then runs SDTEST-2630..2632, browser archive and permission
contracts, and shell/desktop syntax checks. Its tool preflight prevents
SDTEST-2632 from silently skipping.

On Linux, `python3 scripts/test-agents-cli.py target/debug/shelldeck` checks the
built executable's help, version, startup failures and single-instance handoff
using a temporary XDG profile and no GUI.
