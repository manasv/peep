use anyhow::{Result, anyhow, bail};
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_app_kit::{
    NSPasteboard, NSPasteboardContentsOptions, NSPasteboardItem, NSPasteboardTypeString,
    NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSString};

pub struct ClipboardLease {
    board: Retained<NSPasteboard>,
    previous: Vec<Retained<NSPasteboardItem>>,
    ownership: isize,
}
impl ClipboardLease {
    pub fn write(text: &str) -> Result<Self> {
        let board = NSPasteboard::generalPasteboard();
        Self::write_on(board, text)
    }
    fn write_on(board: Retained<NSPasteboard>, text: &str) -> Result<Self> {
        let original_count = board.changeCount();
        let mut previous = Vec::new();
        let mut bytes = 0usize;
        if let Some(items) = board.pasteboardItems() {
            for item in items.iter() {
                let copy = NSPasteboardItem::new();
                for kind in item.types().iter() {
                    let data = item.dataForType(&kind).ok_or_else(|| {
                        anyhow!("The current clipboard can’t be preserved. Use Copy text instead.")
                    })?;
                    bytes = bytes
                        .checked_add(data.len())
                        .ok_or_else(|| anyhow!("Clipboard is too large to preserve."))?;
                    if bytes > 32 * 1024 * 1024 {
                        bail!(
                            "The current clipboard is too large to preserve. Use Copy text instead."
                        );
                    }
                    if !copy.setData_forType(&data, &kind) {
                        bail!("The current clipboard can’t be preserved.");
                    }
                }
                previous.push(copy);
            }
        }
        if board.changeCount() != original_count {
            bail!("The clipboard changed before dictation could be pasted.");
        }
        let ownership =
            board.prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
        let lease = Self {
            board,
            previous,
            ownership,
        };
        if !lease.owns_clipboard()
            || !unsafe {
                lease
                    .board
                    .setString_forType(&NSString::from_str(text), NSPasteboardTypeString)
            }
        {
            bail!("Could not prepare the dictation clipboard.");
        }
        Ok(lease)
    }
    pub fn owns_clipboard(&self) -> bool {
        may_restore(self.ownership, self.board.changeCount())
    }
}
fn may_restore(ownership: isize, current: isize) -> bool {
    ownership == current
}
impl Drop for ClipboardLease {
    fn drop(&mut self) {
        if !self.owns_clipboard() {
            return;
        }
        self.board
            .prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
        if !self.previous.is_empty() {
            let writers: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = self
                .previous
                .drain(..)
                .map(ProtocolObject::from_retained)
                .collect();
            let _ = self
                .board
                .writeObjects(&NSArray::from_retained_slice(&writers));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::NSData;
    #[test]
    fn newer_clipboard_content_is_never_restored_over() {
        assert!(may_restore(12, 12));
        assert!(!may_restore(12, 13));
        assert!(!may_restore(12, 11));
    }

    #[test]
    fn restores_all_formats_on_an_isolated_pasteboard() {
        objc2::rc::autoreleasepool(|_| {
            let board = NSPasteboard::pasteboardWithUniqueName();
            let custom_type = NSString::from_str("dev.peep.test-data");
            unsafe {
                assert!(board.setString_forType(
                    &NSString::from_str("previous text"),
                    NSPasteboardTypeString
                ));
            }
            assert!(
                board.setData_forType(Some(&NSData::with_bytes(b"binary\0payload")), &custom_type)
            );
            let lease = ClipboardLease::write_on(board.clone(), "dictation").unwrap();
            unsafe {
                assert_eq!(
                    board
                        .stringForType(NSPasteboardTypeString)
                        .unwrap()
                        .to_string(),
                    "dictation"
                );
            }
            drop(lease);
            unsafe {
                assert_eq!(
                    board
                        .stringForType(NSPasteboardTypeString)
                        .unwrap()
                        .to_string(),
                    "previous text"
                );
            }
            assert_eq!(
                board.dataForType(&custom_type).unwrap().to_vec(),
                b"binary\0payload"
            );
        });
    }

    #[test]
    fn leaves_a_new_copy_untouched_on_an_isolated_pasteboard() {
        objc2::rc::autoreleasepool(|_| {
            let board = NSPasteboard::pasteboardWithUniqueName();
            let lease = ClipboardLease::write_on(board.clone(), "dictation").unwrap();
            board.prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
            unsafe {
                assert!(
                    board.setString_forType(
                        &NSString::from_str("newer copy"),
                        NSPasteboardTypeString
                    )
                );
            }
            drop(lease);
            unsafe {
                assert_eq!(
                    board
                        .stringForType(NSPasteboardTypeString)
                        .unwrap()
                        .to_string(),
                    "newer copy"
                );
            }
        });
    }
}
