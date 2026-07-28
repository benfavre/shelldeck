# Post-login splash

After Manage accepts a password or browser login, ShellDeck covers the complete
window while the first cloud-profile sync prepares the authenticated workspace.
The splash remains visible for at least 3 seconds, then closes whether the sync
succeeds or fails; the existing toast reports the result.

The mascot animation uses the production Monolith expression SVGs from
`crates/shelldeck/assets/images/brand/svg/expressions/`. The default and wink
expressions are layered in GPUI, with a short blink and gentle floating motion.
Keeping the artwork vector-native makes it resolution-independent and avoids a
second rasterized or distorted mascot source.

The progress bar and numeric percentage advance together from 0 to 100% over
the three-second minimum display period. A monotonic staged curve simulates
discovery, profile preparation, and finalization with natural changes of speed
instead of a perfectly linear fill. If the network sync takes longer, both
remain at 100% until the operation finishes.

Once loading finishes, the full splash fades out over 380ms before it is
unmounted. On first login, onboarding opens only after that transition has
completed.

First-run onboarding is opened only after the splash closes, so the two
full-window transitions never compete visually.
