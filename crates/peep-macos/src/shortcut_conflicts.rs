//! CopySymbolicHotKeys reports system-wide bindings, not application menu shortcuts.
use anyhow::{Result, bail};
use core_foundation::{
    array::{CFArray, CFArrayRef},
    base::{CFType, TCFType},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn CopySymbolicHotKeys(keys: *mut CFArrayRef) -> i32;
}

pub fn system_conflict(key: HotKey) -> Result<bool> {
    let Some(code) = virtual_code(key.key) else {
        return Ok(false);
    };
    let mut keys = std::ptr::null();
    // CopySymbolicHotKeys is not thread-safe and returns an owned array.
    let status = unsafe { CopySymbolicHotKeys(&mut keys) };
    if status != 0 || keys.is_null() {
        bail!("Couldn’t check system shortcuts");
    }
    let keys: CFArray<CFType> = unsafe { TCFType::wrap_under_create_rule(keys) };
    for value in keys.iter() {
        let Some(raw) = value.downcast::<CFDictionary>() else {
            continue;
        };
        let info: CFDictionary<CFString, CFType> =
            unsafe { TCFType::wrap_under_get_rule(raw.as_concrete_TypeRef()) };
        if matches_binding(&info, code, carbon_modifiers(key.mods)) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn peep_conflict(key: HotKey) -> bool {
    key.mods == Modifiers::META && matches!(key.key, Code::KeyQ | Code::Comma)
}

fn matches_binding(info: &CFDictionary<CFString, CFType>, code: i64, modifiers: i64) -> bool {
    let number = |name| {
        info.find(CFString::new(name))
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|v| v.to_i64())
    };
    let enabled = info
        .find(CFString::new("kHISymbolicHotKeyEnabled"))
        .and_then(|v| v.downcast::<CFBoolean>())
        .is_some_and(bool::from);
    enabled
        && number("kHISymbolicHotKeyCode") == Some(code)
        && number("kHISymbolicHotKeyModifiers") == Some(modifiers)
}

fn carbon_modifiers(mods: Modifiers) -> i64 {
    [
        (Modifiers::META, 256),
        (Modifiers::SHIFT, 512),
        (Modifiers::ALT, 2048),
        (Modifiers::CONTROL, 4096),
    ]
    .into_iter()
    .filter(|(flag, _)| mods.contains(*flag))
    .map(|(_, value)| value)
    .sum()
}

// Carbon virtual-key codes from Events.h, matching global-hotkey's mapping.
fn virtual_code(code: Code) -> Option<i64> {
    Some(match code {
        Code::KeyA => 0x00,
        Code::KeyS => 0x01,
        Code::KeyD => 0x02,
        Code::KeyF => 0x03,
        Code::KeyH => 0x04,
        Code::KeyG => 0x05,
        Code::KeyZ => 0x06,
        Code::KeyX => 0x07,
        Code::KeyC => 0x08,
        Code::KeyV => 0x09,
        Code::KeyB => 0x0b,
        Code::KeyQ => 0x0c,
        Code::KeyW => 0x0d,
        Code::KeyE => 0x0e,
        Code::KeyR => 0x0f,
        Code::KeyY => 0x10,
        Code::KeyT => 0x11,
        Code::Digit1 => 0x12,
        Code::Digit2 => 0x13,
        Code::Digit3 => 0x14,
        Code::Digit4 => 0x15,
        Code::Digit6 => 0x16,
        Code::Digit5 => 0x17,
        Code::Digit9 => 0x19,
        Code::Digit7 => 0x1a,
        Code::Digit8 => 0x1c,
        Code::Digit0 => 0x1d,
        Code::KeyO => 0x1f,
        Code::KeyU => 0x20,
        Code::KeyI => 0x22,
        Code::KeyP => 0x23,
        Code::Enter => 0x24,
        Code::KeyL => 0x25,
        Code::KeyJ => 0x26,
        Code::KeyK => 0x28,
        Code::KeyN => 0x2d,
        Code::KeyM => 0x2e,
        Code::Tab => 0x30,
        Code::Space => 0x31,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enabled_system_bindings_match_exact_key_and_modifiers() {
        for enabled in [false, true] {
            let info = CFDictionary::from_CFType_pairs(&[
                (
                    CFString::new("kHISymbolicHotKeyEnabled"),
                    CFBoolean::from(enabled).as_CFType(),
                ),
                (
                    CFString::new("kHISymbolicHotKeyCode"),
                    CFNumber::from(49).as_CFType(),
                ),
                (
                    CFString::new("kHISymbolicHotKeyModifiers"),
                    CFNumber::from(256).as_CFType(),
                ),
            ]);
            assert_eq!(matches_binding(&info, 49, 256), enabled);
            assert!(!matches_binding(&info, 49, 256 + 512));
            assert!(!matches_binding(&info, 48, 256));
        }
        assert_eq!(virtual_code(Code::Space), Some(49));
        assert_eq!(carbon_modifiers(Modifiers::META | Modifiers::SHIFT), 768);
    }

    #[test]
    fn peep_menu_shortcuts_are_reserved_but_extra_modifiers_are_distinct() {
        assert!(peep_conflict(HotKey::new(
            Some(Modifiers::META),
            Code::KeyQ
        )));
        assert!(peep_conflict(HotKey::new(
            Some(Modifiers::META),
            Code::Comma
        )));
        assert!(!peep_conflict(HotKey::new(
            Some(Modifiers::META | Modifiers::SHIFT),
            Code::KeyQ
        )));
    }
}
