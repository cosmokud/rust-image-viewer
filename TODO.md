# TODO

## Features

- **Add Gallery Mode**: Functions similarly to Masonry mode but utilizes a consistent layout to bypass the "warming up" phase for faster navigation.

## QoL

- Add a custom input field for **WEBP/GIF frame rates (FPS)**.
- Scrolling and panning in Masonry mode and Long Strip mode still pause the playback, make sure they don't pause the playback anymore.

## Performance

- Optimize how metadata preloading and caching works, make sure it's only caching important informations like resolution, media type, etc, not generated thumbnails.
- Optimize metadata probing for .WEBP files.

## Known Issues

- **Fatal**: Clicking the arrow buttons in the breadcrumb address bar **crashes** the app.
- The **"Masonry Warming Up"** modal does not appear consistently when navigating folders in Masonry mode. (Note: The core functionality remains intact; only the visual progress indicator is affected.)
