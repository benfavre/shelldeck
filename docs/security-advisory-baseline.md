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

## Reviewed transitive baseline

| Owner / migration boundary | Advisory IDs | Current dependency path |
| --- | --- | --- |
| Vendored GPUI runtime and HTTP stack | RUSTSEC-2024-0384, RUSTSEC-2025-0052, RUSTSEC-2025-0134 | `adabraka_util`, `adabraka_http_client`, and `zed-reqwest`. These require a coordinated GPUI fork sync or upstream dependency migration. |
| Rendering and font stack | RUSTSEC-2024-0436, RUSTSEC-2026-0192 | `image` codecs and the macOS `metal` crate still pull `paste`; current `cosmic-text` uses `fontdb` 0.23, which still uses `ttf-parser` 0.25. `lopdf`'s optional `ttf-parser` feature is not enabled. The text and SVG shapers have already migrated to `skrifa` / `harfrust`. |
| SSH compatibility | RUSTSEC-2023-0071 | `rsa` is transitive through `russh`; no patched RustSec release currently preserves the required RSA SSH-key path. |

## Maintenance rule

Do not add a new advisory to this baseline just to make CI green. First resolve
its full `cargo tree -i <package>@<version> --target all` path and document why
an in-place update or a ShellDeck-side change cannot remove it. Remove an ID
from all three configuration locations as soon as the dependency chain no
longer contains it.
