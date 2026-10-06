# Tauri Asset Gstreamer Plugin

This crate provides a GStreamer plugin that enables media playback on Tauri's `asset://` URIs.

## Compilation

For basic use:

```sh
cargo build --release
```

For size optimized builds as used in Tauri's bundler, see [/.changes/config.json].

## Usage

To be able to use the compiled artifact you need to set the `GST_PLUGIN_PATH` environment variable to point to the directory containing the compiled plugin at runtime and then you can run your application. For example:

```sh
export GST_PLUGIN_PATH="/path/to/target/release:$GST_PLUGIN_PATH"
./your-tauri-app
```

Alternatively, you can put the plugin in one of the standard GStreamer plugin directories (e.g. `/usr/lib64/gstreamer-1.0` on Linux).

## Testing

```sh
cargo test
```

Or with verbose output:

```sh
GST_DEBUG=4 cargo test
```

or only information from the plugin:

```sh
GST_DEBUG=tauri_asset:7 cargo test
```
