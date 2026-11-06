//! Launch the existing PDF application through its installed identity. An
//! absent installation opens the official download page; no PDFs are uploaded.
use std::{
    path::{Path, PathBuf},
    process::Command,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PdfDestination {
    Application,
    Downloads,
}
trait Launcher {
    fn exists(&self, path: &Path) -> bool;
    fn run(&mut self, program: &Path, args: &[&str], wait: bool) -> bool;
    fn downloads(&mut self) -> crate::error::Result<()>;
}
struct SystemLauncher;
impl Launcher for SystemLauncher {
    fn exists(&self, path: &Path) -> bool {
        path.is_file()
    }
    fn run(&mut self, program: &Path, args: &[&str], wait: bool) -> bool {
        let mut command = Command::new(program);
        command.args(args);
        if wait {
            command.status().is_ok_and(|status| status.success())
        } else {
            command.spawn().is_ok()
        }
    }
    fn downloads(&mut self) -> crate::error::Result<()> {
        super::cloud_account::open_in_browser("https://pdf.inklura.fr/")
    }
}
fn launch(
    os: &str,
    candidates: &[PathBuf],
    desktop: Option<&Path>,
    launcher: &mut impl Launcher,
) -> crate::error::Result<PdfDestination> {
    let launched = match os {
        "macos" => launcher.run(Path::new("/usr/bin/open"), &["-a", "Inklura PDF"], true),
        "windows" => candidates
            .iter()
            .any(|path| launcher.exists(path) && launcher.run(path, &[], false)),
        _ => {
            desktop.is_some_and(|path| {
                launcher.exists(path)
                    && launcher.run(Path::new("gtk-launch"), &["com.benfavre.caviard"], true)
            }) || launcher.run(Path::new("caviard"), &[], false)
        }
    };
    if launched {
        return Ok(PdfDestination::Application);
    }
    launcher.downloads()?;
    Ok(PdfDestination::Downloads)
}
pub fn open_pdf_editor() -> crate::error::Result<PdfDestination> {
    let candidates = [
        std::env::var_os("LOCALAPPDATA")
            .map(|root| PathBuf::from(root).join("Programs/Inklura PDF/Inklura PDF.exe")),
        std::env::var_os("LOCALAPPDATA")
            .map(|root| PathBuf::from(root).join("Programs/caviard-pdf/Inklura PDF.exe")),
        std::env::var_os("ProgramFiles")
            .map(|root| PathBuf::from(root).join("Inklura PDF/Inklura PDF.exe")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let desktop = directories::BaseDirs::new().map(|dirs| {
        dirs.data_dir()
            .join("applications/com.benfavre.caviard.desktop")
    });
    launch(
        std::env::consts::OS,
        &candidates,
        desktop.as_deref(),
        &mut SystemLauncher,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        installed: bool,
        success: bool,
        calls: Vec<(PathBuf, Vec<String>, bool)>,
        downloads: usize,
    }
    impl Launcher for Fake {
        fn exists(&self, _: &Path) -> bool {
            self.installed
        }
        fn run(&mut self, program: &Path, args: &[&str], wait: bool) -> bool {
            self.calls.push((
                program.into(),
                args.iter().map(|s| s.to_string()).collect(),
                wait,
            ));
            self.success
        }
        fn downloads(&mut self) -> crate::error::Result<()> {
            self.downloads += 1;
            Ok(())
        }
    }
    // SDTEST-2895 / SDUC-569 — no real process or browser is started.
    #[test]
    fn sdtest_2895_pdf_uses_installed_platform_identity_and_reports_download_fallback() {
        for (os, program, args, wait) in [
            ("macos", "/usr/bin/open", vec!["-a", "Inklura PDF"], true),
            ("windows", "installed/Inklura PDF.exe", vec![], false),
            ("linux", "gtk-launch", vec!["com.benfavre.caviard"], true),
        ] {
            let mut fake = Fake {
                installed: true,
                success: true,
                calls: vec![],
                downloads: 0,
            };
            assert_eq!(
                launch(
                    os,
                    &[PathBuf::from("installed/Inklura PDF.exe")],
                    Some(Path::new("desktop-entry")),
                    &mut fake
                )
                .unwrap(),
                PdfDestination::Application
            );
            assert_eq!(
                fake.calls,
                vec![(
                    PathBuf::from(program),
                    args.into_iter().map(str::to_string).collect(),
                    wait
                )]
            );
            assert_eq!(fake.downloads, 0);
            fake.success = false;
            fake.calls.clear();
            assert_eq!(
                launch(os, &[], None, &mut fake).unwrap(),
                PdfDestination::Downloads
            );
            assert_eq!(fake.downloads, 1);
        }
    }
}
