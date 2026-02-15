# Contributing to ShellDeck

Thanks for your interest in contributing to ShellDeck! Here's how to get started.

## Getting Started

1. Fork the repository
2. Clone your fork: `git clone https://github.com/YOUR_USERNAME/shelldeck.git`
3. Install system dependencies (see [README.md](README.md#requirements))
4. Make sure `cargo check` passes before starting work

## Development Workflow

1. Create a branch from `main` for your changes
2. Make your changes, keeping commits focused and well-described
3. Run checks before submitting:

```bash
cargo check
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

4. Open a pull request against `main`

## Code Style

- Follow standard Rust conventions and `rustfmt` formatting
- Use `parking_lot::Mutex` instead of `std::sync::Mutex` for grid/terminal state
- Prefer `anyhow::Result` for error propagation in application code
- Prefer `thiserror` for library-level error types
- Avoid `unwrap()` in production code -- use `?`, `.ok()`, or `.unwrap_or()` instead
- Use conditional `if/else` for element building instead of `.when()` chains (GPUI pattern)

## Architecture Notes

- **GPUI** is the UI framework -- it uses a retained-mode entity system with `Render` trait
- Terminal grid operations are on the rendering hot path -- keep them fast
- Credentials are stored via the OS keychain (`keyring` crate) -- never hardcode or log secrets
- ShellDeck reads `~/.ssh/config` but never writes to it

## Reporting Issues

- Check existing issues before opening a new one
- Include your OS, Rust version (`rustc --version`), and steps to reproduce
- For rendering issues, include your GPU driver version if possible

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
