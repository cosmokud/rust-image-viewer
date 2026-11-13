# Diagnostics Overlay

Enable with `show_fps = true` in `config.ini`. The overlay appears in the top-right corner.

| Label         | Meaning                                               |
| ------------- | ----------------------------------------------------- |
| `FPS`         | Smoothed render FPS and last active-frame time.       |
| `TTV p50/p95` | Time-to-visible for multi-item media.                 |
| `U`           | Current upload batch limit.                           |
| `L`           | Pending load requests (plus peak).                    |
| `D`           | Pending decoded images (plus peak).                   |
| `V`           | Visible item count (plus peak).                       |
| `IDX H/M`     | Directory-index cache hits / misses.                  |
| `DC H/M`      | Decoded-image cache hits / misses.                    |
| `MC ...`      | Metadata-cache hits, misses, expirations, evictions.  |
| `UP p95`      | Upload pass p95 latency.                              |
| `QW p95`      | Decode queue-wait p95.                                |
| `DEC p95`     | Decode worker p95.                                    |
| `RSZ p95`     | Resize p95.                                           |
| `UTX p95`     | Texture-upload p95.                                   |
| `LY p95`      | Masonry layout rebuild p95.                           |
| `SI p95`      | Spatial-index rebuild p95.                            |
| `VQ p95`      | Visible-query p95.                                    |
| `VQ R/L`      | R-tree vs linear visible-query counts.                |
| `RR`          | Retry requests enqueued / rejected.                   |
| `TS L/M/H`    | Low / mid / high target-side distribution.            |
