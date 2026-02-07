//! Integration tests for DSP nodes
//!
//! The DSP API provides factory methods for creating DSP nodes directly.
//! Use `engine.graph()` to add nodes to the audio graph.

use tutti::dsp_nodes::{LfoNode, LfoShape};
use tutti::prelude::*;

#[test]
fn test_dsp_lfo_creation() {
    let engine = TuttiEngine::builder().sample_rate(48000.0).build().unwrap();

    // Create LFO nodes directly
    let lfo1 = LfoNode::new(LfoShape::Sine, 2.0);

    let lfo2 = LfoNode::new(LfoShape::Sine, 4.0);
    lfo2.set_depth(0.8);

    // Add to graph
    let lfo1_id = engine.graph(|net| net.add(lfo1).id());
    let lfo2_id = engine.graph(|net| net.add(lfo2).id());

    // Verify nodes were created (different IDs)
    assert_ne!(lfo1_id, lfo2_id);
}

#[test]
#[cfg(feature = "dsp-dynamics")]
fn test_dsp_sidechain_dynamics_creation() {
    let engine = TuttiEngine::builder().sample_rate(48000.0).build().unwrap();

    use tutti::dsp_nodes::{SidechainCompressor, SidechainGate};
    use tutti::dsp_nodes::{StereoSidechainCompressor, StereoSidechainGate};

    // Create sidechain compressor and gate using builder pattern
    let comp = SidechainCompressor::builder()
        .threshold_db(-20.0)
        .ratio(4.0)
        .attack_seconds(0.001)
        .release_seconds(0.05)
        .build();

    let gate = SidechainGate::builder()
        .threshold_db(-40.0)
        .attack_seconds(0.001)
        .hold_seconds(0.01)
        .release_seconds(0.1)
        .build();

    // Create stereo versions
    let stereo_comp = StereoSidechainCompressor::new(-20.0, 4.0, 0.001, 0.05);
    let stereo_gate = StereoSidechainGate::new(-40.0, 0.001, 0.01, 0.1);

    // Add to graph
    let comp_id = engine.graph(|net| net.add(comp).id());
    let gate_id = engine.graph(|net| net.add(gate).id());
    let stereo_comp_id = engine.graph(|net| net.add(stereo_comp).id());
    let stereo_gate_id = engine.graph(|net| net.add(stereo_gate).id());

    // Verify all are different
    assert_ne!(comp_id, gate_id);
    assert_ne!(stereo_comp_id, stereo_gate_id);
}

#[test]
#[cfg(feature = "dsp-spatial")]
fn test_dsp_spatial_panners_creation() {
    let engine = TuttiEngine::builder().sample_rate(48000.0).build().unwrap();

    use tutti::dsp_nodes::{BinauralPannerNode, SpatialPannerNode};

    // Create VBAP panner with stereo layout using factory method
    let vbap = SpatialPannerNode::stereo().expect("Failed to create stereo panner");

    // Create binaural panner
    let binaural = BinauralPannerNode::new(48000.0);

    // Add to graph
    let vbap_id = engine.graph(|net| net.add(vbap).id());
    let binaural_id = engine.graph(|net| net.add(binaural).id());

    assert_ne!(vbap_id, binaural_id);
}

#[test]
#[cfg(feature = "dsp-dynamics")]
fn test_dsp_nodes_in_graph() {
    let engine = TuttiEngine::builder().sample_rate(48000.0).build().unwrap();

    use tutti::dsp_nodes::SidechainCompressor;

    // Create DSP nodes
    let lfo = LfoNode::new(LfoShape::Sine, 0.5);
    lfo.set_depth(0.8);

    let comp = SidechainCompressor::builder()
        .threshold_db(-20.0)
        .ratio(4.0)
        .attack_seconds(0.001)
        .release_seconds(0.05)
        .build();

    // Use in audio graph
    engine.graph(|net| {
        let lfo_id = net.add(lfo).id();
        let comp_id = net.add(comp).id();

        // Verify they exist
        assert_ne!(lfo_id, comp_id);
    });
}

#[test]
fn test_multiple_lfo_instances() {
    let engine = TuttiEngine::builder().sample_rate(48000.0).build().unwrap();

    // Create multiple LFO instances with different parameters
    let lfo1 = LfoNode::new(LfoShape::Sine, 2.0);
    lfo1.set_depth(0.5);

    let lfo2 = LfoNode::new(LfoShape::Sine, 2.0);
    lfo2.set_depth(0.8);

    let lfo3 = LfoNode::new(LfoShape::Sine, 4.0);
    lfo3.set_depth(1.0);

    // Add to graph
    let lfo1_id = engine.graph(|net| net.add(lfo1).id());
    let lfo2_id = engine.graph(|net| net.add(lfo2).id());
    let lfo3_id = engine.graph(|net| net.add(lfo3).id());

    // All should be different node IDs
    assert_ne!(lfo1_id, lfo2_id);
    assert_ne!(lfo2_id, lfo3_id);
    assert_ne!(lfo1_id, lfo3_id);
}
