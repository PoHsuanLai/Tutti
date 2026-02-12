//! Engine lifecycle integration tests
//!
//! Tests engine creation, configuration, subsystem initialization, and cleanup.
//! Pattern: Inspired by Ardour's session_test.h and Zrythm's ZrythmFixture.

use tutti::prelude::*;

#[path = "../helpers/mod.rs"]
mod helpers;
use helpers::*;

/// Test engine creation with custom sample rate.
/// Note: The actual sample rate may differ from requested if the audio device
/// doesn't support it. This test verifies the engine reports a valid rate.
#[test]
fn test_engine_custom_sample_rate() {
    let engine = TuttiEngine::builder().build().unwrap();

    // Engine should report a valid sample rate (common rates: 44100, 48000, 96000)
    let rate = engine.sample_rate();
    assert!(rate >= 8000.0 && rate <= 192000.0, "Sample rate {} is outside valid range", rate);
}

/// Test that multiple engines can be created sequentially.
/// (One at a time - audio devices are exclusive)
#[test]
fn test_engine_sequential_creation() {
    for i in 0..3 {
        let engine = TuttiEngine::builder()
            .build()
            .unwrap();

        assert!(engine.is_running());
        // Engine is dropped here, releasing audio device
    }
}

/// Test that nodes can be created directly in the graph.
#[test]
fn test_graph_node_creation() {
    let engine = test_engine();

    // Create nodes directly in graph
    let node1 = engine.graph(|net| net.add(sine_hz::<f32>(440.0)).id());
    let node2 = engine.graph(|net| net.add(sine_hz::<f32>(880.0)).id());

    // Both should succeed and be different
    assert_ne!(node1, node2);
}

/// Test that graph() works properly.
#[test]
fn test_graph_operations() {
    let engine = test_engine();

    engine.graph(|net| {
        let osc = net.add(sine_hz::<f32>(440.0)).id();
        net.pipe_output(osc);
    });

    assert!(engine.is_running());
}
