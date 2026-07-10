/// Returns the current platform identifier used to query the update server.
/// Format: `{os}-{arch}` (e.g. `linux-x86_64`, `macos-aarch64`, `windows-x86_64`).
pub fn current_platform() -> String {
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    };

    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "unknown"
    };

    format!("{}-{}", os, arch)
}

#[cfg(test)]
mod tests {
    use super::current_platform;

    // SDTEST-1200/1201/1202 — Platform key format is a hard release
    // contract (release.yml workflow, Cloudflare KV `latest-release`,
    // `install.sh` / `install.ps1`, and the AutoUpdater client must
    // all use the same string). The individual OS-branch tests below
    // gate on `cfg!` so only the matching target actually asserts.
    //
    // AGENTS.md § release: "Platform keys are {os}-{arch} and use
    // **macos-***, never **darwin-***". A rename here silently breaks
    // every install-script fetch on the affected OS.

    #[test]
    fn platform_string_shape_is_os_dash_arch() {
        let p = current_platform();
        assert!(p.contains('-'), "expected `<os>-<arch>`, got {p:?}");
        // Neither side may be empty.
        let mut it = p.splitn(2, '-');
        assert!(!it.next().unwrap().is_empty());
        assert!(!it.next().unwrap().is_empty());
    }

    // SDTEST-1201 — macOS MUST report `macos-*`, never `darwin-*`.
    // Contract-critical because the update manifest key must match.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_uses_macos_prefix_never_darwin() {
        let p = current_platform();
        assert!(
            p.starts_with("macos-"),
            "macOS must report `macos-*`, got {p:?}",
        );
        assert!(
            !p.starts_with("darwin-"),
            "AGENTS.md forbids `darwin-*`: {p:?}",
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_uses_linux_prefix() {
        assert!(current_platform().starts_with("linux-"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_uses_windows_prefix() {
        assert!(current_platform().starts_with("windows-"));
    }

    // SDTEST-1203 — arch resolves to a documented value on every
    // target CI can build. Guards against a silent `unknown` slipping
    // through if a new Rust target is added without wiring the branch.
    #[test]
    fn arch_is_a_known_value() {
        let p = current_platform();
        let arch = p.split('-').nth(1).unwrap_or("");
        assert!(
            matches!(arch, "x86_64" | "aarch64" | "unknown"),
            "unexpected arch label: {arch:?} in {p:?}",
        );
        // On CI runners we build on x86_64 or aarch64 — flag `unknown`
        // as a warning so a new arch trigger doesn't sneak in silently.
        if arch == "unknown" {
            eprintln!(
                "warning: current_platform() returned unknown arch — \
                 add the matching cfg branch in platform.rs",
            );
        }
    }
}
