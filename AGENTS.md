# Peep

Offline dictation for Apple silicon Macs. Rust, GPUI, and Core ML. Audio and
transcripts stay in memory. Requires macOS 26+, Rust 1.96+, and Xcode with Metal.

- `crates/peep`: window and application lifecycle.
- `crates/peep-core`: dictation state and commands.
- `crates/peep-macos`: capture, inference, permissions, and text insertion.
- `crates/peep-macos/assets/parakeet-v3.json`: pinned model manifest.

## Commands

```sh
./script/build_and_run.sh
./script/build_and_run.sh --build
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
./script/package_release.sh --help
```

`PEEP_CONFIGURATION=release` selects release builds. Real-audio tests need
`PEEP_TEST_MODEL_PATH` and `PEEP_TEST_AUDIO_PATH`. Test macOS permissions with
`/Applications/Peep.app`; ad hoc rebuilds can require granting access again.

## Working rules

- Keep project guidance here. Create other Markdown files only when requested.
- Read the source and tests. Avoid historical narratives and rules that freeze implementation choices.
- Prefer clear code over comments. Keep comments only for legal notices, public API contracts, or verified external constraints.
- Use `main` as the primary branch. Make small, coherent Conventional Commits: `type(scope): summary`, with scope optional.
- Verify behavior relevant to the change. Report results in the conversation.
