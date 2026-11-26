# Tauri Asset Gstreamer Plugin

This crate provides a GStreamer plugin that enables `<audio>` and `<video>` playback from a Tauri app's custom protocols on Linux: app assets (`tauri://`), the asset protocol (`asset://`) and protocols registered by the app.

WebKitGTK hands media URLs with a custom scheme to GStreamer, which runs in WebKit's web process and cannot reach the protocol handlers of the app. The `tauriasset` source element forwards the URL to the app over a Unix socket and reads the media from there, so the app decides what is served (for example, the asset protocol scope applies).

The Tauri runtime sets this up through the environment of the web process:

- `TAURI_MEDIA_SOCKET`: path of the Unix socket the app listens on.
- `TAURI_MEDIA_PROTOCOLS`: comma separated URI schemes the element is registered for (default: `asset,tauri`).
- `WEBKIT_GST_ALLOWED_URI_PROTOCOLS`: must list the same schemes, or WebKitGTK rejects them before GStreamer is asked.

The socket protocol is documented on the element in [src/tauri_asset/imp.rs](src/tauri_asset/imp.rs).

## Compilation

For basic use:

```sh
cargo build --release
```

For size optimized builds as used in Tauri's bundler, see [/.changes/config.json].

## Usage

The Tauri bundler ships the plugin and the runtime points GStreamer at it. To use a build of your own you need to set the `GST_PLUGIN_PATH` environment variable to point to the directory containing the compiled plugin at runtime and then you can run your application. For example:

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
