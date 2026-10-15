//! Settings overlay: theme / safe mode / Vim toggles with live values,
//! opened from the header gear button, `Cmd/Ctrl+,` or the command palette.

use std::path::Path;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    label::Label,
    v_flex, ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
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
    /// A failed trust/revoke surfaces here; the buttons don't disable, so the
    /// user can fix the underlying problem (unreadable file, unwritable store)
    /// and try again instead of being stuck with a silent no-op.
    error: Option<String>,
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
                error: None,
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

    /// 插件目录里的每个库文件 + 信任判定，以及「信任 / 撤销」按钮。
    ///
    /// 信任清单在磁盘上，插件目录也随时可写：不加这块的话，一个被拦下的插件只会
    /// 变成日志里的一行 `warn!`，用户根本不知道有文件、也无处登记。
    fn plugins_section(cx: &Context<Self>) -> Div {
        let mut section = v_flex().gap_0p5().mt_1().child(
            Label::new("Plugins")
                .text_sm()
                .font_semibold()
                .text_color(cx.theme().muted_foreground),
        );

        let files = dbstudio_db::plugin_manager::scan_plugins();
        if files.is_empty() {
            return section.child(
                Label::new("No plugin libraries installed.")
                    .text_xs()
                    .text_color(cx.theme().muted_foreground),
            );
        }

        for file in files {
            let trusted = file.decision == dbstudio_db::TrustDecision::Trusted;
            let title = file.file_name();
            let desc = match &file.decision {
                dbstudio_db::TrustDecision::Trusted => {
                    format!("Trusted · {} B — loaded", file.bytes)
                }
                dbstudio_db::TrustDecision::Unknown => {
                    format!("Not trusted — blocked from loading · {} B", file.bytes)
                }
                dbstudio_db::TrustDecision::Tampered { actual, .. } => {
                    format!(
                        "Content changed since registration ({}) · {} B",
                        hash_prefix(actual),
                        file.bytes
                    )
                }
            };

            let control = h_flex()
                .gap_1()
                .child(
                    Button::new(format!("settings-plugin-revoke-{title}"))
                        .label("Revoke")
                        .small()
                        .ghost()
                        .disabled(!trusted)
                        .on_click(cx.listener({
                            let path = file.path.clone();
                            move |this, _: &ClickEvent, _window, cx| {
                                this.plugin_trust_action(cx, &path, false);
                            }
                        })),
                )
                .child(
                    Button::new(format!("settings-plugin-trust-{title}"))
                        .label(if trusted { "Re-trust" } else { "Trust" })
                        .small()
                        .when(trusted, |b| b.ghost())
                        .when(!trusted, |b| b.primary())
                        .on_click(cx.listener({
                            let path = file.path.clone();
                            move |this, _: &ClickEvent, _window, cx| {
                                this.plugin_trust_action(cx, &path, true);
                            }
                        })),
                );

            section = section.child(Self::row(&title, &desc, control, cx));
        }

        section
    }

    /// 登记或撤销一个插件，然后重新扫描，使它不必重启就生效。
    ///
    /// 扫描本身要 `dlopen` 插件目录，所以这里在 UI 线程上同步做：插件数量是个位数，
    /// 代价是一次文件系统读取加一次库加载，不值得为此引入一个异步往返。
    fn plugin_trust_action(&mut self, cx: &mut Context<Self>, path: &Path, trust: bool) {
        // The two operations return different payloads, so normalise them onto
        // one shape: `Some(hash)` means "trusted", `None` means "deregistered".
        let outcome = if trust {
            dbstudio_db::plugin_manager::trust_plugin(path).map(Some)
        } else {
            dbstudio_db::plugin_manager::revoke_plugin(path).map(|_| None)
        };

        match outcome {
            Ok(Some(hash)) => {
                self.update_status(
                    cx,
                    format!("Plugin trusted and loaded: {}", hash_prefix(&hash)),
                );
            }
            Ok(None) => {
                self.update_status(cx, "Plugin deregistered and unloaded".to_string());
            }
            Err(e) => {
                self.error = Some(e.to_string());
            }
        }
        cx.notify();
    }

    fn update_status(&self, cx: &mut Context<Self>, message: String) {
        cx.update_global::<AppState, _>(|state, _cx| {
            state.status_message = message;
        });
    }
}

/// 信任列表里只显示摘要前缀：完整 64 位 hex 在界面上一行放不下也不好比对。
fn hash_prefix(hash: &str) -> String {
    hash.get(..12).unwrap_or(hash).to_string()
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
            ))
            .child(Self::plugins_section(cx))
            .when_some(self.error.clone(), |body, error| {
                body.child(Label::new(error).text_xs().text_color(cx.theme().danger))
            });

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
