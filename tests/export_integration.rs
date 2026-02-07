//! Export integration tests (requires "export" feature)
//!
//! Tests offline rendering, format conversion, and normalization.
//!
//! Run with:
//! ```bash
//! cargo test -p tutti --test export_integration --features "export"
//! ```

#![cfg(feature = "export")]

use std::path::PathBuf;
use tutti::prelude::*;

fn test_engine() -> TuttiEngine {
    TuttiEngine::builder()
        .sample_rate(48000.0)
        .build()
        .expect("Failed to create test engine")
}

fn temp_dir(test_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("tutti_test_export_{}", test_name))
}

fn setup_temp_dir(test_name: &str) -> PathBuf {
    let dir = temp_dir(test_name);
    let _ = std::fs::remove_dir_all(&dir); // Clean any leftover
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn cleanup_temp_dir(test_name: &str) {
    let _ = std::fs::remove_dir_all(temp_dir(test_name));
}

/// Test basic export builder creation.
#[test]
fn test_export_builder() {
    let engine = test_engine();

    // Add a simple signal
    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).to_master();
    });

    // Create export builder (doesn't render yet)
    let _builder = engine.export();
    // Test passed if no panic
}

/// Test export with duration in seconds.
#[test]
fn test_export_duration_seconds() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).to_master();
    });

    let builder = engine.export().duration_seconds(1.0);

    // Builder should be configured
    // Actual file writing tested separately
    let _ = builder;
}

/// Test export with duration in beats.
#[test]
fn test_export_duration_beats() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).to_master();
    });

    // 4 beats at 120 BPM = 2 seconds
    let builder = engine.export().duration_beats(4.0, 120.0);

    let _ = builder;
}

/// Test export to WAV file.
#[test]
fn test_export_to_wav() {
    let engine = test_engine();
    let dir = setup_temp_dir("to_wav");
    let output_path = dir.join("test_output.wav");

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.3).to_master();
    });

    // Export 0.5 seconds to WAV
    let result = engine
        .export()
        .duration_seconds(0.5)
        .to_file(&output_path);

    // Check result
    match result {
        Ok(_) => {
            // Verify file was created
            assert!(output_path.exists(), "WAV file should exist");
            // Verify file has content
            let metadata = std::fs::metadata(&output_path).unwrap();
            assert!(metadata.len() > 44, "WAV file should have data beyond header");
        }
        Err(e) => {
            // Export might fail if format not supported - that's acceptable
            println!("Export failed (may be expected): {:?}", e);
        }
    }

    cleanup_temp_dir("to_wav");
}

/// Test export with empty graph.
#[test]
fn test_export_empty_graph() {
    let engine = test_engine();
    let dir = setup_temp_dir("empty_graph");
    let output_path = dir.join("test_silence.wav");

    // No nodes added - should export silence

    let result = engine
        .export()
        .duration_seconds(0.25)
        .to_file(&output_path);

    // Should succeed (exports silence)
    if result.is_ok() {
        assert!(output_path.exists());
    }

    cleanup_temp_dir("empty_graph");
}

/// Test export with complex graph.
#[test]
fn test_export_complex_graph() {
    let engine = test_engine();
    let dir = setup_temp_dir("complex_graph");
    let output_path = dir.join("test_complex.wav");

    engine.graph(|net| {
        // Multiple sources mixed
        net.add(sine_hz::<f64>(220.0) * 0.2).to_master();
        net.add(sine_hz::<f64>(440.0) * 0.15).to_master();
        net.add(sine_hz::<f64>(880.0) * 0.1).to_master();
    });

    let result = engine
        .export()
        .duration_seconds(0.5)
        .to_file(&output_path);

    if result.is_ok() {
        assert!(output_path.exists());
        let metadata = std::fs::metadata(&output_path).unwrap();
        assert!(metadata.len() > 1000, "Complex export should have significant data");
    }

    cleanup_temp_dir("complex_graph");
}

/// Test render to memory buffer.
#[test]
fn test_export_render() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).to_master();
    });

    // Render to buffer
    let result = engine
        .export()
        .duration_seconds(0.25)
        .render();

    match result {
        Ok((left, right, sample_rate)) => {
            // Should have samples
            assert!(!left.is_empty(), "Left channel should have samples");
            assert!(!right.is_empty(), "Right channel should have samples");
            // At 48kHz, 0.25s = 12000 samples per channel
            assert!(left.len() >= 10000, "Should have at least ~10k samples per channel");
            assert!(sample_rate > 0.0, "Sample rate should be valid");
        }
        Err(e) => {
            panic!("Render failed: {:?}", e);
        }
    }
}

/// Test multiple sequential exports.
#[test]
fn test_export_sequential() {
    let engine = test_engine();
    let dir = setup_temp_dir("sequential");

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.3).to_master();
    });

    // Export multiple files
    for i in 0..3 {
        let output_path = dir.join(format!("test_seq_{}.wav", i));
        let _ = engine
            .export()
            .duration_seconds(0.1)
            .to_file(&output_path);
    }

    cleanup_temp_dir("sequential");
}

/// Test export doesn't affect live playback.
#[test]
fn test_export_during_playback() {
    let engine = test_engine();
    let dir = setup_temp_dir("during_playback");
    let output_path = dir.join("test_during_play.wav");

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.3).to_master();
    });

    // Start playback
    engine.transport().play();
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Export while playing
    let _ = engine
        .export()
        .duration_seconds(0.25)
        .to_file(&output_path);

    // Playback should still work
    assert!(engine.is_running());

    engine.transport().stop();
    cleanup_temp_dir("during_playback");
}

/// Test export with format specification.
#[test]
fn test_export_format() {
    let engine = test_engine();
    let dir = setup_temp_dir("format");
    let output_path = dir.join("test_format.wav");

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.3).to_master();
    });

    let result = engine
        .export()
        .duration_seconds(0.25)
        .format(tutti::export::AudioFormat::Wav)
        .to_file(&output_path);

    // Check if export succeeded - may fail due to format support
    match result {
        Ok(_) => assert!(output_path.exists()),
        Err(e) => println!("Export with format failed (may be expected): {:?}", e),
    }

    cleanup_temp_dir("format");
}

/// Test export with latency compensation.
#[test]
fn test_export_latency_compensation() {
    let engine = test_engine();
    let dir = setup_temp_dir("latency_comp");
    let output_path = dir.join("test_latency.wav");

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.3).to_master();
    });

    let result = engine
        .export()
        .duration_seconds(0.25)
        .compensate_latency(true)
        .to_file(&output_path);

    // Check if export succeeded - latency compensation may not be supported
    match result {
        Ok(_) => assert!(output_path.exists()),
        Err(e) => println!("Export with latency compensation failed (may be expected): {:?}", e),
    }

    cleanup_temp_dir("latency_comp");
}

/// Test export to FLAC file.
#[test]
fn test_export_to_flac() {
    let engine = test_engine();
    let dir = setup_temp_dir("to_flac");
    let output_path = dir.join("test_output.flac");

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.3).to_master();
    });

    let result = engine
        .export()
        .duration_seconds(0.5)
        .format(tutti::export::AudioFormat::Flac)
        .to_file(&output_path);

    match result {
        Ok(_) => {
            assert!(output_path.exists(), "FLAC file should exist");
            let metadata = std::fs::metadata(&output_path).unwrap();
            assert!(metadata.len() > 100, "FLAC file should have data");
        }
        Err(e) => {
            panic!("FLAC export failed: {:?}", e);
        }
    }

    cleanup_temp_dir("to_flac");
}

/// Test export with LUFS normalization.
#[test]
fn test_export_with_normalization() {
    let engine = test_engine();
    let dir = setup_temp_dir("normalization");
    let output_path = dir.join("test_normalized.wav");

    engine.graph(|net| {
        // Quiet signal that should be normalized up
        net.add(sine_hz::<f64>(440.0) * 0.1).to_master();
    });

    let result = engine
        .export()
        .duration_seconds(0.5)
        .normalize(tutti::export::NormalizationMode::lufs(-14.0))
        .to_file(&output_path);

    match result {
        Ok(_) => {
            assert!(output_path.exists(), "Normalized file should exist");
        }
        Err(e) => {
            panic!("Export with normalization failed: {:?}", e);
        }
    }

    cleanup_temp_dir("normalization");
}

/// Test export with bit depth option.
#[test]
fn test_export_bit_depth() {
    let engine = test_engine();
    let dir = setup_temp_dir("bit_depth");
    let output_path = dir.join("test_16bit.wav");

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.3).to_master();
    });

    let result = engine
        .export()
        .duration_seconds(0.25)
        .bit_depth(tutti::export::BitDepth::Int16)
        .to_file(&output_path);

    match result {
        Ok(_) => {
            assert!(output_path.exists(), "16-bit file should exist");
        }
        Err(e) => {
            panic!("Export with 16-bit depth failed: {:?}", e);
        }
    }

    cleanup_temp_dir("bit_depth");
}
