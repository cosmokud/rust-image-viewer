//! The right-click file context menu.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn modal_thumbnail_target_side(&self) -> u32 {
        LOD_SIDE_BUCKETS
            .iter()
            .copied()
            .find(|&side| side >= 192)
            .unwrap_or(192)
    }
}
