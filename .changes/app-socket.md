---
"gst-plugin-tauri": minor:feat
---

The element now reads media from the Tauri app over a Unix socket (`TAURI_MEDIA_SOCKET`) instead of opening the file itself. This adds support for app assets (`tauri://`) and protocols registered by the app (listed in `TAURI_MEDIA_PROTOCOLS`), and makes `asset://` go through the app's asset protocol handler, so its scope applies to media playback. Requires a Tauri version that serves the socket.
