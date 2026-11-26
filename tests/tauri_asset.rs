// Copyright 2026-2026 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use gst::prelude::*;
use gstreamer as gst;
use gstreamer_check as gst_check;

fn init() {
    use std::sync::Once;
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        gst::init().unwrap();
        gsttauriasset::plugin_register_static().unwrap();
    });
}

#[test]
fn test_uri_handler_interface() {
    init();
    let element = gst::ElementFactory::make("tauriasset")
        .build()
        .expect("Failed to create tauriasset element");

    let uri_handler = element
        .dynamic_cast::<gst::URIHandler>()
        .expect("Element should implement URIHandler");

    let protocols = uri_handler.protocols();
    assert!(!protocols.is_empty(), "Should have at least one protocol");
    assert!(
        protocols.contains(&"asset".into()),
        "Should support 'asset' protocol"
    );
}

#[test]
fn test_set_uri_basic() {
    init();
    let element = gst::ElementFactory::make("tauriasset")
        .build()
        .expect("Failed to create tauriasset element");

    let uri_handler = element
        .dynamic_cast::<gst::URIHandler>()
        .expect("Element should implement URIHandler");

    let uri = "asset:///tmp/test.mp3";
    let result = uri_handler.set_uri(uri);
    assert!(result.is_ok(), "Setting valid URI should succeed");
    assert_eq!(
        uri_handler.uri().unwrap(),
        uri,
        "URI should be set correctly"
    );
}

#[test]
fn test_set_uri_with_localhost() {
    init();
    let element = gst::ElementFactory::make("tauriasset")
        .build()
        .expect("Failed to create tauriasset element");

    let uri_handler = element
        .dynamic_cast::<gst::URIHandler>()
        .expect("Element should implement URIHandler");

    let result = uri_handler.set_uri("asset://localhost/tmp/test.mp3");
    assert!(result.is_ok(), "Setting URI with localhost should succeed");
    assert_eq!(
        uri_handler.uri().unwrap(),
        "asset:///tmp/test.mp3",
        "URI with localhost should be set correctly"
    );
}

#[test]
fn test_set_wrong_protocol_uri() {
    init();
    let element = gst::ElementFactory::make("tauriasset")
        .build()
        .expect("Failed to create tauriasset element");

    let uri_handler = element
        .dynamic_cast::<gst::URIHandler>()
        .expect("Element should implement URIHandler");

    let result = uri_handler.set_uri("invalid://test.mp3");
    assert_eq!(
        result.unwrap_err().kind::<gst::URIError>(),
        Some(gst::URIError::UnsupportedProtocol)
    );
}

#[test]
fn test_no_path() {
    init();
    let element = gst::ElementFactory::make("tauriasset")
        .build()
        .expect("Failed to create tauriasset element");

    let uri_handler = element
        .dynamic_cast::<gst::URIHandler>()
        .expect("Element should implement URIHandler");

    let result = uri_handler.set_uri("asset://");
    assert_eq!(
        result.unwrap_err().kind::<gst::URIError>(),
        Some(gst::URIError::BadUri)
    );
}

#[test]
fn test_set_uri_with_percent_encoding() {
    init();
    let element = gst::ElementFactory::make("tauriasset")
        .build()
        .expect("Failed to create tauriasset element");

    let uri_handler = element
        .dynamic_cast::<gst::URIHandler>()
        .expect("Element should implement URIHandler");

    let result = uri_handler.set_uri("asset:///tmp/test%20file.mp3");
    assert!(
        result.is_ok(),
        "Setting URI with percent-encoded characters should succeed"
    );

    assert_eq!(uri_handler.uri().unwrap(), "asset:///tmp/test%20file.mp3");
}

fn new_uri_handler() -> gst::URIHandler {
    init();
    gst::ElementFactory::make("tauriasset")
        .build()
        .expect("Failed to create tauriasset element")
        .dynamic_cast::<gst::URIHandler>()
        .expect("Element should implement URIHandler")
}

#[test]
fn test_uri_round_trips_special_characters() {
    let uri_handler = new_uri_handler();
    let uri = "asset:///tmp/100%25%20%23%3F.mp3";
    uri_handler.set_uri(uri).unwrap();
    assert_eq!(uri_handler.uri().unwrap(), uri);

    let other = new_uri_handler();
    other.set_uri(&uri_handler.uri().unwrap()).unwrap();
    assert_eq!(other.uri().unwrap(), uri);
}

#[test]
fn test_localhost_without_path() {
    let result = new_uri_handler().set_uri("asset://localhost");
    assert_eq!(
        result.unwrap_err().kind::<gst::URIError>(),
        Some(gst::URIError::BadUri)
    );
}

#[test]
fn test_host_starting_with_localhost_is_kept() {
    let uri_handler = new_uri_handler();
    uri_handler
        .set_uri("asset://localhost-cache/a.mp3")
        .unwrap();
    assert_eq!(uri_handler.uri().unwrap(), "asset://localhost-cache/a.mp3");
}

#[test]
fn test_path_containing_scheme_is_kept() {
    let uri_handler = new_uri_handler();
    uri_handler
        .set_uri("asset:///data/cache/asset://x/song.mp3")
        .unwrap();
    assert_eq!(
        uri_handler.uri().unwrap(),
        "asset:///data/cache/asset://x/song.mp3"
    );
}

#[cfg(unix)]
#[test]
fn test_read_fifo() {
    use std::io::Write;

    init();

    let dir = std::env::temp_dir().join(format!("tauriasset-fifo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fifo = dir.join("audio.fifo");
    let _ = std::fs::remove_file(&fifo);
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("Failed to run mkfifo");
    assert!(status.success());

    let expected_data: Vec<u8> = (0..10_000u32).map(|i| i as u8).collect();
    let writer = {
        let fifo = fifo.clone();
        let data = expected_data.clone();
        std::thread::spawn(move || {
            // blocks until the element opens the FIFO for reading
            let mut file = std::fs::OpenOptions::new().write(true).open(fifo).unwrap();
            file.write_all(&data).unwrap();
        })
    };

    let mut h = gst_check::Harness::new("tauriasset");
    let element = h.element().expect("Harness should have an element");
    element
        .dynamic_cast_ref::<gst::URIHandler>()
        .unwrap()
        .set_uri(&format!("asset://{}", fifo.to_str().unwrap()))
        .unwrap();
    h.play();

    let mut data = Vec::new();
    while data.len() < expected_data.len() {
        let buffer = h.pull().expect("Failed to pull buffer");
        data.extend_from_slice(&buffer.map_readable().unwrap());
    }
    writer.join().unwrap();

    assert_eq!(data, expected_data);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_read_file() {
    init();

    let audio_name = "test_mono_s16.flac";
    let expected_data = include_bytes!("test_mono_s16.flac");

    let mut path = std::env::current_dir().unwrap();
    path.push(format!("tests/{}", audio_name));
    let uri = format!("asset://{}", path.to_str().unwrap());

    let mut h = gst_check::Harness::new("tauriasset");

    let element = h.element().expect("Harness should have an element");
    let uri_handler = element
        .dynamic_cast_ref::<gst::URIHandler>()
        .expect("Element should implement URIHandler");

    uri_handler.set_uri(&uri).expect("Failed to set URI");

    h.play();

    let buffer = h.pull().expect("Failed to pull buffer");
    let map = buffer.map_readable().expect("Failed to map buffer");

    assert_eq!(&*map, expected_data);
}
