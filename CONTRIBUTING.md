# Contributing to fastbrowser

Thanks for your interest! This is a Rust workspace-free single crate (plus a
`fastbrowser_c/` C ABI and `bindings/`).

## Development

```bash
# Format / lint / test (mock engine, no browser required)
cargo fmt --all -- --check
cargo clippy --features engine-cdp --all-targets
cargo test --features engine-cdp

# Real-Chromium integration tests (auto-SKIP when no browser is found)
./scripts/fetch-chromium.sh                 # downloads Chrome for Testing → vendor/chromium
cargo test --features engine-cdp --test chromium_integration
```

Build artifacts (`target/`, `dist/`, `vendor/`) are git-ignored. Override their
location with `CARGO_TARGET_DIR` / `VENDOR_DIR` if needed.

## Language

The kernel is **English-only**: everything a host, the CLI, an LLM, or the C ABI
can see (error messages, JSON results, tool descriptions, log lines, CLI help)
must be English. Do not add runtime localization to the kernel — hosts own the UI
language. `scripts/check-english.py` (run in CI) rejects CJK text in code lines;
keep code and string literals ASCII.

## Pull requests

- Keep changes focused; add tests for new behavior.
- Run `cargo fmt` and `cargo clippy` before opening a PR.
- The public API (`src/sdk/`, `src/engine/`) and the tool manifest
  (`tool_list()`) are considered stable — call out any breaking change.

## License

By contributing you agree your contributions are licensed under Apache-2.0.
