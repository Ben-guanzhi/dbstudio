use super::super::guard::{PendingWrite, WriteKind};
use super::EditLedger;

fn pending(sql: &str, inverse: &str, label: &str) -> PendingWrite {
    PendingWrite {
        sql: sql.to_string(),
        kind: WriteKind::Update,
        label: label.to_string(),
        inverse_sql: inverse.to_string(),
        diff: None,
    }
}

#[test]
fn stage_and_apply_moves_edit_to_undo() {
    let mut l = EditLedger::new();
    l.stage(pending("SET a=1", "SET a=0", "UPDATE a"));
    assert!(l.has_pending());
    l.apply_success("SET a=1");
    assert!(!l.has_pending());
    assert!(l.has_undo());
    assert!(!l.has_redo());
    assert_eq!(l.undo_top().unwrap().label, "UPDATE a");
}

#[test]
fn apply_without_inverse_is_not_undoable() {
    let mut l = EditLedger::new();
    l.stage(pending("x", "y", "undoable"));
    l.apply_success("x");
    let undone = l.undo_top().unwrap();
    l.undo_succeeded(&undone);
    assert!(l.has_redo());

    l.stage(pending("INSERT ...", "", "INSERT id"));
    l.apply_success("INSERT ...");
    assert!(!l.has_pending());
    assert_eq!(l.undo_count(), 0);
    assert!(l.has_redo());
}

#[test]
fn new_apply_clears_redo() {
    let mut l = EditLedger::new();
    l.stage(pending("x", "y", "one"));
    l.apply_success("x");
    let undone = l.undo_top().unwrap();
    l.undo_succeeded(&undone);
    assert!(l.has_redo());

    l.stage(pending("p", "q", "two"));
    l.apply_success("p");
    assert!(!l.has_redo());
    assert_eq!(l.undo_count(), 1);
}

#[test]
fn undo_then_redo_round_trips() {
    let mut l = EditLedger::new();
    l.stage(pending("fwd", "inv", "edit"));
    l.apply_success("fwd");
    let r = l.undo_top().unwrap();
    l.undo_succeeded(&r);
    assert!(l.has_redo() && !l.has_undo());

    let back = l.redo_top().unwrap();
    assert_eq!(back.label, "edit");
    l.redo_succeeded(&back);
    assert!(l.has_undo() && !l.has_redo());
}

#[test]
fn failed_undo_restores_undo_side() {
    let mut l = EditLedger::new();
    l.stage(pending("fwd", "inv", "edit"));
    l.apply_success("fwd");
    let r = l.undo_top().unwrap();
    l.undo_failed(r);
    assert!(l.has_undo() && !l.has_redo());
    assert_eq!(l.undo_top().unwrap().label, "edit");
}

#[test]
fn failed_redo_restores_redo_side() {
    let mut l = EditLedger::new();
    l.stage(pending("fwd", "inv", "edit"));
    l.apply_success("fwd");
    let undone = l.undo_top().unwrap();
    l.undo_succeeded(&undone);
    let r = l.redo_top().unwrap();
    l.redo_failed(r);
    assert!(l.has_redo() && !l.has_undo());
}

#[test]
fn discard_clears_pending_only() {
    let mut l = EditLedger::new();
    l.stage(pending("a", "b", "keep"));
    l.apply_success("a");
    l.stage(pending("c", "d", "pending"));
    l.discard_pending();
    assert!(!l.has_pending());
    assert!(l.has_undo());
}

#[test]
fn pending_is_fifo_applied_in_order() {
    let mut l = EditLedger::new();
    l.stage(pending("first", "i1", "one"));
    l.stage(pending("second", "i2", "two"));
    assert_eq!(l.pending_at(0).unwrap().sql, "first");
    assert_eq!(l.pending_at(1).unwrap().sql, "second");
}
