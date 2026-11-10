//! Opt-in full Google Chrome preview. The embedded CEF backend stays portable.
use crate::{BrowserClient, BrowserLaunch};
use std::{
    io,
    path::PathBuf,
    process::{Command, Stdio},
};

pub fn spawn(profile_dir: PathBuf) -> io::Result<BrowserClient> {
    if !cfg!(target_os = "linux") {
        return Err(io::Error::new(io::ErrorKind::Unsupported,
            "The full Google Chrome preview currently requires Linux. Use built-in Chromium on this platform."));
    }
    if !profile_dir.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Chrome requires an absolute app-owned profile",
        ));
    }
    std::fs::create_dir_all(&profile_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&profile_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let launch = BrowserLaunch {
        executable: PathBuf::from("python3"),
        profile_dir,
    };
    let mut process = Command::new("python3");
    process
        .arg("-u")
        .arg("-c")
        .arg(include_str!("chrome_wrapper.py"))
        .arg("--profile")
        .arg(&launch.profile_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    BrowserClient::spawn_process(launch, process)
}
