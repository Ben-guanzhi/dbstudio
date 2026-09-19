use gpui::*;
use gpui_component::ActiveTheme as _;

/// A thin vertical divider for toolbars.
pub fn toolbar_divider(cx: &App) -> impl IntoElement {
    div()
        .w(px(1.0))
        .h(px(16.0))
        .mx_1()
        .bg(cx.theme().border)
}

/// Truncate a string to a maximum number of characters, ensuring UTF-8 safety.
pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        return s.to_string();
    }
    let end = s
        .char_indices()
        .nth(max_len.saturating_sub(3))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    format!("{}...", &s[..end])
}
