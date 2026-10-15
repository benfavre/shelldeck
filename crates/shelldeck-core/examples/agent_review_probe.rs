//! Explicit read-only checkout snapshot probe. No inference or remote effects.
use shelldeck_core::agent_review::AgentReviewBackend;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: agent_review_probe <checkout>")?;
    let snapshot = AgentReviewBackend::default().observe(std::path::Path::new(&path))?;
    println!(
        "snapshot: files={}, status={}, diffs={}, complete={}, refusals={}",
        snapshot.files.len(),
        snapshot.status.len(),
        snapshot.diffs.len(),
        snapshot.complete,
        snapshot.refusals.len()
    );
    for refusal in snapshot.refusals {
        println!("refusal: {refusal}");
    }
    Ok(())
}
