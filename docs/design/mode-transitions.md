# Mode transitions

Switching between User, Support, and Dev uses a full-window loading transition:

1. A full-window loader fades over the current mode for 180ms.
2. ShellDeck changes and persists the mode while the loader displays
   “Switching to … mode” for 2.54 seconds.
3. The loader fades away over 280ms to reveal the destination mode.

Each destination gets a distinct production Monolith personality derived from
`docs/design/monolith-animations.html`:

- User combines “Breathe” and a soft orbit as a friendly floating companion.
- Support combines “Working / busy”, “Progress ring”, and “Scan line”.
- Dev uses the neutral expression with the “Terminal typing” fill and caret.

GPUI does not execute CSS keyframes embedded in SVG images, so each motion is
reproduced with `with_animation` over the static expression assets.

Dev entities are hidden rather than destroyed, so terminal sessions continue
running throughout the transition. Rapid repeated switches are ignored during
the 3-second sequence, and logout cancels any transition still in progress.
