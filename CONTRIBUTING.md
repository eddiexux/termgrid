# Contributing to termgrid

Thanks for your interest in contributing!

## Getting Started

```bash
git clone https://github.com/eddiexux/termgrid.git
cd termgrid
cargo build
cargo test
```

## Development

- **Test**: `cargo test`
- **Lint**: `cargo clippy --all-targets`
- **Format check**: `cargo fmt -- --check`
- **Run**: `cargo run -- .`
- **Debug logs**: `RUST_LOG=debug cargo run -- .` then `tail -f ~/.local/share/termgrid/termgrid.log`

## Pull Request Process

1. Fork the repository
2. Create a feature branch (`git checkout -b feat/my-feature`)
3. Write tests for new functionality
4. Ensure `cargo test` and `cargo clippy` pass
5. Commit with clear messages (e.g., `feat: add X`, `fix: resolve Y`)
6. Open a pull request

## Code Style

- Follow existing patterns in the codebase
- Run `cargo fmt` before committing
- Keep functions focused and small
- Add `///` doc comments to public APIs
- Tests go in `#[cfg(test)] mod tests` at the bottom of each file

## Architecture

See [README.md](README.md#architecture) for an overview. Key modules:

- `src/screen.rs` — VTE terminal emulator wrapper (vt100 crate)
- `src/app.rs` — main application loop and state machine
- `src/tile.rs` / `src/tile_manager.rs` — tile lifecycle
- `src/ui/` — all ratatui rendering widgets

## Reporting Issues

- Include your OS version and terminal emulator
- Attach relevant log output from `~/.local/share/termgrid/termgrid.log`
- For rendering issues, include a screenshot if possible

## License

By contributing, you agree that your contributions will be licensed under the MIT OR Apache-2.0 license.
