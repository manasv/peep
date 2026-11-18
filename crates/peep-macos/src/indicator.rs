use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSFont, NSPanel, NSScreen, NSTextField, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::{
    cell::{Cell, RefCell},
    time::{Duration, Instant},
};

const BAR_COUNT: usize = 9;
const BAR_WIDTH: f64 = 3.;
const BAR_GAP: f64 = 3.;
const WAVE_HEIGHT: f64 = 24.;
const FEEDBACK_DURATION: Duration = Duration::from_millis(2500);

#[derive(Default)]
struct Feedback {
    until: Option<Instant>,
}
impl Feedback {
    fn show(&mut self, now: Instant) {
        self.until = Some(now + FEEDBACK_DURATION);
    }
    fn clear(&mut self) {
        self.until = None;
    }
    fn expire(&mut self, now: Instant) -> bool {
        if self.until.is_some_and(|until| now >= until) {
            self.clear();
            true
        } else {
            false
        }
    }
}

#[derive(Default)]
struct Waveform {
    levels: [f64; BAR_COUNT],
}
impl Waveform {
    fn push(&mut self, level: f64) {
        self.levels.rotate_left(1);
        self.levels[BAR_COUNT - 1] = if level.is_finite() {
            level.clamp(0., 1.)
        } else {
            0.
        };
    }
    fn reset(&mut self) {
        self.levels.fill(0.);
    }
}

pub struct Indicator {
    panel: Retained<NSPanel>,
    label: Retained<NSTextField>,
    bars: Vec<Retained<NSView>>,
    recording: Cell<bool>,
    waveform: RefCell<Waveform>,
    feedback: RefCell<Feedback>,
}
impl Indicator {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(252., 48.)),
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        unsafe {
            panel.setReleasedWhenClosed(false);
        }
        panel.setLevel(25);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setIgnoresMouseEvents(true);
        panel.setHidesOnDeactivate(false);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        let label = NSTextField::labelWithString(&NSString::new(), mtm);
        let background = NSVisualEffectView::initWithFrame(
            NSVisualEffectView::alloc(mtm),
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(252., 48.)),
        );
        background.setMaterial(NSVisualEffectMaterial::HUDWindow);
        background.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        background.setState(NSVisualEffectState::Active);
        background.setWantsLayer(true);
        if let Some(layer) = background.layer() {
            layer.setCornerRadius(16.);
            layer.setMasksToBounds(true);
        }
        panel.setContentView(Some(&background));
        label.setFrame(NSRect::new(NSPoint::new(82., 13.), NSSize::new(152., 22.)));
        label.setFont(Some(&NSFont::systemFontOfSize(14.)));
        if let Some(content) = panel.contentView() {
            content.addSubview(&label);
        }
        let bars = (0..BAR_COUNT)
            .map(|index| {
                let bar = NSView::initWithFrame(
                    NSView::alloc(mtm),
                    NSRect::new(
                        NSPoint::new(18. + index as f64 * (BAR_WIDTH + BAR_GAP), 22.),
                        NSSize::new(BAR_WIDTH, 4.),
                    ),
                );
                bar.setWantsLayer(true);
                if let Some(layer) = bar.layer() {
                    layer.setCornerRadius(BAR_WIDTH / 2.);
                    layer.setBackgroundColor(Some(&NSColor::systemOrangeColor().CGColor()));
                }
                background.addSubview(&bar);
                bar.setHidden(true);
                bar
            })
            .collect();
        Self {
            panel,
            label,
            bars,
            recording: Cell::new(false),
            waveform: RefCell::new(Waveform::default()),
            feedback: RefCell::new(Feedback::default()),
        }
    }
    pub fn update(&self, text: &str, recording: bool, visible: bool, mtm: MainThreadMarker) {
        if !visible && self.feedback.borrow().until.is_some() {
            return;
        }
        self.feedback.borrow_mut().clear();
        self.present(text, recording, visible, mtm);
    }
    pub fn feedback(&self, text: &str, mtm: MainThreadMarker) {
        self.feedback.borrow_mut().show(Instant::now());
        self.present(text, false, true, mtm);
    }
    pub fn dismiss_feedback(&self) {
        if self.feedback.borrow_mut().expire(Instant::now()) {
            self.panel.orderOut(None);
        }
    }
    pub fn hide(&self) {
        self.feedback.borrow_mut().clear();
        self.recording.set(false);
        self.panel.orderOut(None);
    }
    fn present(&self, text: &str, recording: bool, visible: bool, mtm: MainThreadMarker) {
        let recording = recording && visible;
        self.recording.set(recording);
        self.waveform.borrow_mut().reset();
        for bar in &self.bars {
            bar.setHidden(!recording);
        }
        if !visible {
            self.panel.orderOut(None);
            return;
        }
        self.label.setFrame(NSRect::new(
            NSPoint::new(if recording { 82. } else { 18. }, 13.),
            NSSize::new(if recording { 152. } else { 220. }, 22.),
        ));
        if recording {
            self.level(0.);
        }
        self.label.setStringValue(&NSString::from_str(text));
        let color = if recording {
            NSColor::systemRedColor()
        } else {
            NSColor::labelColor()
        };
        self.label.setTextColor(Some(&color));
        if let Some(screen) = NSScreen::mainScreen(mtm) {
            let frame = screen.visibleFrame();
            self.panel.setFrameOrigin(NSPoint::new(
                frame.origin.x + frame.size.width / 2. - 126.,
                frame.origin.y + 52.,
            ));
        }
        self.panel.orderFrontRegardless();
    }
    pub fn level(&self, level: f64) {
        if !self.recording.get() {
            return;
        }
        let mut waveform = self.waveform.borrow_mut();
        waveform.push(level);
        for (bar, level) in self.bars.iter().zip(waveform.levels) {
            let height = BAR_WIDTH + level * (WAVE_HEIGHT - BAR_WIDTH);
            bar.setFrame(NSRect::new(
                NSPoint::new(bar.frame().origin.x, (48. - height) / 2.),
                NSSize::new(BAR_WIDTH, height),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feedback_expires_without_hiding_a_new_message_or_recording() {
        let now = Instant::now();
        let mut feedback = Feedback::default();
        feedback.show(now);
        assert!(!feedback.expire(now + Duration::from_secs(2)));
        feedback.show(now + Duration::from_secs(1));
        assert!(!feedback.expire(now + FEEDBACK_DURATION));
        assert!(feedback.expire(now + Duration::from_secs(1) + FEEDBACK_DURATION));
        assert!(!feedback.expire(now + Duration::from_secs(10)));
        feedback.show(now);
        feedback.clear();
        assert!(!feedback.expire(now + FEEDBACK_DURATION));
    }
    #[test]
    fn waveform_shows_recent_audio_and_returns_to_silence() {
        let mut wave = Waveform::default();
        wave.push(0.7);
        assert_eq!(wave.levels[BAR_COUNT - 1], 0.7);
        for _ in 0..BAR_COUNT {
            wave.push(0.);
        }
        assert_eq!(wave.levels, [0.; BAR_COUNT]);
        wave.push(f64::NAN);
        assert!(wave.levels.iter().all(|level| level.is_finite()));
        wave.push(2.);
        assert_eq!(wave.levels[BAR_COUNT - 1], 1.);
        wave.reset();
        assert_eq!(wave.levels, [0.; BAR_COUNT]);
    }
}
