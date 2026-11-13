# Changing keyboard shortcuts

The application shortcuts can be changed, or removed, in a `[keybindings]`
table of the configuration file: `config.toml` in the Inklura Desktop
configuration directory (`~/.config/shelldeck/config.toml` on Linux). The
table is read when the application starts, so restart after editing it.

```toml
[keybindings]
next_view = "secondary-alt-v"   # change a shortcut
toggle_sidebar = ""             # remove one (here: leaves Ctrl+B to tmux)
```

Each entry is `action = "keys"`. An empty value removes the shortcut; the
action stays available from the menus and the command palette. An action that
is not listed keeps its built-in keys. Two actions have none until you give
them some: `next_project` and `previous_project`.

## Writing keys

Modifiers and the key are joined with `-`: `ctrl-shift-l`, `alt-[`,
`secondary-,`. `secondary` is Cmd on macOS and Ctrl elsewhere, which is what
the built-in shortcuts use. Other modifiers: `ctrl`, `alt`, `shift`, `cmd`.
The order of the modifiers and upper or lower case do not matter.

## Actions

| Action | Built-in keys | What it does |
|---|---|---|
| `command_palette` | `secondary-shift-p` | Open the command palette |
| `quick_connect` | `secondary-k` | New connection |
| `ai_assistant` | `secondary-shift-k` | Open the assistant |
| `settings` | `secondary-,` | Open Settings |
| `next_view` | `secondary-shift-l` | Next view: full application, Agents, Terminal, Editor |
| `toggle_sidebar` | `secondary-b` | Show or hide the sidebar (the session list in Agents and in Terminal) |
| `toggle_menu_bar` | `secondary-shift-m` | Show or hide the menu row |
| `file_editor` | `secondary-e` | Open the file editor |
| `new_terminal` | `secondary-t` | New terminal tab |
| `close_tab` | `secondary-w` | Close the terminal tab |
| `next_tab` | `ctrl-tab` | Next terminal tab |
| `previous_tab` | `ctrl-shift-tab` | Previous terminal tab |
| `next_project` | none | Next project: the terminal used last in the next project of the session list |
| `previous_project` | none | Previous project |
| `split_horizontal` | `ctrl-shift-d` (`cmd-d` on macOS) | Split the terminal |
| `split_vertical` | `ctrl-shift-alt-d` (`cmd-shift-d` on macOS) | Split the terminal the other way |
| `toggle_split_focus` | `alt-[` | Move between the panes of a split |
| `maximize_pane` | `secondary-shift-enter` | Show the focused pane of a split alone, or bring the split back |
| `copy` | `ctrl-shift-c` (`cmd-c` on macOS) | Copy the terminal selection |
| `paste` | `ctrl-shift-v` (`cmd-v` on macOS) | Paste into the terminal |
| `search` | `secondary-f` | Search in the terminal |
| `clear_terminal` | `secondary-l` | Clear the terminal |
| `zoom_in` | `secondary-=` | Larger terminal text |
| `zoom_out` | `secondary--` | Smaller terminal text |
| `zoom_reset` | `secondary-0` | Default terminal text size |
| `quit` | `secondary-q` | Quit |

## When an entry is not applied

The application tells you at startup, in a notice, and keeps the built-in keys:

- the action name is not in the table above;
- the keys cannot be read;
- the keys are already used by another action. Move or remove that other
  action in the same table and the change is accepted. Two actions never
  share keys.

Menus, the command palette, tooltips and the shortcut lists show the keys in
effect, so a changed shortcut is shown with its new keys and a removed one
with none.

Not covered by the table: keys inside the Agents composer, the file editor
and the embedded browser, and the global shortcuts of the companion (set in
Settings).
