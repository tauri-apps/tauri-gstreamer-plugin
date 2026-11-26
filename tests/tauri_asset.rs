// Copyright 2026-2026 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

use gst::prelude::*;
use gstreamer as gst;
use gstreamer_check as gst_check;

const AUDIO: &[u8] = include_bytes!("test_mono_s16.flac");

/// Stands in for the Tauri app: serves [`AUDIO`] for every URL but the ones containing `denied`.
fn serve(mut stream: UnixStream) -> std::io::Result<()> {
    let mut len = [0; 4];
    stream.read_exact(&mut len)?;
    let mut url = vec![0; u32::from_le_bytes(len) as usize];
    stream.read_exact(&mut url)?;

    if String::from_utf8_lossy(&url).contains("denied") {
        stream.write_all(&[1])?;
        stream.write_all(&403u64.to_le_bytes())?;
        return Ok(());
    }
    stream.write_all(&[0])?;
    stream.write_all(&(AUDIO.len() as u64).to_le_bytes())?;

    loop {
        let mut request = [0; 12];
        if stream.read_exact(&mut request).is_err() {
            return Ok(());
        }
        let offset = u64::from_le_bytes(request[..8].try_into().unwrap()) as usize;
        let len = u32::from_le_bytes(request[8..].try_into().unwrap()) as usize;
        let data = AUDIO.get(offset..).unwrap_or_default();
        // Answer in small pieces so the element has to ask again for the rest.
        let data = &data[..data.len().min(len).min(1000)];
        stream.write_all(&(data.len() as u32).to_le_bytes())?;
        stream.write_all(data)?;
    }
}

fn init() {
    use std::sync::Once;
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        let dir = std::env::temp_dir().join(format!("gsttauriasset-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("socket");
        let listener = UnixListener::bind(&socket).unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || serve(stream));
            }
        });
        // SAFETY: nothing else reads the environment yet, the element only does once started.
        unsafe { std::env::set_var("TAURI_MEDIA_SOCKET", &socket) };

        gst::init().unwrap();
        gsttauriasset::plugin_register_static().unwrap();
    });
}

fn uri_handler() -> gst::URIHandler {
    gst::ElementFactory::make("tauriasset")
        .build()
        .expect("Failed to create tauriasset element")
        .dynamic_cast::<gst::URIHandler>()
        .expect("Element should implement URIHandler")
}

#[test]
fn test_uri_handler_interface() {
    init();
    let protocols = uri_handler().protocols();
    assert!(
        protocols.contains(&"asset".into()),
        "Should support 'asset' protocol"
    );
    assert!(
        protocols.contains(&"tauri".into()),
        "Should support 'tauri' protocol"
    );
}

#[test]
fn test_set_uri_is_kept_as_is() {
    init();
    let uri_handler = uri_handler();

    // The app interprets the URI, so nothing may be decoded or stripped here.
    for uri in [
        "asset://localhost/%2Ftmp%2Ftest%20file.mp3",
        "tauri://localhost/assets/sound.mp3?v=1",
    ] {
        uri_handler.set_uri(uri).expect("valid URI");
        assert_eq!(uri_handler.uri().unwrap(), uri);
    }
}

#[test]
fn test_set_wrong_protocol_uri() {
    init();
    let result = uri_handler().set_uri("invalid://test.mp3");
    assert_eq!(
        result.unwrap_err().kind::<gst::URIError>(),
        Some(gst::URIError::UnsupportedProtocol)
    );
}

#[test]
fn test_no_path() {
    init();
    let result = uri_handler().set_uri("asset://");
    assert_eq!(
        result.unwrap_err().kind::<gst::URIError>(),
        Some(gst::URIError::BadUri)
    );
}

#[test]
fn test_read() {
    init();

    let mut h = gst_check::Harness::new("tauriasset");
    let element = h.element().expect("Harness should have an element");
    element
        .dynamic_cast_ref::<gst::URIHandler>()
        .expect("Element should implement URIHandler")
        .set_uri("tauri://localhost/test_mono_s16.flac")
        .expect("Failed to set URI");

    h.play();

    let mut data = Vec::new();
    while data.len() < AUDIO.len() {
        let buffer = h.pull().expect("Failed to pull buffer");
        data.extend_from_slice(&buffer.map_readable().expect("Failed to map buffer"));
    }
    assert_eq!(data, AUDIO);
}

/// Runs `uri ! fakesink` and returns whether it reached the end of the stream.
fn play(uri: &str) -> Result<(), gst::glib::Error> {
    let source = gst::Element::make_from_uri(gst::URIType::Src, uri, None)
        .expect("Should find an element for the URI");
    assert_eq!(source.factory().unwrap().name(), "tauriasset");
    let sink = gst::ElementFactory::make("fakesink").build().unwrap();

    let pipeline = gst::Pipeline::new();
    pipeline.add_many([&source, &sink]).unwrap();
    source.link(&sink).unwrap();
    let _ = pipeline.set_state(gst::State::Playing);

    let message = pipeline
        .bus()
        .unwrap()
        .timed_pop_filtered(
            gst::ClockTime::from_seconds(10),
            &[gst::MessageType::Error, gst::MessageType::Eos],
        )
        .expect("Pipeline should finish or fail");
    let result = match message.view() {
        gst::MessageView::Error(err) => Err(err.error()),
        _ => Ok(()),
    };
    pipeline.set_state(gst::State::Null).unwrap();
    result
}

#[test]
fn test_pipeline_reads_to_the_end() {
    init();
    play("asset://localhost/allowed.flac").expect("Should play");
}

#[test]
fn test_app_refusing_the_uri_is_an_error() {
    init();
    let error = play("asset://localhost/denied.flac").expect_err("Should not play");
    assert_eq!(
        error.kind::<gst::ResourceError>(),
        Some(gst::ResourceError::NotAuthorized)
    );
}
