use crate::MasonryItemLayout;

#[derive(Clone, Copy, Debug)]
pub(crate) struct StaticMasonryLayoutConfig {
    pub(crate) viewport_width: f32,
    pub(crate) items_per_row: usize,
    pub(crate) side_padding: f32,
    pub(crate) top_padding: f32,
    pub(crate) bottom_padding: f32,
    pub(crate) gutter: f32,
    pub(crate) min_item_height: f32,
}

pub(crate) struct StaticMasonryLayout {
    pub(crate) items: Vec<MasonryItemLayout>,
    pub(crate) total_height: f32,
}

pub(crate) fn compute_static_masonry_layout(
    item_count: usize,
    config: StaticMasonryLayoutConfig,
    mut aspect_ratio_for_index: impl FnMut(usize) -> f32,
) -> StaticMasonryLayout {
    if item_count == 0 {
        return StaticMasonryLayout {
            items: Vec::new(),
            total_height: 0.0,
        };
    }

    let columns = config.items_per_row.clamp(2, 10).max(1);
    let available_width = (config.viewport_width - config.side_padding * 2.0).max(20.0);
    let total_gutter = config.gutter * (columns.saturating_sub(1) as f32);
    let column_width = ((available_width - total_gutter) / columns as f32).max(1.0);
    let used_width = column_width * columns as f32 + total_gutter;
    let start_x = ((config.viewport_width - used_width) * 0.5).max(0.0);

    let mut column_heights = vec![config.top_padding; columns];
    let mut items = vec![MasonryItemLayout::default(); item_count];

    for idx in 0..item_count {
        let mut target_col = 0usize;
        let mut min_height = column_heights[0];
        for col in 1..columns {
            if column_heights[col] < min_height {
                min_height = column_heights[col];
                target_col = col;
            }
        }

        let x = start_x + target_col as f32 * (column_width + config.gutter);
        let y = column_heights[target_col];
        let height = (column_width * aspect_ratio_for_index(idx)).max(config.min_item_height);

        items[idx] = MasonryItemLayout {
            x,
            y,
            width: column_width,
            height,
        };

        column_heights[target_col] = y + height + config.gutter;
    }

    let mut content_bottom = config.top_padding;
    for h in column_heights {
        if h > content_bottom {
            content_bottom = h;
        }
    }

    if item_count > 0 {
        content_bottom = (content_bottom - config.gutter).max(config.top_padding);
    }

    StaticMasonryLayout {
        items,
        total_height: (content_bottom + config.bottom_padding).max(0.0),
    }
}
