# Inklura Desktop personal sharing verification — 2026-11-06

Implementation: SDUC-568 (personal Share), SDUC-569 (existing PDF app launcher).
The native customer surface is User mode → Share. Files, clipboard images and
annotated captures are staged locally. Share is always an explicit action.
The same account/selected shared account, personal gallery, quota, retention and
France storage gate are used on the web and desktop. Successful uploads copy
the last viewer link. ShareX is optional compatibility; no Share API key setup
is required for Desktop. Existing app, profile, updater and signing identities
are preserved.

Verified locally with disposable XDG profiles:

- Full existing UI library suite: 341 pass, 0 fail.
- New rendered Share interaction test: actual controls, bounded 20-file batch,
  immediately visible Send, clipboard link, delete cancellation, account switch
  and a late completion that must not restore old account data.
- Full core library suite: 521 pass, 0 fail, 7 existing opt-in tests ignored.
  Includes HTTP wire contract, hostile origins, multipart reservation cleanup,
  cancellation and auth/outage separation, plus all three PDF launcher plans
  through a fake process runner.
- Production application compiles with the pinned nightly and locked dependencies.
- Native GPU fixture inspected at wide and narrow sizes using an isolated
  profile. Only that fixture window was captured. Xvfb was unsuitable for GPU
  presentation (missing DRI3); it was stopped, then the fixture's own X11 window
  was rendered and captured. No personal desktop window or data was captured.

Live synthetic account verification on Manage and Share:

- Existing website upload is visible in Desktop's personal snapshot and quota.
- Grant replay and revocation-before-redemption fail. Desktop sessions cannot
  mint keys, manage a team or start billing. API route runtime context cannot be
  interpreted as a fetch implementation.
- Rust client persists retention, uploads/downloads an image exactly, uploads
  51 MiB through real signed multipart parts and verifies a full SHA-256 download.
- Personal gallery and quota agree; deletion restores baseline. Test uploads,
  device records and account preference/usage keys were cleaned. No mail or
  payment triggered. The smoke executable accepts only dedicated synthetic
  identities and reads credentials on stdin.

Server counterpart: `benfavre/bext` commit `8c82d9d22`,
`sites/share-prism/DESKTOP.md`. This API is deployed against the active shared
production checkout. Its existing Share pricing/storage baseline includes
uncommitted work tracked by bext#428; the scoped API commit does not promote or
overwrite that broader baseline.
Only Manage/Share site caches were invalidated; only the dedicated native upload
service was restarted for its imported scope validator. The shared server runtime
was not restarted. Strict device lookup distinguishes an unreadable KV store from
a revoked device. An already issued Share session has a bounded five-minute
lifetime after logout; native cancellation and result fencing apply immediately.

Remaining platform acceptance: actual screenshot picker/annotation interaction
and launching an installed PDF app on each OS. These reuse the existing capture
and annotation code. PDF launch plans and fallback are tested without starting
external applications; Windows/macOS binaries are covered by the existing CI
platform gates. Public marketing should advertise native Desktop sharing when
its Desktop release is published; ShareX documentation stays available.
