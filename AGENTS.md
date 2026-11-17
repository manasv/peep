# Peep

Offline dictation in Rust and GPUI. Audio and transcripts stay in memory.
macOS uses Core ML and requires macOS 26+, Apple silicon, and Xcode with Metal.
Windows and Linux use Parakeet ONNX on the CPU, native accessible text delivery,
and native/portal shortcuts. Wayland compositors can also bind `peep --toggle`.
Rust 1.96+ is required on all platforms.

- `crates/peep`: window and application lifecycle.
- `crates/peep-core`: dictation state and commands.
- `crates/peep-runtime`: shared audio, model downloads, settings, and instance locking.
- `crates/peep-macos`: Core ML inference, permissions, and native text insertion.
- `crates/peep-desktop`: Windows/Linux services and ONNX inference.
- `crates/peep-input`: UI Automation, AT-SPI, Wayland portals, and desktop trays.
- `crates/peep-runtime/assets`: pinned Core ML and ONNX model manifests.

## Commands

```sh
./script/build_and_run.sh
./script/build_and_run.sh --build
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
./script/package_release.sh --help
python3 script/package_desktop.py --help
python3 script/test_inference.py --backend onnx
dbus-run-session -- cargo test -p peep-input --features linux-integration
./script/test_linux_desktop.sh x11
./script/test_linux_desktop.sh wayland
```

`PEEP_CONFIGURATION=release` selects release builds. Real-audio tests need
`PEEP_TEST_MODEL_PATH` (Core ML) or `PEEP_TEST_ONNX_MODEL_PATH` (ONNX), plus
`PEEP_TEST_AUDIO_PATH`. `PEEP_DATA_DIR` isolates test app data.
`cargo run -p peep --features desktop-preview -- --models PATH` exercises the
Windows/Linux UI and ONNX backend on a Mac. Test macOS permissions with
`/Applications/Peep.app`; ad hoc rebuilds can require granting access again.

## Working rules

- Keep project guidance here. Create other Markdown files only when requested.
- Read the source and tests. Avoid historical narratives and rules that freeze implementation choices.
- Prefer clear code over comments. Keep comments only for legal notices, public API contracts, or verified external constraints.
- Use `main` as the primary branch. Make small, coherent Conventional Commits: `type(scope): summary`, with scope optional.
- Verify behavior relevant to the change. Report results in the conversation.
