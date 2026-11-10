//! Explicit read-only desktop discovery/history probe. Prints counts, never transcripts or IDs.
use shelldeck_core::{
    agent_local_sessions::{discover, read_history, LocalSessionRoots},
    agent_remote_history::RemoteHistoryQuery,
    agent_runtime::AgentProvider,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let roots = LocalSessionRoots::from_environment()?;
    let start = std::time::Instant::now();
    let page = discover(&roots, None)?;
    println!(
        "local inventory: sessions={}, older={}, warnings={}, elapsed_ms={}",
        page.sessions.len(),
        page.next_cursor.is_some(),
        page.warnings.len(),
        start.elapsed().as_millis()
    );
    for provider in [AgentProvider::Claude, AgentProvider::Codex] {
        println!(
            "{}: {} sessions in this page",
            provider.display_name(),
            page.sessions
                .iter()
                .filter(|s| s.provider == provider)
                .count()
        );
        if std::env::args().any(|arg| arg == "--history") {
            if let Some(session) = page.sessions.iter().find(|s| s.provider == provider) {
                let start = std::time::Instant::now();
                match read_history(&roots, session, &RemoteHistoryQuery::default(), false) {
                    Ok(history) => println!(
                        "{} passive history: total={}, page={}, warnings={}, elapsed_ms={}",
                        provider.display_name(),
                        history.total_entries,
                        history.entries.len(),
                        history.warnings.len(),
                        start.elapsed().as_millis()
                    ),
                    Err(error) => println!(
                        "{} passive history failed: {error}",
                        provider.display_name()
                    ),
                }
            }
        }
    }
    Ok(())
}
