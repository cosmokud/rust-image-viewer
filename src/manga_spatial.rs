//! Spatial indexing helpers for manga/masonry viewport virtualization.

use rstar::{RTree, RTreeObject, AABB};

/// Wide X range used when querying strip-like layouts where visibility is Y-driven.
pub const STRIP_QUERY_HALF_WIDTH: f32 = 1_000_000_000.0;

#[derive(Clone, Copy, Debug)]
pub struct SpatialRect {
    pub index: usize,
    min: [f32; 2],
    max: [f32; 2],
}

impl SpatialRect {
    pub fn new(index: usize, min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Self {
        Self {
            index,
            min: [min_x.min(max_x), min_y.min(max_y)],
            max: [min_x.max(max_x), min_y.max(max_y)],
        }
    }
}

impl RTreeObject for SpatialRect {
    type Envelope = AABB<[f32; 2]>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(self.min, self.max)
    }
}

#[derive(Default)]
pub struct MangaSpatialIndex {
    tree: RTree<SpatialRect>,
    len: usize,
}

impl MangaSpatialIndex {
    pub fn from_rects(rects: Vec<SpatialRect>) -> Self {
        let len = rects.len();
        Self {
            tree: RTree::bulk_load(rects),
            len,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn query_indices(&self, min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Vec<usize> {
        let mut indices = Vec::new();
        self.query_indices_into(min_x, min_y, max_x, max_y, &mut indices);
        indices
    }

    pub fn query_indices_into(
        &self,
        min_x: f32,
        min_y: f32,
        max_x: f32,
        max_y: f32,
        out: &mut Vec<usize>,
    ) {
        out.clear();
        if self.is_empty() {
            return;
        }

        let query_min_x = min_x.min(max_x);
        let query_max_x = min_x.max(max_x);
        let query_min_y = min_y.min(max_y);
        let query_max_y = min_y.max(max_y);

        let envelope = AABB::from_corners([query_min_x, query_min_y], [query_max_x, query_max_y]);

        for entry in self.tree.locate_in_envelope_intersecting(&envelope) {
            // Keep strict overlap semantics to match the linear viewport checks used in
            // the pre-existing rendering and preload code paths.
            let overlaps_x = entry.min[0] < query_max_x && entry.max[0] > query_min_x;
            let overlaps_y = entry.min[1] < query_max_y && entry.max[1] > query_min_y;

            if overlaps_x && overlaps_y {
                out.push(entry.index);
            }
        }

        // Keep output deterministic so preload/draw behavior remains stable.
        out.sort_unstable();
        out.dedup();
    }

    pub fn query_vertical_band(&self, min_y: f32, max_y: f32) -> Vec<usize> {
        let mut indices = Vec::new();
        self.query_vertical_band_into(min_y, max_y, &mut indices);
        indices
    }

    pub fn query_vertical_band_into(&self, min_y: f32, max_y: f32, out: &mut Vec<usize>) {
        self.query_indices_into(
            -STRIP_QUERY_HALF_WIDTH,
            min_y,
            STRIP_QUERY_HALF_WIDTH,
            max_y,
            out,
        );
    }
}
