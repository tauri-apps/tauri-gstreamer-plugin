// Copyright 2026-2026 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use gst::glib;
use gst::prelude::*;
use gstreamer as gst;

mod imp;

pub use imp::PROTOCOLS_ENV_VAR;

glib::wrapper! {
    pub struct TauriAsset(ObjectSubclass<imp::TauriAsset>)
    @extends gstreamer_base::BaseSrc, gst::Element, gst::Object,
    @implements gst::URIHandler;
}

pub fn register(plugin: &gst::Plugin) -> Result<(), glib::BoolError> {
    gst::Element::register(
        Some(plugin),
        "tauriasset",
        gst::Rank::PRIMARY,
        TauriAsset::static_type(),
    )
}
