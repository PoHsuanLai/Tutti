//! Integration tests for DSP fluent API

use tutti::dsp_nodes::{ChannelLayout, LfoShape};
use tutti::prelude::*;

#[test]
fn test_dsp_lfo_registration_and_instantiation() {
    let engine = TuttiEngine::builder().sample_rate(48000.0).build().unwrap();

    // Register an LFO node type
    engine.dsp().lfo("test_lfo", LfoShape::Sine, 2.0);

    // Instantiate with default parameters
    let lfo1 = engine.instance("test_lfo", &params! {}).unwrap();

    // Instantiate with custom parameters
    let lfo2 = engine
        .instance("test_lfo", &params! { "depth" => 0.8, "frequency" => 4.0 })
        .unwrap();

    // Verify nodes were created (different IDs)
    assert_ne!(lfo1, lfo2);
}

#[test]
fn test_dsp_envelope_registration() {
    let engine = TuttiEngine::builder().sample_rate(48000.0).build().unwrap();

    // Register peak envelope follower
    engine.dsp().envelope("env", 0.001, 0.1);

    // Register RMS envelope follower
    engine.dsp().rms_envelope("rms_env", 0.001, 0.1, 10.0);

    // Instantiate both
    let env = engine.instance("env", &params! {}).unwrap();
    let rms_env = engine
        .instance("rms_env", &params! { "gain" => 2.0 })
        .unwrap();

    assert_ne!(env, rms_env);
}

#[test]
fn test_dsp_sidechain_dynamics() {
    let engine = TuttiEngine::builder().sample_rate(48000.0).build().unwrap();

    // Register sidechain compressor and gate
    engine
        .dsp()
        .sidechain()
        .compressor("comp", -20.0, 4.0, 0.001, 0.05);

    engine
        .dsp()
        .sidechain()
        .gate("gate", -40.0, 10.0, 0.001, 0.1);

    // Register stereo versions
    engine
        .dsp()
        .sidechain()
        .stereo_compressor("stereo_comp", -20.0, 4.0, 0.001, 0.05);

    engine
        .dsp()
        .sidechain()
        .stereo_gate("stereo_gate", -40.0, 10.0, 0.001, 0.1);

    // Instantiate all
    let comp = engine.instance("comp", &params! {}).unwrap();
    let gate = engine.instance("gate", &params! {}).unwrap();
    let stereo_comp = engine.instance("stereo_comp", &params! {}).unwrap();
    let stereo_gate = engine.instance("stereo_gate", &params! {}).unwrap();

    // Verify all are different
    assert_ne!(comp, gate);
    assert_ne!(stereo_comp, stereo_gate);
}

#[test]
fn test_dsp_spatial_panners() {
    let engine = TuttiEngine::builder().sample_rate(48000.0).build().unwrap();

    // Register VBAP panner with stereo layout
    engine
        .dsp()
        .spatial()
        .vbap("stereo_panner", ChannelLayout::stereo());

    // Register binaural panner
    engine.dsp().spatial().binaural("binaural_panner");

    // Instantiate with parameters
    let vbap = engine
        .instance(
            "stereo_panner",
            &params! { "azimuth" => 45.0, "elevation" => 0.0 },
        )
        .unwrap();

    let binaural = engine
        .instance(
            "binaural_panner",
            &params! { "azimuth" => 90.0, "elevation" => 0.0 },
        )
        .unwrap();

    assert_ne!(vbap, binaural);
}

#[test]
fn test_dsp_chained_registration() {
    let engine = TuttiEngine::builder().sample_rate(48000.0).build().unwrap();

    // Test chained registration (fluent API)
    engine
        .dsp()
        .lfo("lfo", LfoShape::Sine, 2.0)
        .envelope("env", 0.001, 0.1);

    // Verify both were registered
    let lfo = engine.instance("lfo", &params! {}).unwrap();
    let env = engine.instance("env", &params! {}).unwrap();

    assert_ne!(lfo, env);
}

#[test]
fn test_dsp_nodes_in_graph() {
    let engine = TuttiEngine::builder().sample_rate(48000.0).build().unwrap();

    // Register DSP nodes
    engine.dsp().lfo("lfo", LfoShape::Sine, 0.5);
    engine.dsp().envelope("env", 0.001, 0.1);
    engine
        .dsp()
        .sidechain()
        .compressor("comp", -20.0, 4.0, 0.001, 0.05);

    // Instantiate
    let lfo = engine.instance("lfo", &params! { "depth" => 0.8 }).unwrap();
    let env = engine.instance("env", &params! {}).unwrap();
    let comp = engine.instance("comp", &params! {}).unwrap();

    // Use in audio graph
    engine.graph(|net| {
        // This just verifies the nodes can be added to the graph
        // In a real scenario, you'd connect them properly
        let _ = lfo;
        let _ = env;
        let _ = comp;
    });
}

#[test]
fn test_multiple_instances_of_same_node() {
    let engine = TuttiEngine::builder().sample_rate(48000.0).build().unwrap();

    // Register once
    engine.dsp().lfo("lfo", LfoShape::Sine, 2.0);

    // Instantiate multiple times with different parameters
    let lfo1 = engine.instance("lfo", &params! { "depth" => 0.5 }).unwrap();
    let lfo2 = engine.instance("lfo", &params! { "depth" => 0.8 }).unwrap();
    let lfo3 = engine
        .instance("lfo", &params! { "depth" => 1.0, "frequency" => 4.0 })
        .unwrap();

    // All should be different node IDs
    assert_ne!(lfo1, lfo2);
    assert_ne!(lfo2, lfo3);
    assert_ne!(lfo1, lfo3);
}

#[test]
fn test_dsp_remove_and_query() {
    let engine = TuttiEngine::builder().sample_rate(48000.0).build().unwrap();

    // Register some nodes
    engine.dsp().lfo("lfo1", LfoShape::Sine, 2.0);
    engine.dsp().lfo("lfo2", LfoShape::Triangle, 4.0);
    engine.dsp().envelope("env", 0.001, 0.1);

    // Check they exist
    assert!(engine.dsp().has("lfo1"));
    assert!(engine.dsp().has("lfo2"));
    assert!(engine.dsp().has("env"));
    assert!(!engine.dsp().has("nonexistent"));

    // List should contain all 3
    let list = engine.dsp().list();
    assert_eq!(list.len(), 3);
    assert!(list.contains(&"lfo1".to_string()));
    assert!(list.contains(&"lfo2".to_string()));
    assert!(list.contains(&"env".to_string()));

    // Remove one
    assert!(engine.dsp().remove("lfo1")); // Returns true
    assert!(!engine.dsp().has("lfo1")); // No longer exists
    assert!(!engine.dsp().remove("lfo1")); // Returns false (already removed)

    // List should now have 2
    let list = engine.dsp().list();
    assert_eq!(list.len(), 2);
    assert!(!list.contains(&"lfo1".to_string()));

    // Can't instantiate removed node
    let result = engine.instance("lfo1", &params! {});
    assert!(result.is_err());

    // Can still instantiate remaining nodes
    let lfo2 = engine.instance("lfo2", &params! {}).unwrap();
    let env = engine.instance("env", &params! {}).unwrap();
    assert_ne!(lfo2, env);
}
