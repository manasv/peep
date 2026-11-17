use anyhow::{Result, bail};
use block2::RcBlock;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
};
use objc2_core_audio::{
    AudioObjectAddPropertyListenerBlock, AudioObjectPropertyAddress,
    AudioObjectRemovePropertyListenerBlock, kAudioHardwarePropertyDefaultInputDevice,
    kAudioObjectPropertyElementMain, kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
};
use objc2_foundation::{NSNotificationCenter, NSObjectProtocol};
use peep_core::Event;
use std::ptr::NonNull;

type DeviceListener = RcBlock<dyn Fn(u32, NonNull<AudioObjectPropertyAddress>)>;
pub struct Interruptions {
    center: Retained<NSNotificationCenter>,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    device_listener: DeviceListener,
}
fn address() -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyDefaultInputDevice,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}
impl Interruptions {
    pub fn new(events: async_channel::Sender<Event>) -> Result<Self> {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        let mut observers = Vec::new();
        for name in unsafe {
            [
                NSWorkspaceWillSleepNotification,
                NSWorkspaceSessionDidResignActiveNotification,
            ]
        } {
            let events = events.clone();
            let callback = RcBlock::new(move |_| {
                let _ = events.try_send(Event {
                    kind: "capture_interrupted".into(),
                    text: "Recording was interrupted by sleep or a session change.".into(),
                    ..Default::default()
                });
            });
            let observer = unsafe {
                center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &callback)
            };
            observers.push(observer);
        }
        let device_listener: DeviceListener = RcBlock::new(move |_, _| {
            let _ = events.try_send(Event {
                kind: "capture_interrupted".into(),
                text: "The default microphone changed. Start a new recording.".into(),
                ..Default::default()
            });
        });
        let mut property = address();
        let result = unsafe {
            AudioObjectAddPropertyListenerBlock(
                kAudioObjectSystemObject as u32,
                NonNull::from(&mut property),
                None,
                RcBlock::as_ptr(&device_listener) as *mut _,
            )
        };
        if result != 0 {
            for observer in &observers {
                unsafe {
                    center.removeObserver(AsRef::<objc2::runtime::AnyObject>::as_ref(&**observer));
                }
            }
            bail!("Could not observe microphone changes ({result}).");
        }
        Ok(Self {
            center,
            observers,
            device_listener,
        })
    }
}
impl Drop for Interruptions {
    fn drop(&mut self) {
        for observer in &self.observers {
            unsafe {
                self.center
                    .removeObserver(AsRef::<objc2::runtime::AnyObject>::as_ref(&**observer));
            }
        }
        let mut property = address();
        unsafe {
            AudioObjectRemovePropertyListenerBlock(
                kAudioObjectSystemObject as u32,
                NonNull::from(&mut property),
                None,
                RcBlock::as_ptr(&self.device_listener) as *mut _,
            );
        }
    }
}
