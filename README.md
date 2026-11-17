<p align="center">
  <img src="assets/icons/PeepIcon-128.png" width="96" height="96" alt="Peep">
</p>

# Peep

Offline dictation. Press a shortcut, speak, and press it again to put your words
into the app you're using.

Peep transcribes locally using Parakeet. Audio and transcripts stay in memory;
there's no recording archive or transcript history. After the initial model
download, dictation works offline.

Use a global shortcut to dictate into the focused app. Peep runs in the
background with menu bar or tray controls. macOS uses Core ML; Windows and Linux
use ONNX on the CPU.

## Getting started

You'll need **Rust 1.96+** and **Python 3.11+** to build from source.

### macOS

Requires **macOS 26+**, **Apple silicon**, and **Xcode with the Metal toolchain**.
Select the full Xcode installation as your active developer directory.

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

### Windows and Linux

Windows x64 and Linux x64 have native backends. Linux supports X11 and Wayland.
These ports still need testing across more apps and desktop environments.
Windows builds need the MSVC C++ tools, Windows SDK, CMake, and LLVM/libclang.
Use a Visual Studio developer terminal. If needed, set `LIBCLANG_PATH` to LLVM's
`bin` directory and `GPUI_FXC_PATH` to the SDK's `fxc.exe`.
On Ubuntu, install the native libraries first:

```sh
sudo apt-get install clang cmake pkg-config libasound2-dev libfontconfig1-dev libfreetype6-dev libudev-dev libx11-xcb-dev libxcb-xkb-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libegl1-mesa-dev libvulkan-dev at-spi2-core xdg-desktop-portal
```

From a checkout of this repository:

```sh
cargo run --release --locked -p peep
```

Download the model when prompted (about 671 MB). Focus a text field in another
app, press **Control–Shift–Space**, speak, then press it again to insert the text.
**Record to clipboard** in the Peep window lets you dictate without a destination.
Closing the window minimizes Peep; quit through the tray, the window's Quit
button, or Ctrl–Q.

On Linux, enable **Text insertion** in Peep to activate accessibility support.
The destination app must expose editable text through AT-SPI. Some browsers and
Electron apps need accessibility enabled in their own settings or launch options.
Replacing a selection is supported in plain entry fields; clear selections in
rich or multiline editors before dictating.

Wayland shortcuts use your desktop's Global Shortcuts portal. The desktop asks
for permission and manages the actual key combination. Click the shortcut button
in Peep to configure it. This needs `xdg-desktop-portal` and a desktop backend
that implements Global Shortcuts.

For compositors without that portal, bind `peep --toggle` in the compositor's
shortcut settings while Peep is running. `--stop`, `--cancel`, and `--open` are
also available. These commands use the local session bus.

The Linux archive includes a desktop entry and icon. Put `peep` on your PATH and
install those files as `~/.local/share/applications/dev.peep.app.desktop` and
`~/.local/share/icons/hicolor/128x128/apps/dev.peep.app.png` for app identification.
Tray visibility depends on the desktop's StatusNotifierItem support.

## Usage on macOS

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
[Scriptrs](https://github.com/avencera/scriptrs) on macOS, with
[Sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) on Windows and Linux.
`peep-core` holds the dictation state; `peep-runtime` shares audio, downloads,
and settings. `peep-macos` and `peep-desktop` provide the platform backends. `peep-input`
implements Windows UI Automation, Linux AT-SPI, portals, and desktop controls.

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

On macOS, `./script/build_and_run.sh` builds and launches a development bundle;
`--logs` runs it in the foreground and `--debug` launches LLDB. Test permissions
with the copy in `/Applications`; ad hoc rebuilds may require granting access again.

Run `python3 script/test_inference.py --backend coreml` on a Mac or
`python3 script/test_inference.py --backend onnx` for the portable backend.
These download verified model files and a public speech fixture. Ordinary
workspace tests skip inference unless fixture environment variables are set.

Desktop integration tests use a private D-Bus session and disposable editor
fixtures. `script/test_linux_desktop.sh x11` uses Xvfb/Openbox; `wayland` uses a
headless Sway compositor. Windows UI tests run with `PEEP_TEST_WINDOWS_UI=1`.
The CI workflow installs the dependencies and runs these tests.

For release archives, use `./script/package_release.sh` on macOS or
`python3 script/package_desktop.py` on Windows/Linux. CI builds all three platforms.
`PEEP_DATA_DIR` selects a separate data directory for testing.

Bug reports and pull requests are welcome. For insertion issues, include the
app you were dictating into along with your operating system and steps to reproduce.

## License

[MIT](LICENSE). Third-party notices, including attribution for NVIDIA's
Parakeet model and its Core ML conversion, are in [assets/notices](assets/notices).
