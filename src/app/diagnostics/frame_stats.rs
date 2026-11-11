//! Per-frame diag trace fields.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

/// Frame line fields for the diag trace; only built when diag is enabled.
pub(crate) fn diag_frame_stats(
    viewer: &ImageViewer,
    frame_started: Instant,
    repaint: &'static str,
    backlog: (usize, usize, usize, usize),
) -> crate::diag::FrameStats {
    let (inflight, retry) = viewer
        .manga_loader
        .as_ref()
        .map(crate::manga_loader::MangaLoader::diag_counts)
        .unwrap_or((0, 0));

    crate::diag::FrameStats {
        total: frame_started.elapsed(),
        repaint,
        visible: viewer.manga_visible_indices_last,
        uploads: viewer.manga_uploads_last_pass,
        upload_ms: viewer.manga_upload_ms_last_pass,
        upload_items: viewer.manga_upload_items_last_pass,
        backlog,
        inflight,
        retry,
        ttv_pending: viewer.manga_ttv_pending.len(),
    }
}
