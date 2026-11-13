# Troubleshooting

## Video playback

1. `Failed to create video pipeline` — GStreamer playback plugins were not found. Install the runtime and restart the app.
2. Unstable decode — try `disable_hardware_decode = true`.
3. Hardware decode not being selected — keep `prefer_hardware_decode = true`, enable `enable_d3d12` / `enable_cuda`, and verify GPU drivers.
4. Decode shows unavailable — check GStreamer runtime and GPU drivers, then restart.
5. Stale `PATH` — restart the app after installing GStreamer so the refreshed environment and plugin registry are picked up.
6. Hardware decoder hangs — the app demotes the offending tier automatically and retries with the next one down to software. Restart to reset demotion.

## Config file

1. Delete `config.ini` to regenerate the latest defaults.
2. Legacy config file names (`rust-image-viewer-config.ini`, `setting.ini`) are migrated automatically.
3. Set a shortcut value to empty to disable its default binding.

## Cache

1. If metadata seems stale, delete `%LOCALAPPDATA%\rust-image-viewer\metadata_cache.redb`. The app also resets the cache automatically on schema changes or corruption.
2. To cap disk usage, lower `metadata_cache_max_size_mb`.

## Crash logs

Panic reports are written to `%TEMP%\rust-image-viewer\panic.log`.

## Build issues

1. `pkg-config` errors — `PKG_CONFIG_PATH` is not pointing at GStreamer's `pkgconfig` directory.
2. Linker errors — the GStreamer development package is missing.
