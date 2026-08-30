# Dependency advisory baseline

ShellDeck treats RustSec vulnerabilities and informational dependency debt as
two different signals:

- a newly reported vulnerability must fail the security check;
- known unmaintained or narrowly unsound transitive dependencies stay in one
  reviewed baseline until their owning subsystem can migrate.

The same advisory IDs are listed in `.cargo/audit.toml`, `deny.toml`, and the
`rustsec/audit-check` input in `.github/workflows/security.yml`. The explicit
workflow list matters because a scheduled `audit-check` run creates one GitHub
issue per installed package occurrence. That produced duplicate issues for the
three then-installed `ttf-parser` versions and the two then-installed
`rustybuzz` versions on 2026-08-24.

## Resolved during the 2026-08-27 review

| Advisory / warning | Resolution |
| --- | --- |
| RUSTSEC-2026-0221 (`event-listener` 5.4.1) | Lockfile updated to 5.4.2, the patched release. |
| RUSTSEC-2017-0008 (`serial` 0.4.0) | `portable-pty` updated from 0.8 to 0.9, which migrated to `serial2`. |
| Yanked `spin` 0.9.8 | Lockfile updated to the compatible 0.9.9 release. |

## Resolved during the 2026-08-28 review

| Advisory / warning | Resolution |
| --- | --- |
| RUSTSEC-2021-0139 (`ansi_term`), RUSTSEC-2021-0145 and RUSTSEC-2024-0375 (`atty`) | Vendored GPUI migrated its Linux status item from `ksni` 0.2 to 0.3, removing the `dbus-codegen` -> Clap 2 dependency chain. |
| RUSTSEC-2026-0194 and RUSTSEC-2026-0195 (`quick-xml` 0.30) | `xcb` updated from 1.7.0 to 1.7.1, which uses `quick-xml` 0.41. |
| RUSTSEC-2026-0206 (`rustybuzz`) | Vendored GPUI moved to `cosmic-text` 0.19 and `resvg`/`usvg` 0.48, replacing both `rustybuzz` branches with `harfrust`. |
| Two obsolete RUSTSEC-2026-0192 (`ttf-parser`) occurrences | The rendering upgrades removed versions 0.20 and 0.21. Version 0.25 remains through current `cosmic-text` -> `fontdb`; `lopdf` only declares it behind an unused optional feature. |
| Yanked `chacha20` 0.10.1 | Lockfile updated to the compatible 0.10.2 release. |
| RUSTSEC-2026-0173 (`proc-macro-error2`) | Vendored GPUI moved from `stacksafe` 0.1 to 1.0, whose procedural macro uses maintained Syn diagnostics. |

## Resolved during the 2026-08-30 review

Nothing. This pass re-tested all six remaining entries against the live registry
and the live upstream trackers rather than against the previous write-up, on the
assumption that a blocker recorded days ago may have quietly cleared. None had.
No advisory ID was removed, none was added, and no dependency was upgraded.

The one substantive change is a correction, not progress: the claim that the
`adabraka_*` advisories had "no upstream tracker to file against" was wrong, and
is fixed under RUSTSEC-2024-0384 below. The real blocker there is stronger than
the one previously recorded.

## Reviewed transitive baseline

| Owner / migration boundary | Advisory IDs | Current dependency path |
| --- | --- | --- |
| Vendored GPUI runtime and HTTP stack | RUSTSEC-2024-0384, RUSTSEC-2025-0052, RUSTSEC-2025-0134 | `adabraka_util` and `adabraka_http_client` 0.5.1, the two sibling crates of the vendored GPUI that we do **not** vendor. They pin `futures-lite` 1, `async-tar` 0.5 and `zed-reqwest`. |
| Rendering and font stack | RUSTSEC-2024-0436, RUSTSEC-2026-0192 | `paste` arrives through three independent chains (`metal`, `pulp`/`exr`, `rav1e`/`ravif`); `ttf-parser` 0.25 arrives only through `cosmic-text` 0.19 -> `fontdb` 0.23. |
| SSH compatibility | RUSTSEC-2023-0071 | `rsa` is transitive through `russh`'s default `rsa` feature; the advisory records no patched release at all. |

## Why each remaining ID is still blocked

Re-verified on **2026-08-30** against the committed `Cargo.lock`. `cargo audit`
run without this repository's ignore list reports exactly these six IDs and
nothing else, so no baseline entry is stale and none was widened. Every claim
below was checked against the crates.io sparse index, not from memory.

Two whole-graph checks back that up, and both are cheap to repeat:

- `cargo tree -i <package> --target all` still resolves every path recorded
  below, unchanged.
- `cargo update --dry-run` over the whole workspace moves **nothing** in any of
  the six chains. The single chain crate it touches at all is `exr`
  1.74.1 -> 1.74.2, which still requires `pulp ^0.22.3` and therefore still
  reaches `paste`. No semver-compatible release exists that removes any of the
  six. Do not act on that dry run: it also proposes `pathfinder_simd`
  0.5.5 -> 0.5.6, which breaks the macOS release build against our pinned
  nightly (see the pin note in `AGENTS.md`).

### RUSTSEC-2024-0384: `instant` 0.1.13 (unmaintained)

```
instant 0.1.13 <- fastrand 1.9.0 <- futures-lite 1.13.0 <- adabraka_util 0.5.1
```

`adabraka_util` declares `futures-lite = "^1.13"` as a **non-optional**
dependency, so no feature switch removes it. `futures-lite` 1.13.0 (2023-04-07)
is the last 1.x and pins `fastrand ^1.9`; `fastrand` 1.9.0 (2023-02-14) is the
last 1.x and the version that still carries `instant`. `futures-lite` 2 dropped
it, so the whole fix is "`adabraka_util` moves to `futures-lite` 2".

**Blocked because we do not own that crate.** Only `adabraka-gpui` and
`adabraka-ui` are vendored under `patches/`; `adabraka_util` is a plain
crates.io dependency, and in the vendored fork's manifest it is the
**non-optional** `[dependencies.util]` entry, so no feature switch on our side
reaches it either. GPUI uses it across 46 files (`util::ResultExt`,
`util::defer`, `util::maybe`, ...), so it is not droppable in our code.
0.5.1 (2026-02-17) is still the newest release of every `adabraka_*` crate.

**Upstream is abandoned, not merely unreachable.** The 2026-08-29 revision of
this note said there was "no upstream tracker to file against". That is
overstated and is corrected here. `https://github.com/Augani/adabraka-gpui`
does still return 404 (re-checked 2026-08-30, over HTTP and via `gh api`), but
the same author's `Augani/adabraka-ui` is live, unarchived and has issues
enabled. [`Augani/adabraka-ui#10`](https://github.com/Augani/adabraka-ui/issues/10)
("[BUG] adabraka-gpui source is unavailable", open since 2026-06-04) already
tracks exactly the missing-source problem, and a commenter states that "the
kael project supersedes this". `kael` is not published on crates.io, and the
repository's last push was 2026-02-19.

So a tracker does exist, but filing there would not help: the crate line is
declared superseded and has shipped nothing for six months. That is a stronger
reason to keep the exception than the "no tracker" claim it replaces. The only
public source snapshot (`philippremy/adabraka-gpui`) has issues disabled and
mirrors 0.5.1 with the same `futures-lite = "1.13"` constraint, confirmed by
reading its `crates/util/Cargo.toml`, so no unpublished fix is waiting either.

**Why no workaround.** Changing the constraint means vendoring a third crate or
adding a `[patch.crates-io]` entry. Those are steps 4 and 5 of the decision
ladder in [`.agents/patches.md`](../.agents/patches.md), and both require
explicit human sign-off. Taking on a fourth long-lived fork to silence one
*unmaintained* informational warning on a crate that only provides a WASM
`Instant` shim is a far worse trade than the documented exception.

### RUSTSEC-2025-0052: `async-std` 1.13.2 (discontinued)

```
async-std 1.13.2 <- async-tar 0.5.1 <- adabraka_http_client 0.5.1
```

`adabraka_http_client` declares `async-tar = "^0.5.1"` non-optional, and
`async-tar` 0.5.1 declares `async-std` non-optional with no feature gate at all.
`adabraka_http_client` is itself the non-optional `[dependencies.http_client]`
entry of the vendored fork, and GPUI uses it directly (`http_client::AsyncBody`,
`Request`, `Response`, `Uri`, ...), so there is no feature to switch off on our
side of the boundary either. The same holds for RUSTSEC-2025-0134 below, which
enters through the same crate.

**The upstream migration exists but is out of reach.** `async-tar` 0.6.0 /
0.6.1 (2026-01 / 2026-06) made `async-std` optional behind `runtime-async-std`
and added a `runtime-tokio` alternative. Adopting it requires
`adabraka_http_client` to move to `async-tar` 0.6 with
`default-features = false, features = ["runtime-tokio"]`; a `^0.5.1`
requirement cannot resolve to 0.6. Same ownership problem as above.

### RUSTSEC-2025-0134: `rustls-pemfile` 2.2.0 (unmaintained)

```
rustls-pemfile 2.2.0 <- zed-reqwest 0.12.15-zed <- adabraka_http_client 0.5.1
```

Upstream `reqwest` already solved this: the workspace's own `reqwest` 0.12.28
does **not** pull `rustls-pemfile`, which is why `cargo tree -i rustls-pemfile`
shows a single branch. In `zed-reqwest` the dependency is optional but enabled
by `__tls`, which every TLS feature turns on, so it is present in practice.

**Blocked on a fork that stopped publishing.** `zed-reqwest` has exactly two
releases, both from 2025-10-05; `0.12.15-zed` is a snapshot of reqwest 0.12.15
and nothing newer exists. This clears when `adabraka_http_client` depends on
upstream `reqwest` instead of the Zed fork, or when Zed publishes a rebased
`zed-reqwest`.

### RUSTSEC-2024-0436: `paste` 1.0.15 (unmaintained)

Three independent chains, each still declaring `paste` at its newest upstream
release:

| Chain | Latest checked | Still declares `paste`? |
| --- | --- | --- |
| `metal` <- GPUI macOS renderer (also `adabraka_media`, `core-video`) | 0.33.0 (2025-12-17) | yes, `paste ^1`, non-optional, in 0.29 through 0.33 |
| `pulp` <- `exr` <- `image` 0.25.10 | `pulp` 0.22.3 (2026-06-20) | yes, `paste ^1`; `exr` 1.74.2 still requires `pulp ^0.22.3` |
| `rav1e` <- `ravif` <- `image` 0.25.10 | 0.8.1 (2025-06-16) | yes, `paste ^1.0` |

Our lockfile sits *behind* two of those, for two different reasons, and neither
gap matters here:

- `metal` 0.29.0, because the vendored fork's manifest requires `metal = "0.29"`.
  Reaching 0.33 is a manifest edit across a semver-major renderer boundary, and
  `paste ^1` is non-optional in 0.29 and 0.33 alike, so it would buy nothing.
- `exr` 1.74.1, which is plain lockfile lag. 1.74.2 is semver-compatible and a
  `cargo update` would take it, but it still requires `pulp ^0.22.3`.

`paste` itself is frozen at 1.0.15 with no successor release.

The advisory suggests `pastey` as a drop-in fork, but adopting it is `metal`'s,
`pulp`'s and `rav1e`'s decision, not ours.

**Why no workaround.** The vendored GPUI takes `image` with default features,
so setting `default-features = false` there would drop the EXR and AVIF
chains, but not the `metal` one. `cargo audit` reads `Cargo.lock`, which is
target-agnostic, so the ID would keep firing while GPUI silently lost image
formats it advertises. Making `metal` optional, or moving macOS to the
`macos-blade` feature, is a renderer swap rather than a dependency bump. Both
are strictly worse than the documented exception.

### RUSTSEC-2026-0192: `ttf-parser` 0.25.1 (unmaintained)

```
ttf-parser 0.25.1 <- fontdb 0.23.0 <- cosmic-text 0.19.0 <- vendored adabraka-gpui
```

`cosmic-text` is a Linux/FreeBSD target dependency of the fork, but the
lockfile that `cargo audit` reads is target-agnostic, so the ID fires on
every platform. It is the only remaining `ttf-parser` occurrence: the shapers
already moved to `harfrust` + `skrifa`, and `lopdf` 0.44 declares `ttf-parser`
only behind an optional feature we do not enable.

**This is the closest ID to clearing, and it is one upstream merge away.**
`fontdb` 0.24.0 (2026-07-29) removed `ttf-parser` entirely (its index entry
declares no `ttf-parser` dependency at all), while `fontdb` 0.23.0 declares
`ttf-parser ^0.25` **non-optional**, so no feature switch removes it.
`cosmic-text` 0.19.0 (2026-04-22) is the newest release and pins `fontdb ^0.23`,
also non-optional, so neither a lockfile move nor `default-features = false`
can reach 0.24 today.

Re-checked on 2026-08-30, and the blocker is not stale:
[pop-os/cosmic-text#526](https://github.com/pop-os/cosmic-text/pull/526)
("chore: bump fontdb to 0.24") is **still open**, `merged: false`, last touched
2026-07-30. The bump has not landed by any other route either: `Cargo.toml` on
`cosmic-text` `main` still reads `fontdb = { version = "0.23", … }`, and the most
recent commit touching that file is the 0.19.0 release of 2026-04-22. There is
no 0.20 release, prerelease included.

**When it lands**: bump the `cosmic-text` constraint in
`patches/adabraka-gpui/Cargo.toml`, a manifest-only fork patch of the same
shape as SDPATCH-121; confirm with `cargo tree -i ttf-parser`, then remove the
ID from all three configuration locations and from this document.

### RUSTSEC-2023-0071: `rsa` 0.10.0-rc.16 (Marvin attack)

```
rsa 0.10.0-rc.16 <- russh 0.60.3 (default feature `rsa`)
                 <- internal-russh-forked-ssh-key 0.6.18 (via `ssh-key/rsa`)
```

**No fixed version exists.** Re-read from a freshly fetched advisory database on
2026-08-30, the advisory still records `patched = []`, and `cargo audit` prints
"No fixed upgrade is available!" for it.
[RustCrypto/RSA#626](https://github.com/RustCrypto/RSA/issues/626) ("Padding
implementation is not constant-time") is still open, last updated 2026-06-02,
and the constant-time rewrite has not shipped. `rsa` has never published a
stable 0.10: the registry holds only `0.10.0-rc.13` through `0.10.0-rc.18`.
Upgrading `russh` does not help either: 0.63.1 (2026-08-23) moved to
`rsa =0.10.0-rc.18`, which the advisory covers just the same. This is the one
baseline entry that cannot clear by any action of ours, ever, until upstream
ships the rewrite.

**Why no workaround.** The only way to drop `rsa` is to build `russh` with
`default-features = false` and leave the `rsa` feature off, which removes RSA
public-key authentication and `ssh-rsa` host keys. `shelldeck-ssh` implements
that path deliberately (`client.rs` negotiates `best_supported_rsa_hash()` for
`Algorithm::Rsa` and probes `~/.ssh/id_rsa`), and many production hosts still
accept only RSA user keys. Breaking them is a functional regression in the
app's core use case. The advisory's own workaround section scopes the risk to
settings where an attacker can observe signing timings; ShellDeck signs locally
with a user key rather than serving attacker-driven timing oracles.

## Maintenance rule

Do not add a new advisory to this baseline just to make CI green. First resolve
its full `cargo tree -i <package>@<version> --target all` path and document why
an in-place update or a ShellDeck-side change cannot remove it. Remove an ID
from all three configuration locations as soon as the dependency chain no
longer contains it.

## Re-running the review

`cargo audit` in the repository root reads `.cargo/audit.toml` and therefore
reports nothing while the baseline holds. To see the raw list, copy the
lockfile somewhere without that config and audit there:

```bash
export PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig
mkdir -p /tmp/sd-audit && cp Cargo.lock /tmp/sd-audit/
(cd /tmp/sd-audit && cargo audit)
```

Anything beyond the six IDs above is a genuinely new finding and must be
resolved, not appended here. `cargo update --dry-run --verbose` shows, per
crate, whether a newer compatible release exists. That is the fastest way to
confirm a baseline entry is still pinned by a requirement we do not control.

> **Dry run only.** Never let that command talk you into an actual
> `cargo update`. It proposes `pathfinder_simd` 0.5.5 -> 0.5.6, which breaks the
> macOS release build against our pinned nightly and which `cargo check` will
> not catch. If you ever do run it, follow immediately with
> `cargo update -p pathfinder_simd --precise 0.5.5`. See `AGENTS.md`.

Do not re-derive a blocker from this document alone. Each entry names an upstream
release, pull request or issue: re-check those at their source before repeating
the conclusion. A blocker that was real last week may have merged since, and the
whole point of this file is to make that cheap to test rather than cheap to
assume. Checking the six takes about ten minutes:

```bash
# still reachable, and by which path?
cargo tree -i <package> --target all
# does a newer release exist that changes the requirement?
curl -s https://index.crates.io/<a>/<b>/<crate> | jq -r '.vers'
# did the upstream PR/issue that gates it move?
gh api repos/<owner>/<repo>/pulls/<n> --jq '{state,merged,updated_at}'
```
