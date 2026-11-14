# Shortcuts

Bindings are context-aware — the same key can do different things in different modes.

## Global

| Action            | Default                    |
| ----------------- | -------------------------- |
| Toggle fullscreen | `f`, `f11`, `f12`, `enter` |
| Exit              | `ctrl+w`, `escape`         |
| Undo file action  | `ctrl+z`                   |
| Redo file action  | `ctrl+y`                   |

## Floating and solo fullscreen

| Action                             | Default                           |
| ---------------------------------- | --------------------------------- |
| Pan                                | `mouse_left`                      |
| Previous / next (side zones)       | `mouse_right`                     |
| Freehand autoscroll                | `mouse_middle`                    |
| Mark hovered file                  | `space`                           |
| Toggle mark                        | `ctrl+mouse_left`                 |
| Next item                          | `right`, `pagedown`, `mouse5`     |
| Previous item                      | `left`, `pageup`, `mouse4`        |
| Rotate clockwise                   | `up`                              |
| Rotate counterclockwise            | `down`                            |
| Precise rotation clockwise         | `ctrl+up`                         |
| Precise rotation counterclockwise  | `ctrl+down`                       |
| Zoom in                            | `scroll_up`, `ctrl+scroll_up`     |
| Zoom out                           | `scroll_down`, `ctrl+scroll_down` |
| Jump to first / last               | `home` / `end`                    |

## Long Strip

| Action                               | Default                |
| ------------------------------------ | ---------------------- |
| Drag-pan                             | `mouse_left`           |
| Open item in solo fullscreen         | `mouse_right`          |
| Freehand autoscroll                  | `mouse_middle`         |
| Mark hovered item                    | `space`                |
| Toggle mark                          | `ctrl+mouse_left`      |
| Pan up / down                        | `up`, `down`           |
| Fit-aware next / previous page       | `right`, `left`        |
| Jump to next / previous item         | `pagedown` / `pageup`  |
| Scroll up / down                     | `scroll_up/down`       |
| Zoom in / out                        | `ctrl+scroll_up/down`  |
| Jump to start / end                  | `home`, `end`          |

## Masonry

| Action                               | Default                                  |
| ------------------------------------ | ---------------------------------------- |
| Drag-pan                             | `mouse_left`                             |
| Open item in solo fullscreen         | `mouse_right`                            |
| Freehand autoscroll                  | `mouse_middle`                           |
| Mark hovered item                    | `space`                                  |
| Toggle mark                          | `ctrl+mouse_left`                        |
| Pan up / down                        | `up`, `down`                             |
| Faster pan                           | `left`, `right`                          |
| Jump pan                             | `pageup`, `pagedown`, `mouse4`, `mouse5` |
| Scroll                               | `scroll_up`, `scroll_down`               |
| Zoom in / out                        | `ctrl+scroll_up`, `ctrl+scroll_down`     |

## Video

| Action       | Default |
| ------------ | ------- |
| Play / pause | `space` |
| Mute         | `m`     |

## Stash

| Action             | Default                       |
| ------------------ | ----------------------------- |
| Navigate items     | `left`, `right`, `up`, `down` |
| Open selected item | `enter`                       |
| Remove from stash  | `delete`                      |
| Select all         | `ctrl+a`                      |
| Deselect / Close   | `escape`                      |

## Custom shortcuts

Shortcuts live in `config.ini` under `[Shortcuts]`. Leave a value empty to disable the default binding.

```ini
[Shortcuts]
toggle_fullscreen = q, enter
video_play_pause = k, space
manga_goto_file = mouse_middle
masonry_goto_file = enter
```

| Type          | Values                                                                                                  |
| ------------- | ------------------------------------------------------------------------------------------------------- |
| Mouse buttons | `mouse_left`, `mouse_right`, `mouse_middle`, `mouse4`, `mouse5`                                         |
| Scroll wheel  | `scroll_up`, `scroll_down`                                                                              |
| Modifiers     | `ctrl+<key>`, `shift+<key>`, `alt+<key>`                                                                |
| Letters       | `a` – `z`                                                                                               |
| Numbers       | `0` – `9`                                                                                               |
| Function keys | `f1` – `f12`                                                                                            |
| Arrow keys    | `left`, `right`, `up`, `down`                                                                           |
| Special keys  | `escape`, `enter`, `space`, `tab`, `backspace`, `delete`, `insert`, `home`, `end`, `pageup`, `pagedown` |
