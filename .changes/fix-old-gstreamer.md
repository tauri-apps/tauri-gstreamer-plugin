---
"gst-plugin-tauri": patch
---

Fix playback on GStreamer older than 1.24 (Ubuntu 22.04, Debian 12 and AppImages built there), where the element failed with `Can't typefind stream`. The element is now a source element of its own instead of a bin around `filesrc`.
