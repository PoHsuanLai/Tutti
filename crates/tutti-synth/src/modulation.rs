//! Modulation matrix for synthesizers.
//!
//! Routes modulation sources (LFOs, envelopes, MIDI CCs) to destinations
//! (pitch, filter, amplitude) with configurable amounts.
//!
//! All operations are RT-safe (no allocations in compute path).

/// Maximum number of modulation routes.
pub const MAX_MOD_ROUTES: usize = 32;

/// Maximum number of LFOs.
pub const MAX_LFOS: usize = 8;

/// Maximum number of envelopes.
pub const MAX_ENVELOPES: usize = 4;

/// Modulation source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModSource {
    /// LFO output (index 0-7)
    Lfo(u8),
    /// Envelope output (index 0-3)
    Envelope(u8),
    /// Note velocity (0.0-1.0)
    Velocity,
    /// Channel aftertouch (0.0-1.0)
    Aftertouch,
    /// Mod wheel CC1 (0.0-1.0)
    ModWheel,
    /// Pitch bend (-1.0 to 1.0)
    PitchBend,
    /// Expression CC11 (0.0-1.0)
    Expression,
    /// Breath controller CC2 (0.0-1.0)
    Breath,
    /// Generic MIDI CC (0-127)
    CC(u8),
    /// Key tracking (note number normalized, middle C = 0.5)
    KeyTrack,
    /// Random value (generated on note-on)
    Random,
}

/// Modulation destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModDestination {
    /// Pitch in semitones
    Pitch,
    /// Filter cutoff (0.0-1.0 normalized)
    FilterCutoff,
    /// Filter resonance/Q (0.0-1.0)
    FilterQ,
    /// Amplitude/volume (0.0-1.0)
    Amplitude,
    /// Pan position (-1.0 to 1.0)
    Pan,
    /// Oscillator mix (0.0-1.0)
    OscMix,
    /// Pulse width (0.0-1.0)
    PulseWidth,
    /// LFO rate (0.0-1.0 normalized)
    LfoRate,
    /// LFO depth (0.0-1.0)
    LfoDepth,
    /// Envelope attack time (0.0-1.0)
    EnvAttack,
    /// Envelope decay time (0.0-1.0)
    EnvDecay,
    /// Envelope sustain level (0.0-1.0)
    EnvSustain,
    /// Envelope release time (0.0-1.0)
    EnvRelease,
}

/// A single modulation routing.
#[derive(Debug, Clone, Copy)]
pub struct ModRoute {
    /// Source of modulation
    pub source: ModSource,
    /// Destination parameter
    pub destination: ModDestination,
    /// Modulation amount (-1.0 to 1.0)
    pub amount: f32,
    /// If true, source is bipolar (-1 to 1). If false, unipolar (0 to 1).
    pub bipolar: bool,
}

impl ModRoute {
    /// Create a new modulation route.
    pub fn new(source: ModSource, destination: ModDestination, amount: f32) -> Self {
        Self {
            source,
            destination,
            amount,
            bipolar: false,
        }
    }

    /// Set this route as bipolar.
    pub fn bipolar(mut self) -> Self {
        self.bipolar = true;
        self
    }

    /// Set this route as unipolar.
    pub fn unipolar(mut self) -> Self {
        self.bipolar = false;
        self
    }

    /// Set the modulation amount.
    pub fn with_amount(mut self, amount: f32) -> Self {
        self.amount = amount;
        self
    }
}

/// Input values from all modulation sources.
#[derive(Debug, Clone)]
pub struct ModSourceValues {
    /// LFO outputs (0-7)
    pub lfo: [f32; MAX_LFOS],
    /// Envelope outputs (0-3)
    pub envelope: [f32; MAX_ENVELOPES],
    /// Note velocity
    pub velocity: f32,
    /// Channel aftertouch
    pub aftertouch: f32,
    /// Mod wheel (CC1)
    pub mod_wheel: f32,
    /// Pitch bend (-1.0 to 1.0)
    pub pitch_bend: f32,
    /// Expression (CC11)
    pub expression: f32,
    /// Breath controller (CC2)
    pub breath: f32,
    /// All 128 MIDI CCs
    pub cc: [f32; 128],
    /// Key tracking (note / 127)
    pub key_track: f32,
    /// Random value (0.0-1.0)
    pub random: f32,
}

impl ModSourceValues {
    /// Get value for a specific source.
    #[inline]
    pub fn get(&self, source: ModSource) -> f32 {
        match source {
            ModSource::Lfo(i) => self.lfo[i as usize % MAX_LFOS],
            ModSource::Envelope(i) => self.envelope[i as usize % MAX_ENVELOPES],
            ModSource::Velocity => self.velocity,
            ModSource::Aftertouch => self.aftertouch,
            ModSource::ModWheel => self.mod_wheel,
            ModSource::PitchBend => self.pitch_bend,
            ModSource::Expression => self.expression,
            ModSource::Breath => self.breath,
            ModSource::CC(cc) => self.cc[cc as usize],
            ModSource::KeyTrack => self.key_track,
            ModSource::Random => self.random,
        }
    }

    /// Set key tracking from MIDI note number.
    pub fn set_key_track_from_note(&mut self, note: u8) {
        self.key_track = note as f32 / 127.0;
    }

    /// Generate new random value.
    pub fn randomize(&mut self, rng_state: &mut u32) {
        *rng_state ^= *rng_state << 13;
        *rng_state ^= *rng_state >> 17;
        *rng_state ^= *rng_state << 5;
        self.random = (*rng_state as f32) / (u32::MAX as f32);
    }
}

impl Default for ModSourceValues {
    fn default() -> Self {
        Self {
            lfo: [0.0; MAX_LFOS],
            envelope: [0.0; MAX_ENVELOPES],
            velocity: 0.0,
            aftertouch: 0.0,
            mod_wheel: 0.0,
            pitch_bend: 0.0,
            expression: 0.0,
            breath: 0.0,
            cc: [0.0; 128],
            key_track: 0.0,
            random: 0.0,
        }
    }
}

/// Output values for all modulation destinations.
#[derive(Debug, Clone, Default)]
pub struct ModDestinationValues {
    /// Pitch modulation in semitones
    pub pitch: f32,
    /// Filter cutoff modulation (additive, normalized)
    pub filter_cutoff: f32,
    /// Filter Q modulation
    pub filter_q: f32,
    /// Amplitude modulation
    pub amplitude: f32,
    /// Pan modulation
    pub pan: f32,
    /// Oscillator mix modulation
    pub osc_mix: f32,
    /// Pulse width modulation
    pub pulse_width: f32,
    /// LFO rate modulation
    pub lfo_rate: f32,
    /// LFO depth modulation
    pub lfo_depth: f32,
    /// Envelope attack modulation
    pub env_attack: f32,
    /// Envelope decay modulation
    pub env_decay: f32,
    /// Envelope sustain modulation
    pub env_sustain: f32,
    /// Envelope release modulation
    pub env_release: f32,
}

impl ModDestinationValues {
    /// Reset all values to zero.
    #[inline]
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Add modulation to a destination.
    #[inline]
    fn add(&mut self, dest: ModDestination, value: f32) {
        match dest {
            ModDestination::Pitch => self.pitch += value,
            ModDestination::FilterCutoff => self.filter_cutoff += value,
            ModDestination::FilterQ => self.filter_q += value,
            ModDestination::Amplitude => self.amplitude += value,
            ModDestination::Pan => self.pan += value,
            ModDestination::OscMix => self.osc_mix += value,
            ModDestination::PulseWidth => self.pulse_width += value,
            ModDestination::LfoRate => self.lfo_rate += value,
            ModDestination::LfoDepth => self.lfo_depth += value,
            ModDestination::EnvAttack => self.env_attack += value,
            ModDestination::EnvDecay => self.env_decay += value,
            ModDestination::EnvSustain => self.env_sustain += value,
            ModDestination::EnvRelease => self.env_release += value,
        }
    }
}

/// Configuration for modulation matrix.
#[derive(Debug, Clone, Default)]
pub struct ModulationMatrixConfig {
    /// Active modulation routes
    pub routes: Vec<ModRoute>,
}

/// Modulation matrix processor.
///
/// Computes destination values from source values using configured routes.
/// All operations are RT-safe (no allocations in compute path).
#[derive(Debug, Clone)]
pub struct ModulationMatrix {
    /// Fixed-size route storage
    routes: [Option<ModRoute>; MAX_MOD_ROUTES],
    /// Number of active routes
    route_count: usize,
    /// Cached output values
    destinations: ModDestinationValues,
}

impl ModulationMatrix {
    /// Create a new modulation matrix.
    pub fn new(config: ModulationMatrixConfig) -> Self {
        let mut matrix = Self {
            routes: [None; MAX_MOD_ROUTES],
            route_count: 0,
            destinations: ModDestinationValues::default(),
        };

        for (i, route) in config.routes.into_iter().take(MAX_MOD_ROUTES).enumerate() {
            matrix.routes[i] = Some(route);
            matrix.route_count = i + 1;
        }

        matrix
    }

    /// Compute destination values from source values.
    ///
    /// RT-safe: no allocations, bounded loops.
    #[inline]
    pub fn compute(&mut self, sources: &ModSourceValues) -> &ModDestinationValues {
        self.destinations.reset();

        for i in 0..self.route_count {
            if let Some(route) = &self.routes[i] {
                let source_value = sources.get(route.source);

                // Convert unipolar (0-1) to bipolar (-1 to 1) if needed
                let normalized = if route.bipolar {
                    source_value
                } else {
                    source_value * 2.0 - 1.0
                };

                let mod_value = normalized * route.amount;
                self.destinations.add(route.destination, mod_value);
            }
        }

        &self.destinations
    }

    /// Get current destination values without recomputing.
    #[inline]
    pub fn destinations(&self) -> &ModDestinationValues {
        &self.destinations
    }

    /// Add a modulation route.
    ///
    /// Returns false if matrix is full.
    pub fn add_route(&mut self, route: ModRoute) -> bool {
        if self.route_count >= MAX_MOD_ROUTES {
            return false;
        }

        self.routes[self.route_count] = Some(route);
        self.route_count += 1;
        true
    }

    /// Remove a modulation route by index.
    pub fn remove_route(&mut self, index: usize) {
        if index >= self.route_count {
            return;
        }

        // Shift routes down
        for i in index..self.route_count - 1 {
            self.routes[i] = self.routes[i + 1];
        }
        self.routes[self.route_count - 1] = None;
        self.route_count -= 1;
    }

    /// Clear all routes.
    pub fn clear(&mut self) {
        for route in &mut self.routes {
            *route = None;
        }
        self.route_count = 0;
    }

    /// Get the number of active routes.
    #[inline]
    pub fn route_count(&self) -> usize {
        self.route_count
    }

    /// Get a route by index.
    #[inline]
    pub fn get_route(&self, index: usize) -> Option<&ModRoute> {
        if index < self.route_count {
            self.routes[index].as_ref()
        } else {
            None
        }
    }

    /// Update a route's amount.
    pub fn set_route_amount(&mut self, index: usize, amount: f32) {
        if index < self.route_count {
            if let Some(route) = &mut self.routes[index] {
                route.amount = amount;
            }
        }
    }
}

impl Default for ModulationMatrix {
    fn default() -> Self {
        Self::new(ModulationMatrixConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_matrix() {
        let mut matrix = ModulationMatrix::default();
        let sources = ModSourceValues::default();

        let destinations = matrix.compute(&sources);

        assert!((destinations.pitch - 0.0).abs() < 0.001);
        assert!((destinations.filter_cutoff - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_single_route() {
        let config = ModulationMatrixConfig {
            routes: vec![ModRoute::new(
                ModSource::Lfo(0),
                ModDestination::Pitch,
                2.0, // 2 semitones
            )
            .bipolar()],
        };

        let mut matrix = ModulationMatrix::new(config);

        let mut sources = ModSourceValues::default();
        sources.lfo[0] = 1.0; // Max LFO value

        let destinations = matrix.compute(&sources);

        assert!((destinations.pitch - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_multiple_routes_same_destination() {
        let config = ModulationMatrixConfig {
            routes: vec![
                ModRoute::new(ModSource::Lfo(0), ModDestination::Pitch, 1.0).bipolar(),
                ModRoute::new(ModSource::Envelope(0), ModDestination::Pitch, 0.5).bipolar(),
            ],
        };

        let mut matrix = ModulationMatrix::new(config);

        let mut sources = ModSourceValues::default();
        sources.lfo[0] = 1.0;
        sources.envelope[0] = 1.0;

        let destinations = matrix.compute(&sources);

        // Both should add up
        assert!((destinations.pitch - 1.5).abs() < 0.001);
    }

    #[test]
    fn test_unipolar_source() {
        let config = ModulationMatrixConfig {
            routes: vec![
                ModRoute::new(ModSource::Velocity, ModDestination::FilterCutoff, 1.0).unipolar(),
            ],
        };

        let mut matrix = ModulationMatrix::new(config);

        let mut sources = ModSourceValues::default();
        sources.velocity = 1.0; // Full velocity

        let destinations = matrix.compute(&sources);

        // Unipolar 1.0 -> bipolar 1.0, * amount 1.0 = 1.0
        assert!((destinations.filter_cutoff - 1.0).abs() < 0.001);

        // Half velocity
        sources.velocity = 0.5;
        let destinations = matrix.compute(&sources);

        // Unipolar 0.5 -> bipolar 0.0, * amount 1.0 = 0.0
        assert!((destinations.filter_cutoff - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_cc_source() {
        let config = ModulationMatrixConfig {
            routes: vec![
                ModRoute::new(ModSource::CC(74), ModDestination::FilterCutoff, 1.0).bipolar(),
            ],
        };

        let mut matrix = ModulationMatrix::new(config);

        let mut sources = ModSourceValues::default();
        sources.cc[74] = 0.75;

        let destinations = matrix.compute(&sources);

        assert!((destinations.filter_cutoff - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_add_remove_routes() {
        let mut matrix = ModulationMatrix::default();

        assert_eq!(matrix.route_count(), 0);

        matrix.add_route(ModRoute::new(ModSource::Lfo(0), ModDestination::Pitch, 1.0));
        assert_eq!(matrix.route_count(), 1);

        matrix.add_route(ModRoute::new(
            ModSource::Lfo(1),
            ModDestination::FilterCutoff,
            0.5,
        ));
        assert_eq!(matrix.route_count(), 2);

        matrix.remove_route(0);
        assert_eq!(matrix.route_count(), 1);

        // Remaining route should be the filter one
        let route = matrix.get_route(0).unwrap();
        assert_eq!(route.destination, ModDestination::FilterCutoff);
    }

    #[test]
    fn test_route_amount_update() {
        let config = ModulationMatrixConfig {
            routes: vec![ModRoute::new(ModSource::Lfo(0), ModDestination::Pitch, 1.0).bipolar()],
        };

        let mut matrix = ModulationMatrix::new(config);

        let mut sources = ModSourceValues::default();
        sources.lfo[0] = 1.0;

        let destinations = matrix.compute(&sources);
        assert!((destinations.pitch - 1.0).abs() < 0.001);

        matrix.set_route_amount(0, 0.5);

        let destinations = matrix.compute(&sources);
        assert!((destinations.pitch - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_key_tracking() {
        let config = ModulationMatrixConfig {
            routes: vec![
                ModRoute::new(ModSource::KeyTrack, ModDestination::FilterCutoff, 1.0).bipolar(),
            ],
        };

        let mut matrix = ModulationMatrix::new(config);
        let mut sources = ModSourceValues::default();

        // Low note
        sources.set_key_track_from_note(24);
        let destinations = matrix.compute(&sources);
        let low_cutoff = destinations.filter_cutoff;

        // High note
        sources.set_key_track_from_note(96);
        let destinations = matrix.compute(&sources);
        let high_cutoff = destinations.filter_cutoff;

        assert!(high_cutoff > low_cutoff);
    }

    #[test]
    fn test_max_routes() {
        let mut matrix = ModulationMatrix::default();

        for _ in 0..MAX_MOD_ROUTES {
            assert!(matrix.add_route(ModRoute::new(ModSource::Lfo(0), ModDestination::Pitch, 0.1,)));
        }

        // Should fail at max
        assert!(!matrix.add_route(ModRoute::new(ModSource::Lfo(0), ModDestination::Pitch, 0.1,)));
    }
}
