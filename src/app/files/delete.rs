//! Delete confirmation execution.

use crate::app::*;

impl ImageViewer {
    pub(crate) fn mark_selection_preview_contains(&self, index: usize) -> bool {
        self.mark_selection_box
            .as_ref()
            .is_some_and(|selection| selection.preview_indices.contains(&index))
    }
}
