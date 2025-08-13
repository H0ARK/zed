# Repository Guidelines

## Project Structure & Module Organization
- Rust workspace: `Cargo.toml` with many crates under `crates/` (e.g., `crates/gpui`, `crates/editor`, `crates/project`, `crates/zed`). The `crates/zed` crate is the app entry point.
- Extensions live in `extensions/` (e.g., `extensions/snippets`, `extensions/html`).
- Dev tooling in `script/` (lint, build helpers) and `tooling/` (e.g., `xtask`).
- Docs in `docs/`, assets in `assets/`, CI in `.github/`.

## Build, Test, and Development Commands
- Run debug build: `cargo run` (runs the `zed` binary by default).
- Run release build: `cargo run --release`.
- Test all crates: `cargo test --workspace` (use `cargo nextest run --workspace` for faster local runs).
- Lint (deny warnings): `script/clippy` (adds `--workspace --release --all-targets --all-features`).
- Storybook for UI components: `script/storybook` or `script/storybook Button`.
- Collaboration deps (optional): `docker compose up -d` to start DB/LiveKit for `crates/collab` integration.

## Coding Style & Naming Conventions
- Rust: follow `rustfmt` defaults (4-space indent, 100–120 cols). Prefer `snake_case` for crates/modules/functions, `CamelCase` for types, `SCREAMING_SNAKE_CASE` for consts.
- Lints: keep code clippy-clean; `script/clippy` treats warnings as errors.
- Markdown/JSON docs: Prettier width 120 (`.prettierrc`).
- Paths: new crates in `crates/<snake_case>/`; binaries via `[[bin]]` or `default-members`.

## Testing Guidelines
- Unit/integration tests live with each crate (e.g., `crates/<name>/src/...` and `crates/<name>/tests/`).
- Name tests descriptively: `does_<thing>_when_<condition>()`.
- Prefer deterministic tests; use `-- --ignored` for slow/flake-prone cases.
- Shell integration helpers: `./test_interactive.sh`, `./test_shell_integration.sh` (macOS/Linux).

## Commit & Pull Request Guidelines
- Commit messages: imperative mood with optional scope, mirroring history, e.g., `editor: Fix cursor jump on paste`, `ci: Skip build_docs`. Reference PRs/issues when relevant (e.g., `(#12345)`).
- PRs: include a problem statement, solution summary, screenshots for UI, reproduction steps, and linked issues. Add tests and docs when changing behavior.
- Follow `CONTRIBUTING.md` and the Code of Conduct. Sign the CLA when prompted.

## Security & Configuration Tips
- Do not commit secrets. Use local env only for development (see `livekit.yaml`, Docker Compose). 
- Verify license compliance locally: `script/check-licenses` or `cargo about generate` as configured.
