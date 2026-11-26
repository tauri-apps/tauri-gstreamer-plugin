// Copyright 2026-2026 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{LazyLock, Mutex, MutexGuard};

use gst::glib;
use gst::subclass::prelude::*;
use gst_base::prelude::*;
use gst_base::subclass::prelude::*;
use gstreamer as gst;
use gstreamer_base as gst_base;

// for debugging
// GST_DEBUG=tauri_asset:5 <your-binary-using-the-plugin>
static CAT: LazyLock<gst::DebugCategory> = LazyLock::new(|| {
    gst::DebugCategory::new(
        "tauri_asset",
        gst::DebugColorFlags::empty(),
        Some("Tauri Asset Element"),
    )
});

/// Environment variable with the path of the Unix socket the Tauri app serves media on.
const SOCKET_ENV_VAR: &str = "TAURI_MEDIA_SOCKET";
/// Environment variable with the comma separated URI schemes the app serves.
pub const PROTOCOLS_ENV_VAR: &str = "TAURI_MEDIA_PROTOCOLS";
/// Schemes handled when the app does not list any.
const DEFAULT_PROTOCOLS: &[&str] = &["asset", "tauri"];

/// The app answers this instead of a byte count when a read fails.
const READ_FAILED: u32 = u32::MAX;
/// The app answers this as the size when it does not know it.
const UNKNOWN_SIZE: u64 = u64::MAX;
/// Bytes asked for at a time in push mode, each read is a round trip to the app.
const BLOCK_SIZE: u32 = 64 * 1024;

/// The URI schemes this element is registered for.
///
/// They come from the environment because they are the protocols registered
/// in the app, which differ between apps; GStreamer asks for them once, when
/// the plugin is scanned.
fn protocols() -> &'static [&'static str] {
    static PROTOCOLS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
        let protocols = std::env::var(PROTOCOLS_ENV_VAR)
            .unwrap_or_default()
            .split(',')
            .map(|protocol| protocol.trim().to_ascii_lowercase())
            .filter(|protocol| !protocol.is_empty())
            .map(|protocol| &*protocol.leak())
            .collect::<Vec<_>>();
        if protocols.is_empty() {
            DEFAULT_PROTOCOLS.to_vec()
        } else {
            protocols
        }
    });
    &PROTOCOLS
}

#[derive(Default)]
struct State {
    uri: Option<String>,
    /// The connection to the app and the size of the resource, while the element is started.
    source: Option<(UnixStream, Option<u64>)>,
}

/// Source element reading `<audio>` and `<video>` data from a Tauri app.
///
/// WebKitGTK hands media URLs with a custom scheme (`tauri://`, `asset://`,
/// protocols registered by the app) to GStreamer, in a process that cannot
/// reach the app's protocol handlers. This element forwards the URL to the
/// app over a Unix socket and reads the response from there, so the app
/// decides what is served, the same way it does for any other request.
///
/// Protocol, little endian:
///
/// ```text
/// client: u32 url length, url                      (once)
/// server: u8 status (0 = ok), u64 size or u64::MAX  (size is the HTTP status on failure)
/// client: u64 offset, u32 length                    (repeated)
/// server: u32 n (u32::MAX = failed), n bytes        (n < length only at the end)
/// ```
///
/// This is a source element of its own rather than a bin around an existing
/// one: a ghost pad proxying a source fails to activate in pull mode inside
/// `uridecodebin`/`urisourcebin` on GStreamer < 1.24.
#[derive(Default)]
pub struct TauriAsset {
    state: Mutex<State>,
}

impl TauriAsset {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn open(uri: &str) -> Result<(UnixStream, Option<u64>), gst::ErrorMessage> {
    let socket = std::env::var_os(SOCKET_ENV_VAR).ok_or_else(|| {
        gst::error_msg!(
            gst::ResourceError::Settings,
            ["{SOCKET_ENV_VAR} is not set, is this running in a Tauri app?"]
        )
    })?;

    let io_error = |err: io::Error| {
        gst::error_msg!(
            gst::ResourceError::OpenRead,
            ["Could not request {uri} from the app: {err}"]
        )
    };

    let mut stream = UnixStream::connect(&socket).map_err(io_error)?;
    let mut request = Vec::with_capacity(4 + uri.len());
    request.extend_from_slice(&(uri.len() as u32).to_le_bytes());
    request.extend_from_slice(uri.as_bytes());
    stream.write_all(&request).map_err(io_error)?;

    let mut response = [0; 9];
    stream.read_exact(&mut response).map_err(io_error)?;
    let value = u64::from_le_bytes(response[1..].try_into().unwrap());
    if response[0] != 0 {
        let code = match value {
            404 => gst::ResourceError::NotFound,
            401 | 403 => gst::ResourceError::NotAuthorized,
            _ => gst::ResourceError::OpenRead,
        };
        return Err(gst::error_msg!(
            code,
            ["The app answered {uri} with status {value}"]
        ));
    }

    Ok((stream, (value != UNKNOWN_SIZE).then_some(value)))
}

/// Reads up to `data.len()` bytes at `offset`, less only at the end of the resource.
fn read_at(stream: &mut UnixStream, offset: u64, data: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < data.len() {
        let mut request = [0; 12];
        request[..8].copy_from_slice(&(offset + filled as u64).to_le_bytes());
        request[8..].copy_from_slice(&((data.len() - filled) as u32).to_le_bytes());
        stream.write_all(&request)?;

        let mut len = [0; 4];
        stream.read_exact(&mut len)?;
        let len = u32::from_le_bytes(len);
        if len == READ_FAILED {
            return Err(io::Error::other("the app failed to read the resource"));
        }
        let len = len as usize;
        if len > data.len() - filled {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the app sent more than was asked for",
            ));
        }
        if len == 0 {
            break;
        }
        stream.read_exact(&mut data[filled..filled + len])?;
        filled += len;
    }
    Ok(filled)
}

#[glib::object_subclass]
impl ObjectSubclass for TauriAsset {
    const NAME: &'static str = "GstTauriAsset";
    type Type = super::TauriAsset;
    type ParentType = gst_base::BaseSrc;
    type Interfaces = (gst::URIHandler,);
}

impl ObjectImpl for TauriAsset {
    fn constructed(&self) {
        self.parent_constructed();
        let obj = self.obj();
        obj.set_format(gst::Format::Bytes);
        obj.set_blocksize(BLOCK_SIZE);
    }
}

impl GstObjectImpl for TauriAsset {}

impl ElementImpl for TauriAsset {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static METADATA: LazyLock<gst::subclass::ElementMetadata> = LazyLock::new(|| {
            gst::subclass::ElementMetadata::new(
                "Tauri Asset Source",
                "Source/Network",
                "Reads media served by a Tauri app's protocol handlers",
                "Tauri Programme within The Commons Conservancy",
            )
        });
        Some(&*METADATA)
    }

    fn pad_templates() -> &'static [gst::PadTemplate] {
        static PAD_TEMPLATES: LazyLock<Vec<gst::PadTemplate>> = LazyLock::new(|| {
            vec![
                gst::PadTemplate::new(
                    "src",
                    gst::PadDirection::Src,
                    gst::PadPresence::Always,
                    &gst::Caps::new_any(),
                )
                .unwrap(),
            ]
        });
        PAD_TEMPLATES.as_ref()
    }
}

impl BaseSrcImpl for TauriAsset {
    fn is_seekable(&self) -> bool {
        true
    }

    fn size(&self) -> Option<u64> {
        self.state().source.as_ref().and_then(|(_, size)| *size)
    }

    fn start(&self) -> Result<(), gst::ErrorMessage> {
        let mut state = self.state();
        let uri = state.uri.clone().ok_or_else(|| {
            gst::error_msg!(gst::ResourceError::NotFound, ["No URI set to read from"])
        })?;

        let source = open(&uri)?;
        gst::debug!(CAT, imp = self, "Opened {uri}, size {:?}", source.1);
        state.source = Some(source);

        Ok(())
    }

    fn stop(&self) -> Result<(), gst::ErrorMessage> {
        self.state().source = None;
        Ok(())
    }

    fn fill(
        &self,
        offset: u64,
        _length: u32,
        buffer: &mut gst::BufferRef,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        let mut state = self.state();
        let Some((stream, _)) = state.source.as_mut() else {
            gst::element_imp_error!(self, gst::CoreError::Failed, ["Not started yet"]);
            return Err(gst::FlowError::Error);
        };

        let wanted = buffer.size();
        let read = {
            let mut map = buffer.map_writable().map_err(|_| {
                gst::element_imp_error!(self, gst::CoreError::Failed, ["Failed to map buffer"]);
                gst::FlowError::Error
            })?;
            read_at(stream, offset, map.as_mut_slice()).map_err(|err| {
                gst::element_imp_error!(
                    self,
                    gst::ResourceError::Read,
                    ["Failed to read at {offset}: {err}"]
                );
                gst::FlowError::Error
            })?
        };

        if read == 0 && wanted > 0 {
            return Err(gst::FlowError::Eos);
        }
        buffer.set_size(read);

        Ok(gst::FlowSuccess::Ok)
    }
}

impl URIHandlerImpl for TauriAsset {
    const URI_TYPE: gst::URIType = gst::URIType::Src;

    fn protocols() -> &'static [&'static str] {
        protocols()
    }

    fn uri(&self) -> Option<String> {
        self.state().uri.clone()
    }

    fn set_uri(&self, uri: &str) -> Result<(), glib::Error> {
        // The app is the one making sense of the URI, e.g. `asset://localhost/<path>`
        // is checked against the asset protocol scope there.
        let has_location = uri
            .split_once("://")
            .is_some_and(|(_, location)| !location.is_empty());
        if !has_location {
            let msg = format!("URI does not contain location: {uri}");
            gst::error!(CAT, imp = self, "{msg}");
            return Err(glib::Error::new(gst::URIError::BadUri, &msg));
        }

        let mut state = self.state();
        if state.source.is_some() {
            let msg = "Changing the URI while it is being read is not supported";
            gst::error!(CAT, imp = self, "{msg}");
            return Err(glib::Error::new(gst::URIError::BadState, msg));
        }

        gst::debug!(CAT, imp = self, "URI set to {uri}");
        state.uri = Some(uri.to_string());

        Ok(())
    }
}
