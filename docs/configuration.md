# Configuration

The config file is at `%APPDATA%\rust-image-viewer\config.ini` on Windows. Delete it to regenerate the defaults.

## General

| Key                                             | Default    | Meaning                                                                                            |
| ----------------------------------------------- | ---------- | -------------------------------------------------------------------------------------------------- |
| `controls_hide_delay`                           | `0.5`      | Seconds before the title bar / controls hide.                                                      |
| `bottom_overlay_hide_delay`                     | `0.5`      | Seconds before bottom overlays (video controls, mode buttons, zoom HUD) hide.                     |
| `double_click_grace_period`                     | `0.50`     | Double-click timing window in seconds, with a 12 px click radius.                                  |
| `show_fps`                                      | `false`    | Show the diagnostics overlay in the top-right corner.                                              |
| `resize_border_size`                            | `6`        | Floating-window resize hit area in pixels.                                                         |
| `startup_window_mode`                           | `floating` | `floating` or `fullscreen`.                                                                        |
| `single_instance`                               | `true`     | Reuse the existing window for new file-open requests.                                              |
| `window_title_show_full_path`                   | `auto`     | `auto` = filename in floating, full path in fullscreen. `true` = always full path. `false` = filename only. |
| `vsync`                                         | `true`     | Enable vsync to reduce tearing.                                                                    |
| `metadata_cache_max_size_mb`                    | `1024`     | Max on-disk size of the metadata cache in MiB. `0` disables the cap.                              |
| `background_rgb`                                | `0, 0, 0`  | Background color as an RGB triplet.                                                                |
| `fullscreen_mode`                               | `borderless` | `borderless` = whole display, taskbar covered. `windowed` = whole display above the taskbar, so a non-auto-hide taskbar stays visible. |
| `fullscreen_reset_fit_on_mode_switch`           | `true`     | Fit media when switching between floating and fullscreen.                                          |
| `manga_long_strip_reset_fit_on_fullscreen_exit` | `true`     | Fit long-strip view when returning from solo fullscreen.                                           |
| `fullscreen_remember_view_state_in_ram`         | `true`     | Remember zoom/pan/rotation per image for the current fullscreen session.                           |
| `fullscreen_native_window_transition`           | `true`     | Use Windows maximize/restore animations during fullscreen transitions.                             |
| `maximize_to_borderless_fullscreen`             | `true`     | Make the maximize button enter borderless fullscreen instead of a maximized floating window.       |
| `floating_mode_remember_position`               | `true`     | Keep floating window position and size cap between files.                                          |
| `confirm_delete_to_recycle_bin`                 | `true`     | Show confirmation modal before deleting to Recycle Bin.                                            |
| `auto_unmark_after_paste`                       | `true`     | Clear the marked selection after a successful paste.                                               |
| `zoom_animation_speed`                          | `20`       | Floating autosize zoom spring speed. `0` snaps instantly. Scroll zoom uses its own fixed spring.   |
| `precise_rotation_step_degrees`                 | `2.0`      | Degrees per `Ctrl+Up` / `Ctrl+Down`.                                                               |
| `zoom_step`                                     | `1.02`     | Scroll-wheel zoom multiplier. Floating applies `zoom_step ^ 13` per notch to match fullscreen.     |
| `max_zoom_percent`                              | `1000`     | Maximum zoom level as a percentage.                                                                |

## Long Strip and Masonry

| Key                                            | Default         | Meaning                                            |
| ---------------------------------------------- | --------------- | -------------------------------------------------- |
| `manga_drag_pan_speed`                         | `1.0`           | Drag-pan multiplier.                               |
| `manga_wheel_impulse_per_step`                 | `2400.0`        | Velocity per wheel step.                           |
| `manga_wheel_decay_rate`                       | `11.0`          | Wheel momentum decay.                              |
| `manga_wheel_max_velocity`                     | `9000.0`        | Wheel velocity cap.                                |
| `manga_wheel_edge_spring_hz`                   | `4.5`           | Edge overscroll return stiffness.                  |
| `manga_inertial_friction`                      | `0.33`          | Keyboard / page / autoscroll friction.             |
| `manga_arrow_scroll_speed`                     | `140`           | Base arrow-key pan distance.                       |
| `masonry_items_per_row`                        | `5`             | Number of columns in Masonry.                      |
| `manga_autoscroll_dead_zone_px`                | `14.0`          | Freehand autoscroll dead zone around the anchor.   |
| `manga_autoscroll_base_speed_multiplier`       | `5.0`           | Base autoscroll speed multiplier.                  |
| `manga_autoscroll_min_speed_multiplier`        | `0.6`           | Lower autoscroll speed bound.                      |
| `manga_autoscroll_max_speed_multiplier`        | `14.0`          | Upper autoscroll speed bound.                      |
| `manga_autoscroll_curve_power`                 | `2.0`           | Speed curve power from center to edge.             |
| `manga_autoscroll_min_speed_px_per_sec`        | `80.0`          | Absolute minimum autoscroll speed.                 |
| `manga_autoscroll_max_speed_px_per_sec`        | `14000.0`       | Absolute maximum autoscroll speed.                 |

## Video

| Key                       | Default    | Meaning                                                                                 |
| ------------------------- | ---------- | --------------------------------------------------------------------------------------- |
| `muted_by_default`        | `remember` | `true`, `false`, or `remember` (uses the last muted state).                             |
| `default_volume`          | `remember` | Initial volume (0.0–1.0) or `remember`.                                                 |
| `loop`                    | `true`     | Loop videos at end-of-stream.                                                           |
| `seek_policy`             | `adaptive` | `adaptive` (keyframe while dragging, accurate on release), `accurate`, or `keyframe`.   |
| `prefer_hardware_decode`  | `true`     | Prefer hardware decoders (D3D12 / D3D11).                                               |
| `disable_hardware_decode` | `false`    | Force software decode. Overrides `prefer_hardware_decode`.                              |
| `videos_only_navigation`  | `true`     | Skip non-video files when navigating in video-like playback mode.                       |

## Performance

| Key                           | Default | Meaning                                                   |
| ----------------------------- | ------- | --------------------------------------------------------- |
| `show_fps_update_interval_ms` | `500`   | Diagnostics overlay refresh cadence in milliseconds.      |
| `use_hardware_acceleration`   | `true`  | Master switch for hardware-accelerated paths.             |
| `enable_d3d12`                | `true`  | Prefer D3D12 decoders (falls back to D3D11).              |
| `enable_cuda`                 | `true`  | Enable CUDA decoder preference where supported.           |
| `max_decode_megapixels`       | `256`   | Largest still image the decoder opens, in decimal megapixels (`width x height / 1,000,000`); raise it for 150-200 MP panoramas, lower it to cap RAM. The Settings slider covers 8-1024; typing in the number box or editing this key accepts any larger value (e.g. `10000`) as it stands. |
| `large_decode_threshold_megapixels` | `64` | Images above this size count as large: they take the limited slots below and are the ones checked against free RAM. Slider 8-256; typed or hand-edited values are honoured as they stand. |
| `parallel_large_decodes`      | `1`     | How many images above the threshold may decode at once. 1 bounds the peak memory of a gallery sweep; raise it on a big-RAM machine. |
| `decode_memory_guard`         | `true`  | Refuse a decode that needs more than half of the memory Windows reports as available, instead of thrashing the pagefile. |

The last four form the **Decode Limits** card in Settings -> Performance. For the two megapixel
rows the slider range is a convenience: typing a number into the box next to it, or writing the
key in `config.ini`, is not limited to that range, and the value is used exactly as written.

## Quality

| Key                             | Default    | Meaning                                                                           |
| ------------------------------- | ---------- | --------------------------------------------------------------------------------- |
| `downscale_filter`              | `lanczos3` | CPU resize filter for shrinking images.                                           |
| `gif_resize_filter`             | `triangle` | CPU resize filter for GIF frames.                                                 |
| `texture_filter_static`         | `linear`   | GPU texture filtering for static images.                                          |
| `texture_filter_animated`       | `linear`   | GPU texture filtering for GIF / animated WebP.                                    |
| `texture_filter_video`          | `linear`   | GPU texture filtering for video.                                                  |
| `manga_mipmap_static`           | `true`     | Enable mipmaps for static textures in Long Strip / Masonry.                       |
| `manga_mipmap_video_thumbnails` | `true`     | Enable mipmaps for video thumbnails in Long Strip / Masonry.                      |
| `manga_mipmap_min_side`         | `128`      | Minimum texture side before mipmaps are generated.                                |

Supported values:

- Scaling filters: `nearest`, `triangle`, `catmullrom`, `gaussian`, `lanczos3`
- Texture filters: `nearest`, `linear`
- Startup window modes: `floating`, `fullscreen`
- Video seek policies: `adaptive`, `accurate`, `keyframe`
