//! Title-bar text/button layout maths.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

/// One segment of the title bar's file metadata: resolution, file size, zoom, video badge,
/// position in the list.
pub(crate) struct TitleBarDetail {
    pub(crate) text: String,
    pub(crate) color: egui::Color32,
    /// Higher survives longer when the bar is too narrow for all of them.
    pub(crate) priority: u8,
}

/// Space the title bar wants to keep for the file name before dropping it for good.
pub(crate) const TITLE_BAR_MIN_FILENAME_WIDTH: f32 = 60.0;

/// Gap between the file name and the metadata that follows it.
pub(crate) const TITLE_BAR_FILENAME_DETAILS_GAP: f32 = 8.0;

/// Slack between the measured text widths and the width the laid-out labels really take.
pub(crate) const TITLE_BAR_WIDTH_SLACK: f32 = 6.0;

/// How the left stretch of the title bar splits between the file name and the metadata.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TitleBarLeftLayout {
    /// Width the file name may take; `None` when it does not fit at all.
    pub(crate) filename_width: Option<f32>,
    /// Metadata segments to draw, as ascending indices into the caller's list.
    pub(crate) details: Vec<usize>,
    /// Width the drawn metadata segments take, spacing between them included.
    pub(crate) details_width: f32,
}

/// Splits the title bar's left stretch: metadata first, file name with whatever is left.
///
/// The window buttons own a separate strip outside this budget, so they never lose.
/// Nothing drops while the bar can still hold all the metadata plus
/// `TITLE_BAR_MIN_FILENAME_WIDTH`: the file name shrinks with an ellipsis instead. Only once it
/// cannot keep that much does the name disappear and, if the metadata still does not fit, the
/// lowest `priority` segments after it.
pub(crate) fn title_bar_left_layout(
    available_width: f32,
    has_filename: bool,
    detail_widths: &[f32],
    detail_priorities: &[u8],
    spacing: f32,
) -> TitleBarLeftLayout {
    let available = (available_width - TITLE_BAR_WIDTH_SLACK).max(0.0);

    // Highest priority first, taking each segment only if it still fits: a wide low-priority
    // segment must not push a narrow high-priority one out of the bar.
    let select = |budget: f32| -> (Vec<usize>, f32) {
        let mut order: Vec<usize> = (0..detail_widths.len()).collect();
        order.sort_by(|left, right| {
            detail_priorities[*right]
                .cmp(&detail_priorities[*left])
                .then(left.cmp(right))
        });

        let mut selected: Vec<usize> = Vec::new();
        let mut used = 0.0;
        for index in order {
            let extra = detail_widths[index] + if selected.is_empty() { 0.0 } else { spacing };
            if used + extra <= budget {
                used += extra;
                selected.push(index);
            }
        }

        selected.sort_unstable();
        (selected, used)
    };

    if detail_widths.is_empty() {
        return TitleBarLeftLayout {
            filename_width: has_filename.then_some(available),
            details: Vec::new(),
            details_width: 0.0,
        };
    }

    let gap = TITLE_BAR_FILENAME_DETAILS_GAP + spacing;
    let all_details_width =
        detail_widths.iter().sum::<f32>() + (spacing * (detail_widths.len() - 1) as f32);

    // The whole metadata strip still sits next to a readable name: nothing has to give.
    if has_filename && all_details_width + gap + TITLE_BAR_MIN_FILENAME_WIDTH <= available {
        return TitleBarLeftLayout {
            filename_width: Some(available - all_details_width - gap),
            details: (0..detail_widths.len()).collect(),
            details_width: all_details_width,
        };
    }

    // The name goes first and the metadata takes the whole stretch; only what still does not
    // fit after that drops, lowest priority first.
    let (details, details_width) = select(available);
    TitleBarLeftLayout {
        filename_width: None,
        details,
        details_width,
    }
}
