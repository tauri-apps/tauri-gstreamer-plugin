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
use percent_encoding::{AsciiSet, CONTROLS};

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

/// Characters percent-encoded when turning the location back into a URI.
const PATH_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

#[derive(Default)]
struct State {
    location: Option<String>,
    /// The open file, while the element is started.
    file: Option<OpenFile>,
}

struct OpenFile {
    file: File,
    /// Offset the file cursor is at, `None` when unknown (after an error).
    position: Option<u64>,
    /// Only known for regular files, which are also the only seekable ones.
    size: Option<u64>,
}

impl OpenFile {
    /// Fills `data` from `offset`, returning how many bytes were read.
    ///
    /// Only stops short at the end of the file, downstream expects full blocks.
    fn read_at(&mut self, offset: u64, data: &mut [u8]) -> Result<usize, gst::ErrorMessage> {
        if self.position != Some(offset) {
            self.position = None;
            self.file.seek(SeekFrom::Start(offset)).map_err(|err| {
                gst::error_msg!(
                    gst::ResourceError::Seek,
                    ["Failed to seek to {offset}: {err}"]
                )
            })?;
            self.position = Some(offset);
        }

        let mut read = 0;
        while read < data.len() {
            match self.file.read(&mut data[read..]) {
                Ok(0) => break,
                Ok(n) => {
                    read += n;
                    self.position = Some(offset + read as u64);
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                Err(err) => {
                    self.position = None;
                    return Err(gst::error_msg!(
                        gst::ResourceError::Read,
                        ["Failed to read at {}: {err}", offset + read as u64]
                    ));
                }
            }
        }
        Ok(read)
    }
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
        self.state()
            .file
            .as_ref()
            .is_some_and(|file| file.size.is_some())
    }

    fn size(&self) -> Option<u64> {
        self.state().file.as_ref()?.size
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
        let metadata = file.metadata().map_err(|err| {
            gst::error_msg!(
                gst::ResourceError::OpenRead,
                ["Could not read metadata of {location}: {err}"]
            )
        })?;
        if metadata.is_dir() {
            return Err(gst::error_msg!(
                gst::ResourceError::OpenRead,
                ["{location} is a directory"]
            ));
        }

        // FIFOs, devices and procfs/sysfs entries report a length of 0 and cannot
        // seek, so only regular files get a size (and with it seekability).
        let size = metadata.file_type().is_file().then_some(metadata.len());

        gst::debug!(CAT, imp = self, "Opened {location}, size {size:?}");
        state.file = Some(OpenFile {
            file,
            position: Some(0),
            size,
        });

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
        let Some(file) = state.file.as_mut() else {
            gst::element_imp_error!(self, gst::CoreError::Failed, ["Not started yet"]);
            return Err(gst::FlowError::Error);
        };

        let wanted = buffer.size();
        let read = {
            let mut map = buffer.map_writable().map_err(|_| {
                gst::element_imp_error!(self, gst::CoreError::Failed, ["Failed to map buffer"]);
                gst::FlowError::Error
            })?;
            file.read_at(offset, map.as_mut_slice()).map_err(|msg| {
                self.post_error_message(msg);
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
        &[ASSET_URI_SCHEME]
    }

    fn uri(&self) -> Option<String> {
        self.state().location.as_ref().map(|location| {
            let location = percent_encoding::utf8_percent_encode(location, PATH_ENCODE_SET);
            format!("{ASSET_URI_SCHEME}://{location}")
        })
    }

    fn set_uri(&self, uri: &str) -> Result<(), glib::Error> {
        let bad_uri = |msg: String| {
            gst::error!(CAT, imp = self, "{msg}");
            glib::Error::new(gst::URIError::BadUri, &msg)
        };

        // uri is like: asset://path/to/asset or asset://localhost/path/to/asset
        let location = uri
            .strip_prefix(ASSET_URI_SCHEME)
            .and_then(|rest| rest.strip_prefix("://"))
            .ok_or_else(|| bad_uri(format!("Not an {ASSET_URI_SCHEME} URI: {uri}")))?;

        // directly having full path after asset:// or having localhost as the host
        let location = match location.strip_prefix("localhost") {
            Some(path) if path.is_empty() || path.starts_with('/') => path,
            _ => location,
        };

        // Uri could be percent-encoded
        let location = percent_encoding::percent_decode_str(location)
            .decode_utf8()
            .map_err(|_| bad_uri(format!("Failed to decode percent-encoded URI: {uri}")))?
            .into_owned();
        if location.is_empty() {
            return Err(bad_uri(format!("URI does not contain location: {uri}")));
        }

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
