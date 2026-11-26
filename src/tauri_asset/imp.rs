// Copyright 2026-2026 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
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

const ASSET_URI_SCHEME: &str = "asset";

#[derive(Default)]
struct State {
    location: Option<String>,
    /// The open file and the offset its cursor is at, while the element is started.
    file: Option<(File, u64)>,
}

/// Source element reading the file an `asset://` URI points to.
///
/// This is a source element of its own rather than a bin around `filesrc`:
/// a ghost pad proxying `filesrc` fails to activate in pull mode inside
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
        self.obj().set_format(gst::Format::Bytes);
    }
}

impl GstObjectImpl for TauriAsset {}

impl ElementImpl for TauriAsset {
    fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
        static METADATA: LazyLock<gst::subclass::ElementMetadata> = LazyLock::new(|| {
            gst::subclass::ElementMetadata::new(
                "Tauri Asset Source",
                "Source/File",
                "Reads files requested through Tauri's asset protocol",
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
        let state = self.state();
        let (file, _) = state.file.as_ref()?;
        file.metadata().ok().map(|metadata| metadata.len())
    }

    fn start(&self) -> Result<(), gst::ErrorMessage> {
        let mut state = self.state();
        let location = state.location.clone().ok_or_else(|| {
            gst::error_msg!(gst::ResourceError::NotFound, ["No URI set to read from"])
        })?;

        let file = File::open(&location).map_err(|err| {
            let code = if err.kind() == std::io::ErrorKind::NotFound {
                gst::ResourceError::NotFound
            } else {
                gst::ResourceError::OpenRead
            };
            gst::error_msg!(code, ["Could not open file {location} for reading: {err}"])
        })?;
        if file.metadata().is_ok_and(|metadata| metadata.is_dir()) {
            return Err(gst::error_msg!(
                gst::ResourceError::OpenRead,
                ["{location} is a directory"]
            ));
        }

        gst::debug!(CAT, imp = self, "Opened {location}");
        state.file = Some((file, 0));

        Ok(())
    }

    fn stop(&self) -> Result<(), gst::ErrorMessage> {
        self.state().file = None;
        Ok(())
    }

    fn fill(
        &self,
        offset: u64,
        _length: u32,
        buffer: &mut gst::BufferRef,
    ) -> Result<gst::FlowSuccess, gst::FlowError> {
        let mut state = self.state();
        let Some((file, position)) = state.file.as_mut() else {
            gst::element_imp_error!(self, gst::CoreError::Failed, ["Not started yet"]);
            return Err(gst::FlowError::Error);
        };

        if *position != offset {
            file.seek(SeekFrom::Start(offset)).map_err(|err| {
                gst::element_imp_error!(
                    self,
                    gst::ResourceError::Seek,
                    ["Failed to seek to {offset}: {err}"]
                );
                gst::FlowError::Error
            })?;
            *position = offset;
        }

        let wanted = buffer.size();
        let read = {
            let mut map = buffer.map_writable().map_err(|_| {
                gst::element_imp_error!(self, gst::CoreError::Failed, ["Failed to map buffer"]);
                gst::FlowError::Error
            })?;
            let data = map.as_mut_slice();

            // Downstream expects full blocks until the end of the file.
            let mut read = 0;
            while read < data.len() {
                match file.read(&mut data[read..]) {
                    Ok(0) => break,
                    Ok(n) => read += n,
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(err) => {
                        gst::element_imp_error!(
                            self,
                            gst::ResourceError::Read,
                            ["Failed to read at {offset}: {err}"]
                        );
                        return Err(gst::FlowError::Error);
                    }
                }
            }
            read
        };

        *position += read as u64;
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
        &[ASSET_URI_SCHEME]
    }

    fn uri(&self) -> Option<String> {
        self.state()
            .location
            .as_ref()
            .map(|location| format!("{ASSET_URI_SCHEME}://{location}"))
    }

    fn set_uri(&self, uri: &str) -> Result<(), glib::Error> {
        // uri is like: asset://path/to/asset or asset://localhost/path/to/asset
        let sep = format!("{}://", ASSET_URI_SCHEME);
        let mut split = uri.split(sep.as_str());
        let location = split.nth(1).filter(|l| !l.is_empty()).ok_or_else(|| {
            let msg = format!("URI does not contain location: {}", uri);
            gst::error!(CAT, imp = self, "{msg}");
            glib::Error::new(gst::URIError::BadUri, &msg)
        })?;

        // directly having full path after asset:// or having localhost
        let location = location.strip_prefix("localhost").unwrap_or(location);

        // Uri could be percent-encoded
        let location = percent_encoding::percent_decode_str(location)
            .decode_utf8()
            .map_err(|_| {
                let msg = format!("Failed to decode percent-encoded URI: {}", uri);
                gst::error!(CAT, imp = self, "{msg}");
                glib::Error::new(gst::URIError::BadUri, &msg)
            })?
            .to_string();

        let mut state = self.state();
        if state.file.is_some() {
            let msg = "Changing the URI while the file is open is not supported";
            gst::error!(CAT, imp = self, "{msg}");
            return Err(glib::Error::new(gst::URIError::BadState, msg));
        }

        gst::debug!(CAT, imp = self, "URI from \"{}\" to \"{}\"", uri, &location);
        state.location = Some(location);

        Ok(())
    }
}
