//! Audio graph integration tests
//!
//! Tests DSP graph construction, node routing, and signal flow.
//! Pattern: Inspired by Ardour's audiographer tests and Zrythm's graph tests.

use tutti::prelude::*;

#[path = "../helpers/mod.rs"]
mod helpers;
use helpers::*;

/// Test node creation directly in graph.
/// Verifies that each created node gets a unique ID.
#[test]
fn test_graph_direct_nodes() {
    let engine = test_engine();

    // Create nodes directly in graph
    let osc1 = engine.graph(|net| net.add(sine_hz::<f64>(220.0)).id());
    let osc2 = engine.graph(|net| net.add(sine_hz::<f64>(440.0)).id());
    let filter = engine.graph(|net| net.add(lowpole_hz(800.0)).id());

    // Verify different instances get unique IDs
    assert_ne!(osc1, osc2);
    assert_ne!(osc1, filter);
}

/// Test parameterized node via register + graph.
#[test]
fn test_graph_registered_nodes() {
    let engine = test_engine();

    // Register parameterized nodes
    engine.register("osc", |p| {
        let freq: f32 = p.get_or("freq", 440.0);
        sine_hz::<f64>(freq)
    });

    engine.register("filter", |p| {
        let cutoff: f64 = p.get_or("cutoff", 1000.0);
        lowpole_hz(cutoff)
    });

    // Nodes can now be accessed via registry, but we use graph() directly
    let osc = engine.graph(|net| net.add(sine_hz::<f64>(440.0)).id());
    let filter = engine.graph(|net| net.add(lowpole_hz(800.0)).id());

    // Verify different nodes get unique IDs
    assert_ne!(osc, filter);
}

/// Test DSP nodes created directly.
/// Verifies LFO parameters can be set and instances are unique.
#[test]
fn test_graph_dsp_lfo() {
    let engine = test_engine();

    use tutti::dsp_nodes::{LfoNode, LfoShape};

    let lfo1 = LfoNode::new(LfoShape::Sine, 5.0);
    lfo1.set_depth(0.5);

    let lfo2 = LfoNode::new(LfoShape::Sine, 5.0);
    lfo2.set_depth(0.8);

    let lfo1_id = engine.graph(|net| net.add(lfo1).id());
    let lfo2_id = engine.graph(|net| net.add(lfo2).id());

    assert_ne!(lfo1_id, lfo2_id);
}

/// Test stereo split node routing.
#[test]
fn test_graph_stereo_split() {
    let engine = test_engine();

    engine.graph(|net| {
        let mono = net.add(sine_hz::<f64>(440.0)).id();
        let split = net.add_split();
        let reverb = net.add(reverb_stereo(10.0, 2.0, 0.5)).id();

        net.pipe(mono, split);
        net.pipe_all(split, reverb);
        net.pipe_output(reverb);
    });

    assert!(engine.is_running());
}
