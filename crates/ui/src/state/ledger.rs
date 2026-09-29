use super::guard::{EditRecord, PendingWrite};

/// Lifecycle phase of a single edit within a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditPhase {
    /// Staged for review; not yet applied to the database.
    Pending,
    /// Applied to the database; available on the undo side.
    Applied,
    /// Reverted by an undo; available on the redo side.
    Undone,
}

/// The single ledger that tracks a session's data edits across their whole
/// lifecycle: staged (pending), applied (undoable), and reverted (redoable).
///
/// It replaces the three parallel `Vec`s that used to sit on
/// `ConnectionSession` (`pending_edits`, `undo_stack`, `redo_stack`). Those
/// three lists had to stay mutually consistent by hand at every call site
/// (a new apply clears redo, a failed undo puts the record back, ...). Keeping
/// them behind one owner whose transition methods are the *only* mutation
/// points makes those invariants live in one place and unit-testable without
/// touching the GPUI runtime.
///
/// Ordering is the same as before: pending edits are a FIFO queue (applied in
/// stage order); the undo and redo sides are LIFO stacks (the most recent
/// applied edit is the one an undo reaches first).
#[derive(Debug, Clone, Default)]
pub struct EditLedger {
    pending: Vec<PendingWrite>,
    undo: Vec<EditRecord>,
    redo: Vec<EditRecord>,
}

impl EditLedger {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- pending side ----

    /// Stage an edit for review. It enters the pending queue, applied in the
    /// order it was staged.
    pub fn stage(&mut self, edit: PendingWrite) {
        self.pending.push(edit);
    }

    /// Staged edits, in application order.
    pub fn pending(&self) -> &[PendingWrite] {
        &self.pending
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Drop every staged edit without applying it.
    pub fn discard_pending(&mut self) {
        self.pending.clear();
    }

    /// Borrow the staged edit at `index` (for single-edit application), leaving
    /// the rest of the queue in place.
    pub fn pending_at(&self, index: usize) -> Option<&PendingWrite> {
        self.pending.get(index)
    }

    // ---- apply ----

    /// Record that the staged edit identified by `sql` just executed
    /// successfully. It leaves the pending queue and, when it carries a
    /// derivable inverse, enters the undo side — invalidating any redo
    /// history, since redo can only replay the exact sequence that was undone.
    ///
    /// Edits with an empty `inverse_sql` are not undoable (e.g. an insert
    /// without a known primary-key value): they leave the pending queue but
    /// push nothing onto the ledger's history and leave redo untouched.
    pub fn apply_success(&mut self, sql: &str) {
        let Some(i) = self.pending.iter().position(|e| e.sql == sql) else {
            return;
        };
        let edit = self.pending.remove(i);
        if edit.inverse_sql.is_empty() {
            return;
        }
        self.undo.push(EditRecord {
            forward_sql: edit.sql,
            inverse_sql: edit.inverse_sql,
            label: edit.label,
        });
        self.redo.clear();
    }

    // ---- undo side ----

    /// Pop the most recent applied edit so its inverse can run. Returns `None`
    /// when the undo side is empty.
    pub fn undo_top(&mut self) -> Option<EditRecord> {
        self.undo.pop()
    }

    /// The inverse executed: the edit now sits on the redo side.
    pub fn undo_succeeded(&mut self, record: &EditRecord) {
        self.redo.push(record.clone());
    }

    /// The inverse failed: the edit stays applied, so put it back on the undo
    /// side instead of arming a redo that would replay an edit still in force.
    pub fn undo_failed(&mut self, record: EditRecord) {
        self.undo.push(record);
    }

    // ---- redo side ----

    /// Pop the most recent undone edit so its forward SQL can re-run.
    pub fn redo_top(&mut self) -> Option<EditRecord> {
        self.redo.pop()
    }

    /// The forward re-executed: the edit is back on the undo side.
    pub fn redo_succeeded(&mut self, record: &EditRecord) {
        self.undo.push(record.clone());
    }

    /// The forward re-run failed: put the edit back on the redo side.
    pub fn redo_failed(&mut self, record: EditRecord) {
        self.redo.push(record);
    }

    // ---- queries ----

    pub fn has_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn has_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_count(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_count(&self) -> usize {
        self.redo.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(sql: &str, inverse: &str, label: &str) -> PendingWrite {
        PendingWrite {
            sql: sql.to_string(),
            kind: super::super::guard::WriteKind::Update,
            label: label.to_string(),
            inverse_sql: inverse.to_string(),
            diff: None,
        }
    }

    fn rec(sql: &str, inverse: &str, label: &str) -> EditRecord {
        EditRecord {
            forward_sql: sql.to_string(),
            inverse_sql: inverse.to_string(),
            label: label.to_string(),
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
        // Arm a redo entry first so we can prove a non-undoable apply leaves it.
        l.stage(pending("x", "y", "undoable"));
        l.apply_success("x");
        let undone = l.undo_top().unwrap();
        l.undo_succeeded(&undone);
        assert!(l.has_redo());

        l.stage(pending("INSERT ...", "", "INSERT id"));
        l.apply_success("INSERT ...");
        // Left pending, pushed nothing to undo, and did NOT clear redo.
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
        // A fresh undoable apply invalidates the redo side. ("one" was already
        // undone above, so the undo side now holds only "two".)
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
        // Inverse failed: the edit is still applied, so it returns to undo.
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
        // Forward re-run failed: the edit is still undone, back on redo.
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
        assert!(l.has_undo()); // applied history is untouched by a discard
    }

    #[test]
    fn pending_is_fifo_applied_in_order() {
        let mut l = EditLedger::new();
        l.stage(pending("first", "i1", "one"));
        l.stage(pending("second", "i2", "two"));
        assert_eq!(l.pending_at(0).unwrap().sql, "first");
        assert_eq!(l.pending_at(1).unwrap().sql, "second");
    }
}
