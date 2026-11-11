//! Suspending manga runtime for solo fullscreen and back.

use crate::app::*;
#[allow(unused_imports)]
use crate::prelude::*;

impl ImageViewer {
    pub(crate) fn latest_video_position_from_player(player: &mut VideoPlayer) -> Option<Duration> {
        player
            .get_frame()
            .and_then(|frame| frame.pts)
            .or_else(|| player.displayed_position())
            .or_else(|| player.position())
    }
}
