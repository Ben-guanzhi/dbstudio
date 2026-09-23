//! Settings overlay: theme / safe mode / Vim toggles with live values,
//! opened from the header gear button, `Cmd/Ctrl+,` or the command palette.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    label::Label,
    v_flex, ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
};

use crate::state::AppState;

/// Actions the settings card delegates to the workspace (they need app-crate
/// or editor access the ui crate does not have).
pub enum SettingsEvent {
    ToggleTheme,
    ToggleVim,
}

impl EventEmitter<SettingsEvent> for SettingsModal {}

pub struct SettingsModal {
    is_open: bool,
    _subscriptions: Vec<Subscription>,
}

impl SettingsModal {
    pub fn view(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            // Re-render whenever safe mode / vim mode change so the ON/OFF
            // buttons always reflect the global state.
            let _subscriptions = vec![cx.observe_global::<AppState>(|_this, cx| cx.notify())];
            Self {
                is_open: false,
                _subscriptions,
            }
        })
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn open(&mut self, cx: &mut Context<Self>) {
        self.is_open = true;
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.is_open = false;
        cx.notify();
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.is_open {
            self.close(cx);
        } else {
            self.open(cx);
        }
    }

    /// One settings row: label + description on the left, control on the right.
    fn row(title: &str, desc: &str, control: impl IntoElement, cx: &Context<Self>) -> Div {
        h_flex()
            .items_center()
            .justify_between()
            .gap_3()
            .py_2()
            .child(
                v_flex()
                    .gap_0p5()
                    .child(Label::new(title).text_sm().font_medium())
                    .child(
                        Label::new(desc)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground),
                    ),
            )
            .child(control)
    }
}

impl Render for SettingsModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.is_open {
            return div().into_any_element();
        }

        let state = cx.global::<AppState>();
        let safe_mode = state.safe_mode;
        let vim_mode = state.vim_mode;
        let version = env!("CARGO_PKG_VERSION");

        let theme_control = Button::new("settings-theme")
            .label("Switch")
            .small()
            .ghost()
            .on_click(cx.listener(|_this, _: &ClickEvent, _window, cx| {
                cx.emit(SettingsEvent::ToggleTheme);
            }));

        let safe_control = Button::new("settings-safe-mode")
            .label(if safe_mode { "ON" } else { "OFF" })
            .small()
            .when(safe_mode, |b| b.primary())
            .when(!safe_mode, |b| b.ghost())
            .on_click(cx.listener(|_this, _: &ClickEvent, _window, cx| {
                crate::state::toggle_safe_mode(cx);
            }));

        let vim_control = Button::new("settings-vim")
            .label(if vim_mode { "ON" } else { "OFF" })
            .small()
            .when(vim_mode, |b| b.primary())
            .when(!vim_mode, |b| b.ghost())
            .on_click(cx.listener(|_this, _: &ClickEvent, _window, cx| {
                cx.emit(SettingsEvent::ToggleVim);
            }));

        let header = h_flex()
            .id("settings-modal-header")
            .justify_between()
            .items_center()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(Label::new("Settings").text_base().font_semibold())
            .child(
                Button::new("settings-close")
                    .icon(Icon::new(IconName::Close).size_3_5())
                    .ghost()
                    .small()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.close(cx);
                    })),
            );

        let body = v_flex()
            .id("settings-modal-body")
            .px_4()
            .py_3()
            .gap_1()
            .child(
                Label::new("Appearance")
                    .text_sm()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(Self::row(
                "Theme",
                "Switch between light and dark appearance",
                theme_control,
                cx,
            ))
            .child(
                Label::new("Editing")
                    .text_sm()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(Self::row(
                "Safe Mode",
                "Require confirmation for every write statement",
                safe_control,
                cx,
            ))
            .child(Self::row(
                "Vim Mode",
                "Vim key bindings in the SQL editor",
                vim_control,
                cx,
            ));

        let footer = h_flex()
            .id("settings-modal-footer")
            .px_4()
            .py_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                Label::new(format!("{} v{}", dbstudio_core::APP_NAME, version))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground),
            );

        let card = v_flex()
            .id("settings-modal")
            .w(px(480.0))
            .rounded(px(8.0))
            .bg(cx.theme().background)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_lg()
            .overflow_hidden()
            .on_click(|_e, _window, cx| {
                cx.stop_propagation();
            })
            .child(header)
            .child(body)
            .child(footer);

        div()
            .id("settings-modal-overlay")
            .absolute()
            .inset_0()
            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                this.close(cx);
            }))
            .child(div().absolute().inset_0().bg(gpui::black().opacity(0.5)))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .items_center()
                    .justify_center()
                    .child(card),
            )
            .into_any_element()
    }
}
