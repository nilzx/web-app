# Project conventions

- Documentation is written in English first. Chinese translations live next to it as `docs/<name>.zh-CN.md` and are linked from the English document; keep both in sync when changing either. Screenshots follow the same pattern: `docs/<name>.png` shows the English UI, `docs/<name>.zh-CN.png` the Chinese UI.
- Keep user docs short and focused on how to use WebDock; implementation details belong in code comments.
- The launcher UI defaults to English (`launcher.language = "en"`); Chinese strings live in `ui/i18n.js`.
- Rust code lives in `src-tauri/`. Before pushing run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` there.
