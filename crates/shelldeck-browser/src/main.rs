fn main() -> std::process::ExitCode {
    if std::env::args().any(|arg| arg == "--mcp") {
        return match shelldeck_browser::mcp::run_stdio_proxy() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Inklura Desktop browser bridge: {error}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    match shelldeck_browser::runtime::run() {
        Ok(code) => std::process::ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("Inklura Desktop browser: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
