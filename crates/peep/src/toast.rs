use super::*;

#[cfg(target_os = "linux")]
pub fn show(owner: &mut Peep, message: &str, recover_text: bool, _: &mut Context<Peep>) {
    send(Command::Notify {
        text: message.to_owned(),
        recover_text: recover_text.then(|| owner.session.transcript.clone()),
    });
}

#[cfg(not(target_os = "linux"))]
pub fn show(owner: &mut Peep, message: &str, recover_text: bool, cx: &mut Context<Peep>) {
    if let Some(previous) = owner.toast.take() {
        let _ = cx.update_window(previous, |_, window, _| window.remove_window());
    }
    let Some(display) = cx.primary_display() else {
        return;
    };
    let screen = display.bounds();
    let width = px(380.).min(screen.size.width - px(32.));
    let height = px(if recover_text { 164. } else { 128. });
    let bounds = Bounds::new(
        point(
            screen.origin.x + (screen.size.width - width) / 2.,
            screen.origin.y + screen.size.height - height - px(80.),
        ),
        size(width, height),
    );
    let message = message.to_owned();
    let recovery = recover_text.then(|| owner.session.transcript.clone());
    #[cfg(target_os = "macos")]
    let settings = cx
        .update_window(owner.window, |_, window, _| {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match HasWindowHandle::window_handle(window).ok()?.as_raw() {
                RawWindowHandle::AppKit(handle) => Some(handle.ns_view),
                _ => None,
            }
        })
        .ok()
        .flatten();
    let weak = cx.weak_entity();
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id: Some(display.id()),
            titlebar: None,
            focus: false,
            show: false,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: WindowBackgroundAppearance::Transparent,
            ..Default::default()
        },
        |window, cx| {
            window.defer(cx, move |window, _| {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                let Ok(handle) = HasWindowHandle::window_handle(window) else {
                    return;
                };
                match handle.as_raw() {
                    #[cfg(target_os = "macos")]
                    RawWindowHandle::AppKit(handle) => unsafe {
                        peep_macos::show_toast_window(handle.ns_view, settings)
                    },
                    #[cfg(target_os = "windows")]
                    RawWindowHandle::Win32(handle) => {
                        let scale = window.scale_factor();
                        if let Err(error) = platform::show_toast_window(
                            handle.hwnd.get(),
                            f32::from(bounds.origin.x) * scale,
                            f32::from(bounds.origin.y) * scale,
                            f32::from(bounds.size.width) * scale,
                            f32::from(bounds.size.height) * scale,
                        ) {
                            eprintln!("Peep notification could not be shown: {error}");
                        }
                    }
                    _ => {}
                }
            });
            cx.new(|cx| {
                cx.spawn_in(window, async move |_, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(7))
                        .await;
                    let _ = cx.update(|window, _| window.remove_window());
                })
                .detach();
                Toast {
                    message,
                    recovery,
                    owner: weak,
                }
            })
        },
    );
    match result {
        Ok(handle) => owner.toast = Some(handle.into()),
        Err(error) => eprintln!("Peep notification could not be shown: {error}"),
    }
}

#[cfg(not(target_os = "linux"))]
struct Toast {
    message: String,
    recovery: Option<String>,
    owner: WeakEntity<Peep>,
}

#[cfg(not(target_os = "linux"))]
impl Toast {
    fn act(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(expected) = &self.recovery {
            let _ = self
                .owner
                .update(cx, |owner, cx| owner.toast_action(action, expected, cx));
        }
        window.remove_window();
    }
}

#[cfg(not(target_os = "linux"))]
impl Render for Toast {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let (background, text, muted, border) = if dark {
            (0x2c2723, 0xf8f1e5, 0xb7a99b, 0x494039)
        } else {
            (0xfffcf6, 0x302820, 0x75685c, 0xe5dbcd)
        };
        div()
            .size_full()
            .rounded_xl()
            .bg(rgb(background))
            .border_1()
            .border_color(rgb(border))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .font_family(platform::FONT)
            .text_sm()
            .text_color(rgb(text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xs().text_color(rgb(muted)).child("Peep"))
                    .child(
                        div()
                            .id("dismiss")
                            .px_2()
                            .cursor_pointer()
                            .text_color(rgb(muted))
                            .child("×")
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    ),
            )
            .child(div().flex_1().overflow_hidden().child(self.message.clone()))
            .when(self.recovery.is_some(), |view| {
                view.child(
                    div()
                        .flex()
                        .gap_3()
                        .child(
                            div()
                                .id("copy")
                                .cursor_pointer()
                                .child("Copy text")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.act("copy", window, cx)),
                                ),
                        )
                        .child(
                            div()
                                .id("discard")
                                .cursor_pointer()
                                .text_color(rgb(muted))
                                .child("Discard")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.act("discard", window, cx)
                                })),
                        ),
                )
            })
    }
}
