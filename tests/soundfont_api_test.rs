//! Integration tests for SoundFont fluent API

#[cfg(feature = "soundfont")]
mod soundfont_tests {
    use tutti::prelude::*;

    #[test]
    fn test_soundfont_load_api() {
        let engine = TuttiEngine::builder()
            .sample_rate(48000.0)
            .build()
            .unwrap();

        // Note: This test documents the API but won't actually load a file
        // In a real scenario, you'd provide a valid .sf2 file path
        let result = engine.load_sf2("piano", "test.sf2");

        // We expect this to fail because test.sf2 doesn't exist
        // But the API is correctly structured
        assert!(result.is_err());
    }

    #[test]
    fn test_soundfont_instance_api() {
        let engine = TuttiEngine::builder()
            .sample_rate(48000.0)
            .build()
            .unwrap();

        // If we had a valid .sf2 file, we could:
        // 1. Load it: engine.load_sf2("piano", "path/to/piano.sf2")?;
        // 2. Instantiate with default settings:
        //    let piano1 = engine.instance("piano", &params! {})?;
        // 3. Instantiate with specific preset:
        //    let piano2 = engine.instance("piano", &params! {
        //        "preset" => 0,
        //        "channel" => 0
        //    })?;
        // 4. Use in audio graph:
        //    engine.graph(|net| {
        //        chain!(net, piano1 => output);
        //    });

        // For now, just verify the engine was created successfully
        assert_eq!(engine.sample_rate(), 48000.0);
    }
}
