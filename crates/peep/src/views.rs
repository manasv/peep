use super::*;

#[derive(Clone, Copy)]
struct Palette {
    base: u32,
    surface: u32,
    line: u32,
    text: u32,
    muted: u32,
    accent: u32,
    tinted: u32,
    green: u32,
}
impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                base: 0x211e1b,
                surface: 0x2c2723,
                line: 0x494039,
                text: 0xf8f1e5,
                muted: 0xb7a99b,
                accent: 0xf1b18e,
                tinted: 0x423027,
                green: 0xa7c5a1,
            }
        } else {
            Self {
                base: 0xf8f4eb,
                surface: 0xfffcf6,
                line: 0xe5dbcd,
                text: 0x302820,
                muted: 0x75685c,
                accent: 0x985332,
                tinted: 0xf6e5d5,
                green: 0x4f7350,
            }
        }
    }
}
impl Peep {
    fn toggle(
        &self,
        index: usize,
        checked: bool,
        action: UiAction,
        enabled: bool,
        p: Palette,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(("toggle", index))
            .flex()
            .items_center()
            .gap_2()
            .px_1()
            .py_1()
            .rounded_lg()
            .border_2()
            .border_color(rgba(0x00000000))
            .child(
                div()
                    .w(px(24.))
                    .text_xs()
                    .text_color(rgb(p.muted))
                    .child(if checked { "On" } else { "Off" }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .w(px(44.))
                    .h(px(26.))
                    .px(px(3.))
                    .rounded_full()
                    .bg(rgb(if checked { p.green } else { p.line }))
                    .when(checked, |d| d.justify_end())
                    .child(div().size(px(20.)).rounded_full().bg(rgb(0xfffcf6))),
            )
            .when(!enabled, |d| d.opacity(0.4))
            .when(enabled, |d| {
                d.cursor_pointer()
                    .track_focus(&self.buttons[index])
                    .hover(|d| d.bg(rgb(p.tinted)))
                    .focus(|d| d.border_color(rgb(p.accent)))
                    .on_click(cx.listener(move |this, _, _, cx| this.action(action, cx)))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.action(action, cx);
                            cx.stop_propagation();
                        }
                    }))
            })
    }

    fn control(
        &self,
        index: usize,
        label: impl Into<SharedString>,
        action: UiAction,
        enabled: bool,
        p: Palette,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(("control", index))
            .flex()
            .items_center()
            .justify_center()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.line))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(p.text))
            .child(label.into())
            .when(!enabled, |d| d.opacity(0.4))
            .when(enabled, |d| {
                d.cursor_pointer()
                    .track_focus(&self.buttons[index])
                    .hover(|d| d.border_color(rgb(p.accent)).bg(rgb(p.tinted)))
                    .focus(|d| d.border_color(rgb(p.accent)))
                    .on_click(cx.listener(move |this, _, _, cx| this.action(action, cx)))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.action(action, cx);
                            cx.stop_propagation();
                        }
                    }))
            })
    }
    fn card(&self, p: Palette) -> Div {
        div()
            .flex()
            .flex_col()
            .bg(rgb(p.surface))
            .border_1()
            .border_color(rgb(p.line))
            .rounded_xl()
            .px_4()
    }
    fn setting_row(
        &self,
        title: &str,
        description: &str,
        control: Stateful<Div>,
        p: Palette,
    ) -> Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .py_2()
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(title.to_owned()),
                    )
                    .when(!description.is_empty(), |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(rgb(p.muted))
                                .line_height(relative(1.4))
                                .child(description.to_owned()),
                        )
                    }),
            )
            .child(control)
    }
    fn access_row(&self, microphone: bool, index: usize, p: Palette, cx: &Context<Self>) -> Div {
        let allowed = if microphone {
            self.preferences.microphone == "Allowed"
        } else {
            self.preferences.accessibility
        };
        self.setting_row(
            if microphone {
                "Microphone"
            } else {
                "Text insertion"
            },
            if allowed {
                "Manage in System Settings"
            } else if microphone {
                "Click to allow microphone access"
            } else {
                "Click to allow text insertion"
            },
            self.toggle(
                index,
                allowed,
                if microphone {
                    UiAction::Microphone
                } else {
                    UiAction::Accessibility
                },
                true,
                p,
                cx,
            ),
            p,
        )
    }
    fn setup(&self, p: Palette, cx: &Context<Self>) -> Div {
        let phase = self.session.phase;
        div()
            .flex()
            .flex_col()
            .gap_3()
            .when(phase == Phase::NeedsModel, |d| {
                d.child(
                    self.card(p)
                        .py_4()
                        .gap_3()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child("Set up offline dictation"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(p.muted))
                                .child("One download. Your speech stays on this Mac."),
                        )
                        .child(div().flex().gap_2().child(self.control(
                            7,
                            "Download · 632 MB",
                            UiAction::Download,
                            true,
                            p,
                            cx,
                        ))),
                )
            })
            .when(phase == Phase::Downloading, |d| {
                d.child(
                    self.card(p)
                        .py_4()
                        .gap_3()
                        .child(div().text_xs().text_color(rgb(p.muted)).child(format!(
                            "Downloading offline dictation · {:.0}%",
                            self.session.progress * 100.
                        )))
                        .child(
                            div().h_1().w_full().rounded_full().bg(rgb(p.tinted)).child(
                                div()
                                    .h_full()
                                    .rounded_full()
                                    .bg(rgb(p.accent))
                                    .w(relative(self.session.progress as f32)),
                            ),
                        ),
                )
            })
    }
}
impl Render for Peep {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::new(matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ));
        let phase = self.session.phase;
        let busy = self.session.busy();
        let title = if phase == Phase::Preparing {
            "Getting dictation ready…"
        } else {
            phase.title()
        };
        let shortcut = if self.preferences.shortcut.is_empty() {
            "Control–Shift–Space"
        } else {
            &self.preferences.shortcut
        };
        div()
            .id("peep")
            .track_focus(&self.focus)
            .key_context("Peep")
            .size_full()
            .font_family(".SystemUIFont")
            .text_sm()
            .text_color(rgb(p.text))
            .bg(rgb(p.base))
            .on_action(
                cx.listener(|this, _: &CancelRecording, _, cx| this.action(UiAction::Cancel, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ShowSettings, _, cx| this.action(UiAction::Settings, cx)),
            )
            .on_action(cx.listener(|this, _: &Quit, _, _| this.quit()))
            .on_action(cx.listener(|_, _: &NextFocus, window, _| window.focus_next()))
            .on_action(cx.listener(|_, _: &PreviousFocus, window, _| window.focus_prev()))
            .child(
                div()
                    .on_children_prepainted(|bounds, window, cx| {
                        let Some(content) = bounds.first() else {
                            return;
                        };
                        if window.is_fullscreen() {
                            return;
                        }
                        let current = window.viewport_size();
                        let height = content
                            .size
                            .height
                            .min(px(peep_macos::available_content_height()));
                        if (height - current.height).abs() > px(1.) {
                            window.defer(cx, move |window, _| {
                                window.resize(size(current.width, height))
                            });
                        }
                    })
                    .id("content-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .flex_shrink_0()
                            .px_4()
                            .pb_4()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .id("titlebar")
                                    .window_control_area(WindowControlArea::Drag)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .py_1()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(img(self.logo.clone()).size_6())
                                            .child(
                                                div()
                                                    .text_lg()
                                                    .font_weight(FontWeight::BOLD)
                                                    .child("Peep"),
                                            ),
                                    ),
                            )
                            .when(busy, |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .items_start()
                                        .gap_2()
                                        .py_2()
                                        .child(
                                            div()
                                                .text_lg()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(title),
                                        )
                                        .when(self.session.can_cancel(), |d| {
                                            d.child(
                                                div()
                                                    .flex()
                                                    .gap_2()
                                                    .mt_2()
                                                    .when(phase == Phase::Recording, |d| {
                                                        d.child(self.control(
                                                            9,
                                                            "Finish",
                                                            UiAction::Stop,
                                                            true,
                                                            p,
                                                            cx,
                                                        ))
                                                    })
                                                    .child(self.control(
                                                        10,
                                                        "Cancel",
                                                        UiAction::Cancel,
                                                        true,
                                                        p,
                                                        cx,
                                                    )),
                                            )
                                        }),
                                )
                            })
                            .when(
                                matches!(phase, Phase::NeedsModel | Phase::Downloading),
                                |d| d.child(self.setup(p, cx)),
                            )
                            .child(
                                self.card(p)
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .gap_4()
                                            .py_2()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .flex()
                                                    .flex_col()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .child("Keyboard shortcut"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(rgb(p.accent))
                                                            .text_ellipsis()
                                                            .child(
                                                                if self.shortcut_problem.is_empty()
                                                                {
                                                                    "Shortcut unavailable"
                                                                        .to_owned()
                                                                } else {
                                                                    self.shortcut_problem.clone()
                                                                },
                                                            )
                                                            .when(
                                                                self.shortcut_problem.is_empty(),
                                                                |d| d.opacity(0.),
                                                            ),
                                                    ),
                                            )
                                            .child(
                                                self.control(
                                                    0,
                                                    if self.recording_shortcut {
                                                        "Press shortcut…".to_owned()
                                                    } else {
                                                        shortcut.to_owned()
                                                    },
                                                    UiAction::RecordShortcut,
                                                    !busy && !self.recording_shortcut,
                                                    p,
                                                    cx,
                                                )
                                                .flex_shrink_0()
                                                .whitespace_nowrap()
                                                .bg(rgb(p.tinted))
                                                .text_color(rgb(p.accent)),
                                            ),
                                    )
                                    .child(div().h_px().bg(rgb(p.line)))
                                    .child(self.setting_row(
                                        "Dictation",
                                        "",
                                        self.toggle(
                                            1,
                                            self.preferences.shortcut_enabled,
                                            UiAction::ShortcutEnabled,
                                            !busy,
                                            p,
                                            cx,
                                        ),
                                        p,
                                    ))
                                    .child(div().h_px().bg(rgb(p.line)))
                                    .child(self.setting_row(
                                        "Floating indicator",
                                        "",
                                        self.toggle(
                                            2,
                                            self.preferences.indicator,
                                            UiAction::Indicator,
                                            true,
                                            p,
                                            cx,
                                        ),
                                        p,
                                    ))
                                    .child(div().h_px().bg(rgb(p.line)))
                                    .child(self.access_row(true, 4, p, cx))
                                    .child(div().h_px().bg(rgb(p.line)))
                                    .child(self.access_row(false, 5, p, cx)),
                            )
                            .child(
                                div().flex().justify_end().child(
                                    self.control(
                                        6,
                                        "Refresh permissions",
                                        UiAction::Refresh,
                                        true,
                                        p,
                                        cx,
                                    )
                                    .border_0()
                                    .bg(rgb(p.base))
                                    .text_color(rgb(p.muted)),
                                ),
                            ),
                    ),
            )
    }
}
