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
three installed `ttf-parser` versions and the two `rustybuzz` versions on
2026-08-24.

## Resolved during the 2026-08-27 review

| Advisory / warning | Resolution |
| --- | --- |
| RUSTSEC-2026-0221 (`event-listener` 5.4.1) | Lockfile updated to 5.4.2, the patched release. |
| RUSTSEC-2017-0008 (`serial` 0.4.0) | `portable-pty` updated from 0.8 to 0.9, which migrated to `serial2`. |
| Yanked `spin` 0.9.8 | Lockfile updated to the compatible 0.9.9 release. |

## Reviewed transitive baseline

| Owner / migration boundary | Advisory IDs | Current dependency path |
| --- | --- | --- |
| Linux status item in vendored GPUI | RUSTSEC-2021-0139, RUSTSEC-2021-0145, RUSTSEC-2024-0375 | `ksni` -> `dbus-codegen` -> Clap 2 -> `ansi_term` / `atty`. The `atty` unsoundness requires a custom Windows allocator; this path is built for Linux. |
| Linux GTK3 tray backend | RUSTSEC-2024-0370, RUSTSEC-2024-0412, RUSTSEC-2024-0413, RUSTSEC-2024-0415, RUSTSEC-2024-0416, RUSTSEC-2024-0418, RUSTSEC-2024-0419, RUSTSEC-2024-0420, RUSTSEC-2024-0429 | `tray-icon` / `libappindicator` still use GTK3. ShellDeck does not call the affected `glib::VariantStrIter`; removing the family requires a tray-backend migration rather than piecemeal crate bumps. |
| Vendored GPUI runtime and HTTP stack | RUSTSEC-2024-0384, RUSTSEC-2025-0052, RUSTSEC-2025-0134, RUSTSEC-2026-0173 | `adabraka_util`, `adabraka_http_client`, `zed-reqwest`, and `stacksafe`. These require a coordinated GPUI fork sync or upstream dependency migration. |
| Rendering and font stack | RUSTSEC-2024-0436, RUSTSEC-2026-0192, RUSTSEC-2026-0206 | `image`, `cosmic-text`, `usvg`, and `lopdf` pull `paste`, three `ttf-parser` versions, and two `rustybuzz` versions. Replacement means moving the owning renderers to maintained alternatives (`skrifa` / `harfrust`). |
| SSH compatibility | RUSTSEC-2023-0071 | `rsa` is transitive through `russh`; no patched RustSec release currently preserves the required RSA SSH-key path. |
| X11 code generation | RUSTSEC-2026-0194, RUSTSEC-2026-0195 | `quick-xml` 0.30 is a build-only dependency of `xcb` and parses trusted protocol XML. |

## Maintenance rule

Do not add a new advisory to this baseline just to make CI green. First resolve
its full `cargo tree -i <package>@<version> --target all` path and document why
an in-place update or a ShellDeck-side change cannot remove it. Remove an ID
from all three configuration locations as soon as the dependency chain no
longer contains it.
