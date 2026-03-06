//! Spatial indexing helpers for manga/masonry viewport virtualization.

use rstar::{AABB, RTree, RTreeObject};

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
        if self.is_empty() {
            return Vec::new();
        }

        let query_min_x = min_x.min(max_x);
        let query_max_x = min_x.max(max_x);
        let query_min_y = min_y.min(max_y);
        let query_max_y = min_y.max(max_y);

        let envelope = AABB::from_corners(
            [query_min_x, query_min_y],
            [query_max_x, query_max_y],
        );

        let mut indices: Vec<usize> = self
            .tree
            .locate_in_envelope_intersecting(&envelope)
            .filter_map(|entry| {
                // Keep strict overlap semantics to match the linear viewport checks used in
                // the pre-existing rendering and preload code paths.
                let overlaps_x = entry.min[0] < query_max_x && entry.max[0] > query_min_x;
                let overlaps_y = entry.min[1] < query_max_y && entry.max[1] > query_min_y;

                if overlaps_x && overlaps_y {
                    Some(entry.index)
                } else {
                    None
                }
            })
            .collect();

        // Keep output deterministic so preload/draw behavior remains stable.
        indices.sort_unstable();
        indices.dedup();
        indices
    }

    pub fn query_vertical_band(&self, min_y: f32, max_y: f32) -> Vec<usize> {
        self.query_indices(
            -STRIP_QUERY_HALF_WIDTH,
            min_y,
            STRIP_QUERY_HALF_WIDTH,
            max_y,
        )
    }
}

#[cfg(test)]
#[allow(dead_code, unused_imports)]
mod tests {
    use super::{MangaSpatialIndex, SpatialRect, STRIP_QUERY_HALF_WIDTH};

    fn build_strip_bounds(count: usize) -> Vec<(f32, f32)> {
        let mut bounds = Vec::with_capacity(count);
        let mut y = 0.0;

        for idx in 0..count {
            let h = 96.0 + ((idx.saturating_mul(37) % 280) as f32);
            let start = y;
            y += h;
            bounds.push((start, y));
        }

        bounds
    }

    fn strip_rects(bounds: &[(f32, f32)]) -> Vec<SpatialRect> {
        bounds
            .iter()
            .enumerate()
            .map(|(idx, (start, end))| {
                SpatialRect::new(
                    idx,
                    -STRIP_QUERY_HALF_WIDTH,
                    *start,
                    STRIP_QUERY_HALF_WIDTH,
                    *end,
                )
            })
            .collect()
    }

    fn linear_vertical_band(bounds: &[(f32, f32)], min_y: f32, max_y: f32) -> Vec<usize> {
        let (min_y, max_y) = if min_y <= max_y {
            (min_y, max_y)
        } else {
            (max_y, min_y)
        };

        bounds
            .iter()
            .enumerate()
            .filter_map(|(idx, (start, end))| {
                if *start < max_y && *end > min_y {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    fn build_masonry_bounds(count: usize, columns: usize) -> Vec<(f32, f32, f32, f32)> {
        let columns = columns.max(1);
        let mut column_heights = vec![8.0f32; columns];
        let mut bounds = Vec::with_capacity(count);

        let col_width = 220.0f32;
        let gutter = 12.0f32;

        for idx in 0..count {
            let col = idx % columns;
            let x = col as f32 * (col_width + gutter);
            let y = column_heights[col];
            let h = 80.0 + ((idx.saturating_mul(53) % 260) as f32);
            let max_x = x + col_width;
            let max_y = y + h;

            bounds.push((x, y, max_x, max_y));
            column_heights[col] = max_y + gutter;
        }

        bounds
    }

    fn masonry_rects(bounds: &[(f32, f32, f32, f32)]) -> Vec<SpatialRect> {
        bounds
            .iter()
            .enumerate()
            .map(|(idx, (min_x, min_y, max_x, max_y))| {
                SpatialRect::new(idx, *min_x, *min_y, *max_x, *max_y)
            })
            .collect()
    }

    fn linear_query_indices(
        bounds: &[(f32, f32, f32, f32)],
        min_x: f32,
        min_y: f32,
        max_x: f32,
        max_y: f32,
    ) -> Vec<usize> {
        let (min_x, max_x) = if min_x <= max_x {
            (min_x, max_x)
        } else {
            (max_x, min_x)
        };
        let (min_y, max_y) = if min_y <= max_y {
            (min_y, max_y)
        } else {
            (max_y, min_y)
        };

        bounds
            .iter()
            .enumerate()
            .filter_map(|(idx, (item_min_x, item_min_y, item_max_x, item_max_y))| {
                let intersects_x = *item_min_x < max_x && *item_max_x > min_x;
                let intersects_y = *item_min_y < max_y && *item_max_y > min_y;

                if intersects_x && intersects_y {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn strip_visibility_matches_linear_scan() {
        let bounds = build_strip_bounds(4096);
        let total_height = bounds.last().map(|(_, end)| *end).unwrap_or(0.0);
        let index = MangaSpatialIndex::from_rects(strip_rects(&bounds));

        for viewport_h in [180.0f32, 720.0, 1600.0] {
            let mut y = 0.0f32;
            while y <= total_height {
                let expected = linear_vertical_band(&bounds, y, y + viewport_h);
                let actual = index.query_vertical_band(y, y + viewport_h);
                assert_eq!(actual, expected, "mismatch at y={y}, h={viewport_h}");
                y += 257.0;
            }
        }
    }

    #[test]
    fn masonry_vertical_band_matches_linear_scan() {
        let bounds = build_masonry_bounds(6000, 8);
        let index = MangaSpatialIndex::from_rects(masonry_rects(&bounds));

        let max_y = bounds
            .iter()
            .fold(0.0f32, |acc, (_, _, _, item_max_y)| acc.max(*item_max_y));

        for viewport_h in [320.0f32, 900.0, 1800.0] {
            let mut y = 0.0f32;
            while y <= max_y {
                let expected: Vec<usize> = bounds
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, (_, item_min_y, _, item_max_y))| {
                        if *item_min_y < y + viewport_h && *item_max_y > y {
                            Some(idx)
                        } else {
                            None
                        }
                    })
                    .collect();

                let actual = index.query_vertical_band(y, y + viewport_h);
                assert_eq!(actual, expected, "mismatch at y={y}, h={viewport_h}");
                y += 411.0;
            }
        }
    }

    #[test]
    fn masonry_point_query_matches_linear_scan() {
        let bounds = build_masonry_bounds(5000, 7);
        let index = MangaSpatialIndex::from_rects(masonry_rects(&bounds));

        let eps = 0.0001f32;
        for i in 0usize..250 {
            let x = (i.saturating_mul(97) % 2200) as f32;
            let y = (i.saturating_mul(131) % 120000) as f32;

            let expected = linear_query_indices(&bounds, x - eps, y - eps, x + eps, y + eps);
            let actual = index.query_indices(x - eps, y - eps, x + eps, y + eps);

            assert_eq!(actual, expected, "point mismatch at ({x}, {y})");
        }
    }
}
