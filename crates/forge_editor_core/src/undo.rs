//! Editor-side undo/redo history.
//!
//! Because the ECS world lives in the runtime process, every undoable edit is
//! expressed as *forward and inverse [`EditorToRuntime`] command batches*. The
//! editor captures both sides at edit time (it always knows the previous value
//! from its mirrored inspector state), pushes the pair onto the stack, and can
//! replay either side. Multi-command entries (e.g. `RemoveComponent` undo =
//! add-back + N field restores) are simply longer batches.

use forge_ipc::{EditorToRuntime, EntityId};

/// One undo unit: inverse commands to roll back, forward commands to redo.
#[derive(Debug, Clone)]
pub struct UndoEntry {
    /// Human label shown in menus ("Translate Player", "Delete Crate").
    pub label: String,
    /// Commands that revert the change.
    pub undo: Vec<EditorToRuntime>,
    /// Commands that reapply the change.
    pub redo: Vec<EditorToRuntime>,
    /// Entity created by a spawn (tracked so redo-of-undo deletes the copy).
    pub spawned_entity: Option<EntityId>,
    /// Trash token returned by the runtime for delete operations.
    pub trash_id: Option<u64>,
}

#[derive(Default)]
pub struct UndoStack {
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    limit: usize,
}

impl UndoStack {
    pub fn new(limit: usize) -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), limit: limit.max(1) }
    }

    /// Record a completed edit (clears the redo branch, like every editor).
    pub fn push(&mut self, entry: UndoEntry) {
        self.redo.clear();
        self.undo.push(entry);
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    pub fn pop_undo(&mut self) -> Option<UndoEntry> {
        let entry = self.undo.pop()?;
        self.redo.push(entry.clone());
        Some(entry)
    }

    pub fn pop_redo(&mut self) -> Option<UndoEntry> {
        let entry = self.redo.pop()?;
        self.undo.push(entry.clone());
        Some(entry)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn top_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn len(&self) -> usize {
        self.undo.len()
    }

    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_ipc::{ComponentField, ComponentKind, FieldValue};

    #[test]
    fn undo_redo_flow() {
        let mut stack = UndoStack::new(10);
        let entry = UndoEntry {
            label: "Move Cube".into(),
            undo: vec![EditorToRuntime::SetField {
                entity: 1,
                component: ComponentKind::Transform,
                field: ComponentField::Translation,
                value: FieldValue::Vec3([0.0; 3]),
            }],
            redo: vec![EditorToRuntime::SetField {
                entity: 1,
                component: ComponentKind::Transform,
                field: ComponentField::Translation,
                value: FieldValue::Vec3([5.0, 0.0, 0.0]),
            }],
            spawned_entity: None,
            trash_id: None,
        };
        stack.push(entry);
        assert!(stack.can_undo());
        let undone = stack.pop_undo().unwrap();
        assert!(matches!(undone.undo[0], EditorToRuntime::SetField { .. }));
        assert!(stack.can_redo());
        let redone = stack.pop_redo().unwrap();
        assert!(matches!(redone.redo[0], EditorToRuntime::SetField { .. }));
        assert!(stack.can_undo());
    }
}
