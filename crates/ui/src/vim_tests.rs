use crate::components::vim::{VimBuf, VimMode};

fn buf(text: &str) -> VimBuf {
    let mut v = VimBuf::default();
    v.text = text.to_string();
    v.caret = 0;
    v
}

#[test]
fn motions_are_byte_aware_and_utf8_safe() {
    let mut v = buf("héllo wörld");
    v.step("w", false, false, false);
    assert_eq!(v.caret, 7); // skips the multi-byte 'é'
    v.step("b", false, false, false);
    assert_eq!(v.caret, 0);
    v.step("$", false, false, false);
    assert_eq!(v.caret, 13);
}

#[test]
fn numeric_count_composition() {
    let mut v = buf("abcdef");
    v.step("3", false, false, false);
    v.step("l", false, false, false);
    assert_eq!(v.caret, 3);
}

#[test]
fn insert_roundtrip_after_a() {
    let mut v = buf("ab\ncd");
    v.caret = 1;
    let step = v.step("a", false, false, false).unwrap();
    assert!(step.to_insert);
    assert!(matches!(v.mode, VimMode::Insert));
    let back = v.step("escape", false, false, false).unwrap();
    assert!(!back.to_insert);
    assert!(matches!(v.mode, VimMode::Normal));
}

#[test]
fn open_line_above_and_below_land_on_blank_line() {
    let mut o = buf("a\nb\nc");
    o.caret = 2; // on 'b'
    let step = o.step("o", false, false, false).unwrap();
    assert!(matches!(o.mode, VimMode::Insert));
    assert_eq!(step.caret, 4);
    assert_eq!(o.text, "a\nb\n\nc");

    let mut big = buf("a\nb\nc");
    big.caret = 2;
    let step = big.step("O", false, false, false).unwrap();
    assert_eq!(big.text, "a\n\nb\nc");
    assert_eq!(step.caret, 2);

    let mut tail = buf("a");
    tail.step("o", false, false, false).unwrap();
    assert_eq!(tail.text, "a\n");
    assert_eq!(tail.caret, 2);
}

#[test]
fn x_deletes_char_with_undo_and_redo() {
    let mut v = buf("abc");
    v.caret = 1;
    let step = v.step("x", false, false, false).unwrap();
    assert_eq!(step.text.as_deref(), Some("ac"));
    assert_eq!(step.caret, 1);
    v.step("u", false, false, false);
    assert_eq!(v.text, "abc");
    v.step("r", true, false, false); // ctrl-r redo
    assert_eq!(v.text, "ac");
}

#[test]
fn dd_deletes_current_line() {
    let mut v = buf("a\nb\nc");
    v.caret = 2;
    v.step("d", false, false, false);
    assert!(v.operator.is_some());
    let step = v.step("d", false, false, false).unwrap();
    assert_eq!(step.text.as_deref(), Some("a\nc"));
}

#[test]
fn visual_char_delete_removes_selected_range() {
    let mut v = buf("abcd ef");
    v.caret = 2;
    v.step("v", false, false, false);
    assert!(matches!(v.mode, VimMode::VisualChar));
    v.step("l", false, false, false); // extend to 'cd'
    let step = v.step("d", false, false, false).unwrap();
    assert_eq!(step.text.as_deref(), Some("ab ef"));
}

#[test]
fn visual_block_deletes_characters_per_line() {
    let mut v = buf("abc\ndef\nghi");
    v.caret = 1; // 'b'
    v.step("v", true, false, false);
    assert!(matches!(v.mode, VimMode::VisualBlock));
    v.step("j", false, false, false); // anchor the second line
    let step = v.step("d", false, false, false).unwrap();
    assert_eq!(step.text.as_deref(), Some("ac\ndf\nghi"));
}

#[test]
fn yank_motion_then_paste_repeats_word() {
    let mut v = buf("one two");
    v.caret = 0;
    v.step("y", false, false, false);
    v.step("w", false, false, false);
    assert_eq!(v.register, "one "); // range runs to the next word start
    v.step("p", false, false, false);
    assert_eq!(v.text, "oone ne two");
}

#[test]
fn replace_pending_r_swaps_one_char() {
    let mut v = buf("abc");
    v.caret = 1;
    v.step("r", false, false, false);
    let step = v.step("z", false, false, false).unwrap();
    assert_eq!(step.text.as_deref(), Some("azc"));
    assert_eq!(v.register, "b");
}

#[test]
fn join_lines_replaces_newline_with_space() {
    let mut v = buf("select *\nfrom t");
    v.caret = 0;
    let step = v.step("J", false, false, false).unwrap();
    assert_eq!(step.text.as_deref(), Some("select * from t"));
}

#[test]
fn smart_indent_steps_only_on_trigger_lines() {
    // 触发词：换行带一级缩进，光标落在缩进之后，且仍在插入模式。
    let mut v = buf("BEGIN");
    v.caret = 5;
    v.mode = VimMode::Insert;
    let step = v.step("enter", false, false, false).unwrap();
    assert_eq!(step.text.as_deref(), Some("BEGIN\n  "));
    assert_eq!(v.caret, 8);
    assert!(step.to_insert);
    assert!(matches!(v.mode, VimMode::Insert));

    // 非触发词不接管，交回原生编辑器插裸换行。
    let mut v = buf("SELECT * FROM t");
    v.caret = 15;
    v.mode = VimMode::Insert;
    assert!(v.step("enter", false, false, false).is_none());

    // 整词匹配；注释与字符串字面量开头的行不算语句。
    for text in ["FORM", "THENISH", "-- BEGIN", "/* BEGIN", "'BEGIN'"] {
        let mut v = buf(text);
        v.caret = text.len();
        v.mode = VimMode::Insert;
        assert!(v.step("enter", false, false, false).is_none(), "{text}");
    }

    // 中间行命中也缩进，其余行原样保留。
    let mut v = buf("select 1\nwhen x = 1 then\nselect 2");
    v.caret = 24; // end of the `when` line
    v.mode = VimMode::Insert;
    let step = v.step("enter", false, false, false).unwrap();
    assert_eq!(
        step.text.as_deref(),
        Some("select 1\nwhen x = 1 then\n  \nselect 2")
    );
}

#[test]
fn smart_indent_undo_restores_the_buffer() {
    let mut v = buf("LOOP");
    v.caret = 4;
    v.mode = VimMode::Insert;
    let step = v.step("enter", false, false, false).unwrap();
    assert_eq!(step.text.as_deref(), Some("LOOP\n  "));
    v.step("escape", false, false, false).unwrap();
    assert!(matches!(v.mode, VimMode::Normal));
    let undone = v.step("u", false, false, false).unwrap();
    assert_eq!(undone.text.as_deref(), Some("LOOP"));
}
