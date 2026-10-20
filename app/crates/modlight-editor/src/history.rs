//! `Core/History`: undo and redo with mementos, exactly as `EditorHistory` keeps them.
//!
//! - An annotations memento holds deep copies of the annotations and the selected id.
//! - A canvas memento (before crop, cut-out, image operations) also holds the image.
//! - At most 50 mementos are kept, and at most 5 of them with an image; the limits apply
//!   from the newest down, and everything older than the first one dropped goes with it.
//! - A new memento clears the redo stack. Undo of an annotations memento saves the
//!   current annotations for redo; undo of a canvas memento saves the whole state.

use std::sync::Arc;

use image::RgbaImage;

use crate::model::{Annotation, AnnotationId};

pub const MAX_CANVAS_MEMENTOS: usize = 5;
pub const MAX_ANNOTATION_MEMENTOS: usize = 50;

/// `EditorMemento`.
#[derive(Clone, Debug)]
pub struct Memento {
    pub annotations: Vec<Annotation>,
    pub canvas_size: (f32, f32),
    pub canvas: Option<Arc<RgbaImage>>,
    pub selected: Option<AnnotationId>,
}

#[derive(Default, Debug)]
pub struct History {
    undo: Vec<Memento>,
    redo: Vec<Memento>,
}

impl History {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// `AddMemento`: push, apply the limits newest first, clear redo.
    pub fn add(&mut self, memento: Memento) {
        self.undo.push(memento);
        let mut kept = 0usize;
        let mut kept_canvas = 0usize;
        // Walk from the newest (end) to the oldest (start).
        let mut cut_at: Option<usize> = None;
        for index in (0..self.undo.len()).rev() {
            let m = &self.undo[index];
            let keep = if kept < MAX_ANNOTATION_MEMENTOS {
                if m.canvas.is_some() {
                    if kept_canvas < MAX_CANVAS_MEMENTOS {
                        kept_canvas += 1;
                        true
                    } else {
                        false
                    }
                } else {
                    true
                }
            } else {
                false
            };
            if keep {
                kept += 1;
            } else {
                cut_at = Some(index);
                break;
            }
        }
        if let Some(index) = cut_at {
            // Discard this one and everything older (history is linear).
            self.undo.drain(..=index);
        }
        self.redo.clear();
    }

    pub fn pop_undo(&mut self) -> Option<Memento> {
        self.undo.pop()
    }

    pub fn pop_redo(&mut self) -> Option<Memento> {
        self.redo.pop()
    }

    pub fn push_redo(&mut self, memento: Memento) {
        self.redo.push(memento);
    }

    /// Redo pushes straight onto the undo stack, without the limits (as upstream does).
    pub fn push_undo_raw(&mut self, memento: Memento) {
        self.undo.push(memento);
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memento(canvas: bool) -> Memento {
        Memento {
            annotations: Vec::new(),
            canvas_size: (1.0, 1.0),
            canvas: canvas.then(|| Arc::new(RgbaImage::new(1, 1))),
            selected: None,
        }
    }

    #[test]
    fn keeps_fifty_annotation_mementos() {
        let mut h = History::default();
        for _ in 0..60 {
            h.add(memento(false));
        }
        assert_eq!(h.undo_len(), 50);
    }

    #[test]
    fn a_sixth_canvas_memento_drops_everything_older() {
        let mut h = History::default();
        h.add(memento(false));
        for _ in 0..5 {
            h.add(memento(true));
        }
        h.add(memento(false));
        assert_eq!(h.undo_len(), 7);
        h.add(memento(true));
        // The newest canvas memento, the annotation one, then four canvas mementos: the
        // fifth older canvas memento and the first annotation memento are discarded.
        assert_eq!(h.undo_len(), 6);
    }

    #[test]
    fn new_memento_clears_redo() {
        let mut h = History::default();
        h.push_redo(memento(false));
        assert!(h.can_redo());
        h.add(memento(false));
        assert!(!h.can_redo());
    }
}
