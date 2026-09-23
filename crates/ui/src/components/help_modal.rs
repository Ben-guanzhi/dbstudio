//! F1 keyboard-shortcuts help overlay: a centered cheatsheet card rendered as
//! a workspace-level overlay and toggled with `F1` (or the command palette's
//! "Keyboard Shortcuts" entry).

use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    label::Label,
    v_flex, ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
};

/// Display label for gpui's cross-platform primary modifier (`secondary` in
/// keystroke syntax): `Cmd` on macOS, `Ctrl` everywhere else.
#[cfg(target_os = "macos")]
const MOD: &str = "Cmd";
#[cfg(not(target_os = "macos"))]
const MOD: &str = "Ctrl";

/// Display label for the theme-toggle binding (`secondary-ctrl-t`), which
/// collapses to plain `Ctrl+T` outside macOS.
#[cfg(target_os = "macos")]
const THEME_KEY: &str = "Cmd+Ctrl+T";
#[cfg(not(target_os = "macos"))]
const THEME_KEY: &str = "Ctrl+T";

/// Overlay card listing every registered keyboard shortcut.
pub struct HelpModal {
    is_open: bool,
}

impl HelpModal {
    pub fn view(cx: &mut App) -> Entity<Self> {
        cx.new(|_| Self { is_open: false })
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

    /// The full cheatsheet as `(title, rows)`, each row a
    /// `(keys, description)` pair. Kept in sync with `keybindings.rs` and
    /// asserted by unit tests so the help never drifts from real bindings.
    pub fn sections() -> Vec<(&'static str, Vec<(String, String)>)> {
        let m = MOD;
        vec![
            (
                "General",
                vec![
                    (
                        format!("{m}+Enter"),
                        "Run the current SQL query".to_string(),
                    ),
                    (format!("{m}+K"), "Open the command palette".to_string()),
                    (
                        format!("{m}+P"),
                        "Quick open — jump to a table, database or connection".to_string(),
                    ),
                    (
                        "F1".to_string(),
                        "Show or hide this shortcut help".to_string(),
                    ),
                    (
                        format!("{m}+,"),
                        "Open settings (theme, safe mode, Vim)".to_string(),
                    ),
                    (format!("{m}+N"), "Open a new window".to_string()),
                    (format!("{m}+Q"), "Quit dbstudio".to_string()),
                    (format!("{m}+Shift+A"), "Toggle the AI panel".to_string()),
                    (
                        THEME_KEY.to_string(),
                        "Toggle light / dark theme".to_string(),
                    ),
                    ("Shift+Alt+F".to_string(), "Format SQL".to_string()),
                    ("Ctrl+/".to_string(), "Toggle line comment".to_string()),
                ],
            ),
            (
                "Editing",
                vec![
                    (format!("{m}+Shift+D"), "Duplicate line".to_string()),
                    ("Shift+Alt+↑".to_string(), "Move line up".to_string()),
                    ("Shift+Alt+↓".to_string(), "Move line down".to_string()),
                    (format!("{m}+Z"), "Undo last edit".to_string()),
                    (format!("{m}+Shift+Z"), "Redo last edit".to_string()),
                ],
            ),
            (
                "Mouse",
                vec![
                    (
                        "Right-click".to_string(),
                        "Table tree — open Data / Structure / Query, copy name, \
                         refresh, truncate or drop"
                            .to_string(),
                    ),
                    (
                        "Drag dividers".to_string(),
                        "Resize the side panels and the editor / results split".to_string(),
                    ),
                ],
            ),
            (
                "Vim mode",
                vec![
                    ("i / a / o".to_string(), "Enter Insert mode".to_string()),
                    ("Esc".to_string(), "Back to Normal mode".to_string()),
                    ("dd".to_string(), "Delete line".to_string()),
                    ("yy".to_string(), "Yank (copy) line".to_string()),
                    ("p".to_string(), "Paste".to_string()),
                    ("u".to_string(), "Undo".to_string()),
                    ("Ctrl+R".to_string(), "Redo".to_string()),
                    (
                        "v / V / Ctrl+v".to_string(),
                        "Visual mode — character / line / block".to_string(),
                    ),
                ],
            ),
        ]
    }

    fn key_chip(keys: &str, cx: &Context<Self>) -> Div {
        div()
            .px_2()
            .py_1()
            .rounded(cx.theme().radius)
            .bg(cx.theme().tiles)
            .border_1()
            .border_color(cx.theme().border)
            .text_xs()
            .font_family("monospace")
            .text_color(cx.theme().foreground)
            .child(Label::new(keys))
    }
}

impl Render for HelpModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.is_open {
            return div().into_any_element();
        }

        let sections = Self::sections();

        div()
            .id("help-modal-overlay")
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
                    .child(
                        v_flex()
                            .id("help-modal")
                            .w(px(620.0))
                            .max_h(px(560.0))
                            .rounded(px(8.0))
                            .bg(cx.theme().background)
                            .border_1()
                            .border_color(cx.theme().border)
                            .shadow_lg()
                            .overflow_hidden()
                            .on_click(|_e, _window, cx| {
                                cx.stop_propagation();
                            })
                            .child(
                                h_flex()
                                    .id("help-modal-header")
                                    .justify_between()
                                    .items_center()
                                    .px_4()
                                    .py_3()
                                    .border_b_1()
                                    .border_color(cx.theme().border)
                                    .child(
                                        Label::new("Keyboard Shortcuts")
                                            .text_base()
                                            .font_semibold(),
                                    )
                                    .child(
                                        Button::new("help-close")
                                            .icon(Icon::new(IconName::Close).size_3_5())
                                            .ghost()
                                            .small()
                                            .on_click(cx.listener(
                                                |this, _: &ClickEvent, _, cx| {
                                                    this.close(cx);
                                                },
                                            )),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .id("help-modal-body")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .px_4()
                                    .py_3()
                                    .gap_4()
                                    .children(sections.into_iter().map(|(title, rows)| {
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                Label::new(title)
                                                    .text_sm()
                                                    .font_semibold()
                                                    .text_color(cx.theme().muted_foreground),
                                            )
                                            .children(rows.into_iter().map(|(keys, desc)| {
                                                h_flex()
                                                    .items_center()
                                                    .gap_3()
                                                    .child(
                                                        div()
                                                            .w(px(180.0))
                                                            .child(Self::key_chip(&keys, cx)),
                                                    )
                                                    .child(
                                                        Label::new(desc).text_sm().text_color(
                                                            cx.theme().muted_foreground,
                                                        ),
                                                    )
                                            }))
                                    })),
                            )
                            .child(
                                h_flex()
                                    .id("help-modal-footer")
                                    .px_4()
                                    .py_2()
                                    .border_t_1()
                                    .border_color(cx.theme().border)
                                    .child(
                                        Label::new("F1 toggles this help · click outside to close")
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}
