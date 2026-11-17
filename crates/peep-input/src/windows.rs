use crate::Snapshot;
use anyhow::{Result, bail};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::{
    Win32::{
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            Variant::VariantToBoolean,
        },
        UI::{
            Accessibility::*,
            Controls::{EM_GETPASSWORDCHAR, EM_GETSEL},
            Input::KeyboardAndMouse::*,
            WindowsAndMessaging::*,
        },
    },
    core::Interface,
};

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

pub struct Target {
    element: IUIAutomationElement,
    snapshot: Snapshot,
    pid: u32,
    foreground: windows::Win32::Foundation::HWND,
}
impl Target {
    pub fn belongs_to_process(&self, pid: u32) -> bool {
        self.pid == pid
    }
}

pub struct Context {
    automation: IUIAutomation,
    _apartment: Apartment,
}
impl Context {
    pub fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            let apartment = Apartment;
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            if let Ok(options) = automation.cast::<IUIAutomation2>() {
                options.SetConnectionTimeout(500)?;
                options.SetTransactionTimeout(500)?;
            }
            Ok(Self {
                automation,
                _apartment: apartment,
            })
        }
    }
    pub fn capture(&mut self) -> Result<Target> {
        unsafe {
            let foreground = GetForegroundWindow();
            let element = self.automation.GetFocusedElement()?;
            let pid = element.CurrentProcessId()? as u32;
            let mut window_pid = 0;
            GetWindowThreadProcessId(foreground, Some(&mut window_pid));
            if foreground.is_invalid()
                || pid == std::process::id()
                || window_pid == std::process::id()
            {
                bail!("Choose a text field in another app.");
            }
            let mut class = [0u16; 256];
            let length = GetClassNameW(foreground, &mut class) as usize;
            let class = String::from_utf16_lossy(&class[..length]);
            if class == "ConsoleWindowClass" || class.contains("CASCADIA") {
                bail!("Terminals are not used for automatic dictation.");
            }
            let snapshot = snapshot(&element)?;
            Ok(Target {
                element,
                snapshot,
                foreground,
                pid,
            })
        }
    }
    pub fn insert(&mut self, target: Target, text: &str, cancel: &CancellationToken) -> Result<()> {
        unsafe {
            let expected = target.snapshot.replace(text)?;
            if text.trim().is_empty() {
                bail!("No text to insert.");
            }
            let deadline = Instant::now() + Duration::from_millis(500);
            while modifiers_pressed() {
                if cancel.is_cancelled() || Instant::now() >= deadline {
                    bail!("Release the shortcut keys before inserting text.");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let focused = self.automation.GetFocusedElement()?;
            if GetForegroundWindow() != target.foreground
                || !self
                    .automation
                    .CompareElements(&focused, &target.element)?
                    .as_bool()
                || snapshot(&target.element)? != target.snapshot
            {
                bail!("The original field, cursor, or text changed.");
            }
            let mut inputs = Vec::new();
            for unit in text.replace("\r\n", "\n").encode_utf16() {
                let newline = unit == 10 || unit == 13;
                for up in [false, true] {
                    inputs.push(INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT {
                                wVk: if newline { VK_RETURN } else { VIRTUAL_KEY(0) },
                                wScan: if newline { 0 } else { unit },
                                dwFlags: (if newline {
                                    KEYBD_EVENT_FLAGS(0)
                                } else {
                                    KEYEVENTF_UNICODE
                                }) | if up {
                                    KEYEVENTF_KEYUP
                                } else {
                                    KEYBD_EVENT_FLAGS(0)
                                },
                                ..Default::default()
                            },
                        },
                    });
                }
            }
            if cancel.is_cancelled() {
                bail!("Dictation canceled.");
            }
            if SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) as usize != inputs.len() {
                bail!(
                    "Windows blocked text insertion. Check the field before pasting manually; elevated apps require matching privileges."
                );
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(25));
                if read_text(&target.element)
                    .is_ok_and(|value| comparable(&value) == comparable(&expected))
                {
                    return Ok(());
                }
            }
            bail!(
                "The editor did not confirm the inserted text. Check the field before pasting manually."
            );
        }
    }
    pub fn enable_accessibility(&mut self) -> Result<()> {
        Ok(())
    }
    pub fn accessibility_status(&mut self) -> Result<bool> {
        Ok(true)
    }
}

unsafe fn modifiers_pressed() -> bool {
    [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
        .iter()
        .any(|key| unsafe { GetAsyncKeyState(key.0 as i32) < 0 })
}
fn read_text(element: &IUIAutomationElement) -> Result<String> {
    unsafe {
        if is_native_edit(element)? {
            return Ok(edit_snapshot(element)?.text);
        }
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
        {
            return Ok(pattern.DocumentRange()?.GetText(1_000_001)?.to_string());
        }
        Ok(edit_snapshot(element)?.text)
    }
}
fn snapshot(element: &IUIAutomationElement) -> Result<Snapshot> {
    unsafe {
        if element.CurrentIsPassword()?.as_bool()
            || !element.CurrentHasKeyboardFocus()?.as_bool()
            || !element.CurrentIsEnabled()?.as_bool()
        {
            bail!("Choose an enabled, non-password text field.");
        }
        if is_native_edit(element)? {
            return edit_snapshot(element);
        }
        let pattern: IUIAutomationTextPattern = match element.GetCurrentPatternAs(UIA_TextPatternId)
        {
            Ok(pattern) => pattern,
            Err(_) => return edit_snapshot(element),
        };
        let document = pattern.DocumentRange()?;
        let read_only = document.GetAttributeValue(UIA_IsReadOnlyAttributeId)?;
        if VariantToBoolean(&read_only)?.as_bool() {
            bail!("The field is read-only.");
        }
        let text = document.GetText(1_000_001)?.to_string();
        if text.len() > 1_000_000 {
            bail!("The editor is too large to verify safely.");
        }
        let selections = pattern.GetSelection()?;
        if selections.Length()? != 1 {
            bail!("Choose a single insertion point or selection.");
        }
        let selected = selections.GetElement(0)?;
        let prefix = document.Clone()?;
        prefix.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            &selected,
            TextPatternRangeEndpoint_Start,
        )?;
        let start = prefix.GetText(-1)?.to_string().chars().count();
        let end = start + selected.GetText(-1)?.to_string().chars().count();
        let snapshot = Snapshot { text, start, end };
        snapshot.replace("")?;
        Ok(snapshot)
    }
}

fn comparable(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn message(window: HWND, code: u32, wparam: WPARAM, lparam: LPARAM) -> Result<usize> {
    let mut result = 0;
    if unsafe {
        SendMessageTimeoutW(
            window,
            code,
            wparam,
            lparam,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            500,
            Some(&mut result),
        )
    }
    .0 == 0
    {
        bail!("The editor did not respond.");
    }
    Ok(result)
}

fn is_native_edit(element: &IUIAutomationElement) -> Result<bool> {
    let class = unsafe { element.CurrentClassName()? }
        .to_string()
        .to_lowercase();
    Ok(class == "edit" || class.starts_with("richedit"))
}

fn edit_snapshot(element: &IUIAutomationElement) -> Result<Snapshot> {
    unsafe {
        let class = element.CurrentClassName()?.to_string().to_lowercase();
        if class != "edit" && !class.starts_with("richedit") {
            bail!("The editor does not expose a verifiable selection.");
        }
        let window = element.CurrentNativeWindowHandle()?;
        if window.is_invalid() {
            bail!("The editor has no native window.");
        }
        let style = GetWindowLongPtrW(window, GWL_STYLE);
        if style & (ES_PASSWORD | ES_READONLY) as isize != 0
            || message(window, EM_GETPASSWORDCHAR, WPARAM(0), LPARAM(0))? != 0
        {
            bail!("Password fields and read-only fields are not used for dictation.");
        }
        let length = message(window, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0))?;
        if length > 1_000_000 {
            bail!("The editor is too large to verify safely.");
        }
        let mut buffer = vec![0u16; length + 1];
        let count = message(
            window,
            WM_GETTEXT,
            WPARAM(buffer.len()),
            LPARAM(buffer.as_mut_ptr() as isize),
        )?;
        if count > length {
            bail!("The editor changed while reading its contents.");
        }
        buffer.truncate(count);
        let (mut start, mut end) = (u32::MAX, u32::MAX);
        message(
            window,
            EM_GETSEL,
            WPARAM(&mut start as *mut u32 as usize),
            LPARAM(&mut end as *mut u32 as isize),
        )?;
        if start > end || end as usize > buffer.len() {
            bail!("Invalid editor selection.");
        }
        Ok(Snapshot {
            text: String::from_utf16(&buffer)?,
            start: String::from_utf16(&buffer[..start as usize])?
                .chars()
                .count(),
            end: String::from_utf16(&buffer[..end as usize])?.chars().count(),
        })
    }
}
