use accessibility::{AXAttribute, AXUIElement};
use accessibility_sys::*;
use anyhow::{Context, Result, anyhow, bail};
use core_foundation::{
    base::{CFRange, CFType, TCFType},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    string::CFString,
};
use core_graphics::{
    event::{CGEvent, CGEventFlags},
    event_source::{CGEventSource, CGEventSourceStateID},
};
use objc2_app_kit::NSWorkspace;
use std::time::{Duration, Instant};

fn attr(element: &AXUIElement, name: &str) -> Result<CFType> {
    Ok(element.attribute(&AXAttribute::new(&CFString::new(name)))?)
}
fn optional_attr(element: &AXUIElement, name: &str) -> Result<Option<CFType>> {
    match element.attribute(&AXAttribute::new(&CFString::new(name))) {
        Ok(value) => Ok(Some(value)),
        Err(accessibility::Error::Ax(code))
            if [
                kAXErrorNoValue,
                kAXErrorAttributeUnsupported,
                kAXErrorNotImplemented,
            ]
            .contains(&code) =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}
fn focused(system: &AXUIElement) -> Result<AXUIElement> {
    attr(system, kAXFocusedUIElementAttribute)?
        .downcast::<AXUIElement>()
        .ok_or_else(|| anyhow!("No focused editor."))
}
fn prepare_application(application: &AXUIElement) -> Result<bool> {
    match application.set_attribute(
        &AXAttribute::new(&CFString::new("AXManualAccessibility")),
        CFBoolean::true_value().as_CFType(),
    ) {
        Ok(()) => Ok(true),
        Err(accessibility::Error::Ax(code))
            if [kAXErrorAttributeUnsupported, kAXErrorNotImplemented].contains(&code) =>
        {
            Ok(false)
        }
        Err(error) => Err(error).context("Could not enable the editor's accessibility support."),
    }
}
fn focused_editor(
    system: &AXUIElement,
    application: &AXUIElement,
    expected_pid: i32,
    initializing: bool,
) -> Result<AXUIElement> {
    let deadline = Instant::now() + Duration::from_millis(750);
    loop {
        if frontmost().map(|app| app.0) != Some(expected_pid) {
            bail!("The original app is no longer focused.");
        }
        let result = focused(system).and_then(|element| {
            if focused(application)? != element {
                bail!("The destination is ambiguous.");
            }
            Ok(element)
        });
        if initializing
            && Instant::now() < deadline
            && result.as_ref().is_err_and(|error| {
                matches!(
                    error.downcast_ref::<accessibility::Error>(),
                    Some(accessibility::Error::Ax(code))
                        if [kAXErrorNoValue, kAXErrorCannotComplete].contains(code)
                )
            })
        {
            std::thread::sleep(Duration::from_millis(30));
            continue;
        }
        return result.context("The editor does not expose its focused text field.");
    }
}
fn string(value: CFType) -> Result<String> {
    value
        .downcast::<CFString>()
        .map(|x| x.to_string())
        .ok_or_else(|| anyhow!("Unsupported editor attribute."))
}
fn selection(element: &AXUIElement) -> Result<(isize, isize)> {
    let value = attr(element, kAXSelectedTextRangeAttribute)?;
    let mut range = CFRange {
        location: 0,
        length: 0,
    };
    unsafe {
        if value.type_of() != AXValueGetTypeID()
            || AXValueGetType(value.as_CFTypeRef() as AXValueRef) != kAXValueTypeCFRange
            || !AXValueGetValue(
                value.as_CFTypeRef() as AXValueRef,
                kAXValueTypeCFRange,
                &mut range as *mut _ as *mut _,
            )
        {
            bail!("The editor does not expose a verifiable selection.");
        }
    }
    if range.location < 0 || range.length < 0 {
        bail!("Invalid editor selection.");
    }
    Ok((range.location, range.length))
}
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}

fn editable(element: &AXUIElement) -> Result<()> {
    if unsafe { IsSecureEventInputEnabled() } {
        bail!("Secure keyboard input is active. Use Copy Transcript.");
    }
    let role = string(attr(element, kAXRoleAttribute)?)?;
    if ![kAXTextFieldRole, kAXTextAreaRole, kAXComboBoxRole].contains(&role.as_str()) {
        bail!("Choose an editable text field.");
    }
    if let Some(subrole) = optional_attr(element, kAXSubroleAttribute)?
        && string(subrole)? == kAXSecureTextFieldSubrole
    {
        bail!("Password fields are never used for dictation.");
    }
    let selected_text =
        element.is_settable(&AXAttribute::new(&CFString::new(kAXSelectedTextAttribute)))?;
    let value = element.is_settable(&AXAttribute::new(&CFString::new(kAXValueAttribute)))?;
    if !selected_text && !value {
        bail!("This field is read-only. Use Copy text instead.");
    }
    Ok(())
}
pub fn trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub fn request_trust() -> bool {
    let key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) }
}
pub fn frontmost() -> Option<(i32, String, String)> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    Some((
        app.processIdentifier(),
        app.bundleIdentifier()
            .map(|x| x.to_string())
            .unwrap_or_default(),
        app.localizedName()
            .map(|x| x.to_string())
            .unwrap_or_else(|| "the editor".into()),
    ))
}

/// Owns one verified destination and is consumed on the first delivery attempt.
pub struct Target {
    system: AXUIElement,
    application: AXUIElement,
    element: AXUIElement,
    window: Option<CFType>,
    pid: i32,
    selection: (isize, isize),
    name: String,
    paste: bool,
    original_value: Option<String>,
}
pub enum DeliveryOutcome {
    Verified(String),
    Unconfirmed(String),
}
impl Target {
    pub fn capture(expected_pid: i32) -> Result<Self> {
        if !trusted() {
            bail!("Enable Accessibility to insert text in other apps.");
        }
        let (pid, bundle, name) = frontmost().ok_or_else(|| anyhow!("No destination app."))?;
        if pid != expected_pid {
            bail!("The original app is no longer focused.");
        }
        if pid == std::process::id() as i32 {
            bail!("Choose a text field in another app.");
        }
        if [
            "com.apple.Terminal",
            "com.googlecode.iterm2",
            "dev.warp.Warp-Stable",
        ]
        .contains(&bundle.as_str())
        {
            bail!("Use Copy Transcript for terminals.");
        }
        let system = AXUIElement::system_wide();
        system.set_messaging_timeout(0.25)?;
        let application = AXUIElement::application(pid);
        application.set_messaging_timeout(0.25)?;
        let initializing = prepare_application(&application)?;
        let element = focused_editor(&system, &application, pid, initializing)?;
        element.set_messaging_timeout(0.25)?;
        editable(&element)?;
        let selection = selection(&element)?;
        let original_value =
            optional_attr(&element, kAXValueAttribute)?.and_then(|value| string(value).ok());
        let paste = prefers_paste(&bundle)
            || !element.is_settable(&AXAttribute::new(&CFString::new(kAXSelectedTextAttribute)))?;
        let window = optional_attr(&application, kAXFocusedWindowAttribute)?;
        let target = Self {
            system,
            application,
            element,
            window,
            pid,
            selection,
            name,
            paste,
            original_value,
        };
        target.verify()?;
        Ok(target)
    }
    fn verify(&self) -> Result<()> {
        if !trusted()
            || frontmost().map(|x| x.0) != Some(self.pid)
            || focused(&self.system)? != self.element
            || focused(&self.application)? != self.element
            || optional_attr(&self.application, kAXFocusedWindowAttribute)? != self.window
            || selection(&self.element)? != self.selection
        {
            bail!("The original field or cursor changed.");
        }
        editable(&self.element)?;
        if let Some(original) = &self.original_value
            && string(attr(&self.element, kAXValueAttribute)?)? != *original
        {
            bail!("The original text field changed during dictation.");
        }
        if frontmost().map(|x| x.0) != Some(self.pid) {
            bail!("The original app is no longer focused.");
        }
        Ok(())
    }
    pub fn insert(
        self,
        text: &str,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<DeliveryOutcome> {
        if text.trim().is_empty() {
            bail!("No text to insert.");
        }
        self.verify()?;
        if cancel.is_cancelled() {
            bail!("Dictation canceled.");
        }
        if self.paste {
            return self.paste_text(text, cancel);
        }
        let value = CFString::new(text).as_CFType();
        self.element.set_attribute(
            &AXAttribute::new(&CFString::new(kAXSelectedTextAttribute)),
            value,
        )?;
        Ok(DeliveryOutcome::Verified(format!(
            "Inserted into {}.",
            self.name
        )))
    }
    fn paste_text(
        &self,
        text: &str,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<DeliveryOutcome> {
        let source = CGEventSource::new(CGEventSourceStateID::Private)
            .map_err(|_| anyhow!("Could not create the paste event."))?;
        let down = CGEvent::new_keyboard_event(source.clone(), 9, true)
            .map_err(|_| anyhow!("Could not create the paste event."))?;
        let up = CGEvent::new_keyboard_event(source, 9, false)
            .map_err(|_| anyhow!("Could not create the paste event."))?;
        down.set_flags(CGEventFlags::CGEventFlagCommand);
        up.set_flags(CGEventFlags::CGEventFlagCommand);
        self.verify()?;
        if cancel.is_cancelled() {
            bail!("Dictation canceled.");
        }
        let clipboard = crate::clipboard::ClipboardLease::write(text)?;
        self.verify()?;
        if cancel.is_cancelled() || !clipboard.owns_clipboard() {
            bail!("Dictation canceled or the clipboard changed.");
        }
        down.post_to_pid(self.pid);
        up.post_to_pid(self.pid);
        let expected = self
            .original_value
            .as_ref()
            .and_then(|value| replace_selection(value, self.selection, text));
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(40));
            if let Some(expected) = &expected
                && optional_attr(&self.element, kAXValueAttribute)
                    .ok()
                    .flatten()
                    .and_then(|value| string(value).ok())
                    .as_ref()
                    == Some(expected)
            {
                drop(clipboard);
                return Ok(DeliveryOutcome::Verified(format!(
                    "Pasted into {}.",
                    self.name
                )));
            }
        }
        drop(clipboard);
        Ok(DeliveryOutcome::Unconfirmed(format!(
            "Paste sent to {}. Copy is available from Peep’s menu if needed.",
            self.name
        )))
    }
}

fn prefers_paste(bundle: &str) -> bool {
    matches!(
        bundle,
        "com.openai.codex"
            | "com.openai.chat"
            | "com.t3tools.t3code"
            | "com.apple.Safari"
            | "app.zen-browser.zen"
            | "org.mozilla.firefox"
            | "com.microsoft.VSCode"
            | "com.hnc.Discord"
            | "com.tinyspeck.slackmacgap"
    ) || bundle.starts_with("com.google.Chrome")
        || bundle.starts_with("com.microsoft.edgemac")
}
fn replace_selection(value: &str, selection: (isize, isize), text: &str) -> Option<String> {
    let mut units: Vec<u16> = value.encode_utf16().collect();
    let start = usize::try_from(selection.0).ok()?;
    let end = start.checked_add(usize::try_from(selection.1).ok()?)?;
    if end > units.len() {
        return None;
    }
    units.splice(start..end, text.encode_utf16());
    String::from_utf16(&units).ok()
}

pub enum Job {
    Capture(u64, i32, tokio_util::sync::CancellationToken),
    Deliver(u64, String),
    Discard(u64),
    Shutdown,
}
pub fn spawn(
    events: async_channel::Sender<peep_core::Event>,
) -> (std::sync::mpsc::Sender<Job>, async_channel::Receiver<()>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let (cleanup_tx, cleanup_rx) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("peep-delivery".into())
        .spawn(move || {
            let mut target = None;
            for job in rx {
                if matches!(job, Job::Shutdown) {
                    drop(target);
                    let _ = cleanup_tx.try_send(());
                    break;
                }
                objc2::rc::autoreleasepool(|_| match job {
                    Job::Capture(id, pid, cancel) => {
                        let captured = Target::capture(pid);
                        if !cancel.is_cancelled() {
                            let (kind, text) = match &captured {
                                Ok(_) => ("target_ready", String::new()),
                                Err(error) => ("error", error.to_string()),
                            };
                            let _ = events.try_send(peep_core::Event {
                                kind: kind.into(),
                                id,
                                text,
                                ..Default::default()
                            });
                        }
                        target = Some((id, captured, cancel));
                    }
                    Job::Deliver(id, text) => {
                        let result = match target.take() {
                            Some((expected, result, cancel)) if expected == id => {
                                result.and_then(|t| t.insert(&text, &cancel))
                            }
                            _ => Err(anyhow!("The original editor is unavailable.")),
                        };
                        let (kind, text) = match result {
                            Ok(DeliveryOutcome::Verified(text)) => ("delivered", text),
                            Ok(DeliveryOutcome::Unconfirmed(text)) => ("paste_sent", text),
                            Err(error) => ("error", error.to_string()),
                        };
                        let _ = events.try_send(peep_core::Event {
                            kind: kind.into(),
                            id,
                            text,
                            ..Default::default()
                        });
                    }
                    Job::Discard(id) => {
                        if target.as_ref().map(|x| x.0) == Some(id) {
                            target = None;
                        }
                    }
                    Job::Shutdown => {
                        target = None;
                    }
                });
            }
        })
        .expect("start delivery worker");
    (tx, cleanup_rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chat_and_browser_editors_use_paste() {
        assert!(prefers_paste("com.openai.codex"));
        assert!(prefers_paste("com.openai.chat"));
        assert!(prefers_paste("com.t3tools.t3code"));
        assert!(prefers_paste("com.google.Chrome.canary"));
        assert!(!prefers_paste("com.apple.TextEdit"));
    }
    #[test]
    fn expected_paste_preserves_surrounding_text_and_utf16_selection() {
        assert_eq!(
            replace_selection("a😀b", (1, 2), "hello"),
            Some("ahellob".into())
        );
        assert_eq!(
            replace_selection("ab", (1, 0), " hello "),
            Some("a hello b".into())
        );
        assert_eq!(replace_selection("ab", (4, 0), "x"), None);
        assert_eq!(replace_selection("ab", (-1, 0), "x"), None);
    }
}
