//! Provider-free native fixture using the same GPUI feature set as the app.
#[path = "../../shelldeck-ui/examples/agent_console_preview.rs"]
mod preview;

fn main() -> anyhow::Result<()> {
    preview::main()
}
