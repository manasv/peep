#[cfg(any(not(target_os = "macos"), feature = "desktop-preview"))]
pub use peep_desktop::*;
#[cfg(all(target_os = "macos", not(feature = "desktop-preview")))]
pub use peep_macos::*;

pub const MACOS: bool = cfg!(all(target_os = "macos", not(feature = "desktop-preview")));
pub const QUIT_KEY: &str = if MACOS { "cmd-q" } else { "ctrl-q" };
pub const SETTINGS_KEY: &str = if MACOS { "cmd-," } else { "ctrl-," };
pub const FONT: &str = if cfg!(target_os = "macos") {
    ".SystemUIFont"
} else if cfg!(target_os = "windows") {
    "Segoe UI"
} else {
    "sans-serif"
};
