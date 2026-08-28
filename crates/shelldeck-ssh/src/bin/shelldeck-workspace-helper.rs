#[cfg(unix)]
fn main() -> std::process::ExitCode {
    let config = match shelldeck_ssh::workspace_helper::remote::RemoteHelperConfig::from_args(
        std::env::args(),
    ) {
        Ok(config) => config,
        Err(_) => return std::process::ExitCode::FAILURE,
    };
    shelldeck_ssh::workspace_helper::remote::run_stdio(config)
}

#[cfg(not(unix))]
fn main() -> std::process::ExitCode {
    std::process::ExitCode::FAILURE
}
