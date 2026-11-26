# Changelog

## [0.0.2]

- [`759280f`](https://www.github.com/tauri-apps/tauri-gstreamer-plugin/commit/759280fb3e08573c60099a9ec9a6694180bff1ea) ([#3](https://www.github.com/tauri-apps/tauri-gstreamer-plugin/pull/3) by [@lucasfernog](https://www.github.com/tauri-apps/tauri-gstreamer-plugin/../../lucasfernog)) Fix playback on GStreamer older than 1.24 (Ubuntu 22.04, Debian 12 and AppImages built there), where the element failed with `Can't typefind stream`. The element is now a source element of its own instead of a bin around `filesrc`.

## [0.0.1]

- [`2577af3`](https://www.github.com/tauri-apps/tauri-gstreamer-plugin/commit/2577af3c3f36acf2a08b682f340bc3bf901fd8c1) Initial release.
