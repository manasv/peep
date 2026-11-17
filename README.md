<p align="center">
  <img src="assets/icons/PeepIcon-128.png" width="96" height="96" alt="Peep">
</p>

# Peep

Offline dictation. Press a shortcut, speak, and press it again to put your words
into the app you're using.

Peep lives in the menu bar and transcribes on your Mac using Parakeet and
Core ML. Audio and transcripts stay in memory; there's no recording archive
or transcript history. After the initial model download, dictation works offline.

## Getting started

Peep currently supports **macOS 26 or later** on **Apple silicon**. Windows and
Linux aren't supported yet.

To build from source, you'll need **Rust 1.96+**, **Python 3**, and
**Xcode with the Metal toolchain**.
Make sure the full Xcode installation is selected as your active developer directory.

```sh
git clone https://github.com/manasv/peep.git
cd peep
PEEP_CONFIGURATION=release ./script/build_and_run.sh --build
```

Copy `.build/gpui/release/Peep.app` to `/Applications` and open it.
Local builds are ad hoc signed, not notarized.

On first launch, download the speech model (about 632 MB) and enable Microphone
and Text insertion access. If macOS asks you to open System Settings, allow Peep
under **Privacy & Security → Microphone** and **Accessibility**.

## Usage

1. Click a text field in another app.
2. Press **Control–Shift–Space** and start speaking.
3. Press it again to finish. Peep transcribes and inserts the text.

You can change the shortcut in Peep's settings. The menu bar has controls to
finish or cancel a recording, and recordings stop automatically after two minutes.
Closing the window keeps Peep running.

Keep the original field and cursor in place while dictating. If insertion fails,
choose **Copy text** or **Copy Transcript** and paste it yourself. Password fields
and terminals aren't supported.

## Development

Peep is built with Rust, [GPUI](https://github.com/zed-industries/zed), and
[Scriptrs](https://github.com/avencera/scriptrs). The workspace has three crates:
`peep` for the UI, `peep-core` for the dictation state, and `peep-macos` for
capture, inference, and macOS integration.

```sh
./script/build_and_run.sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The build script also accepts `--logs` and `--debug`. For packaging options,
run `./script/package_release.sh --help`.

The real-audio test needs `PEEP_TEST_MODEL_PATH` (a directory containing
`parakeet-v3/`) and `PEEP_TEST_AUDIO_PATH` (a mono, 16-bit PCM WAV file).
Without both, it returns without running inference.

Use the copy in `/Applications` when testing permissions. Rebuilding an ad hoc
signed app may require granting access again.

Bug reports and pull requests are welcome. For insertion issues, include the
app you were dictating into along with your macOS version and steps to reproduce.

## License

[MIT](LICENSE). Third-party notices, including attribution for NVIDIA's
Parakeet model and its Core ML conversion, are in [assets/notices](assets/notices).
