//! Sending to the web UI on the audio thread, with the allocation check of
//! beamer-core installed. A binary of its own, so that the allocator does
//! not reach other tests.

#![cfg(debug_assertions)]

use beamer_core::allocation_check::{self, CheckingAllocator};
use beamer_webview::{WebViewEventQueue, WebViewStream, WebViewValue};

#[global_allocator]
static ALLOCATOR: CheckingAllocator = CheckingAllocator;

#[test]
fn test_sending_to_the_web_ui_neither_allocates_nor_frees() {
    let events = WebViewEventQueue::new();
    events.claim();
    events.editor_opened();
    // The fields as `#[beamer::model]` connects them.
    let mut levels_field = WebViewValue::<[f32; 2]>::default();
    levels_field.connect("levels".to_string(), &events);
    let mut spectrum_field = WebViewValue::<Vec<f32>>::default();
    spectrum_field.connect("spectrum".to_string(), &events);
    let mut history_field = WebViewStream::<f32>::default();
    history_field.connect("history".to_string(), &events);
    let mut levels = levels_field.sender([0.0f32; 2]);
    let mut spectrum = spectrum_field.sender(vec![0.0f32; 512]);
    let mut history = history_field.sender(64);
    let bins = vec![0.5f32; 512];

    let (_, counts) = allocation_check::count_in_process(|| {
        for _ in 0..3 {
            if events.is_editor_open() {
                levels.send(&[0.5, 0.25]);
                spectrum.send(&bins);
                spectrum.send_with(|values| values.fill(0.25));
                history.push(0.5f32);
                history.push_slice(&[0.25, 0.125]);
            }
        }
    });
    assert!(counts.is_empty(), "{counts:?}");
    assert!(events.take_script().is_some());
}
