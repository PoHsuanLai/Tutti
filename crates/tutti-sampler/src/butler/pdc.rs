//! Plugin delay compensation (PDC) integration for butler thread.

use super::cache::LruCache;
use super::config::BufferConfig;
use super::loops::{capture_fadein_samples, capture_fadeout_samples};
use super::metrics::IOMetrics;
use super::prefetch::RegionBufferProducer;
use super::request::RegionId;
use super::stream_state::ChannelStreamState;
use dashmap::DashMap;
use std::sync::Arc;
use tutti_core::PdcManager;

/// Check for PDC changes and apply updates to streams.
///
/// Called each refill cycle to detect when plugin latencies have changed
/// and adjust stream positions accordingly with smooth crossfades.
#[allow(clippy::too_many_arguments)]
pub(super) fn check_pdc_updates(
    pdc_manager: &Option<Arc<PdcManager>>,
    stream_states: &DashMap<usize, ChannelStreamState>,
    producers: &mut [RegionBufferProducer],
    producer_index: &std::collections::HashMap<RegionId, usize>,
    sample_cache: &LruCache,
    metrics: &IOMetrics,
    config: &BufferConfig,
) {
    let Some(pdc) = pdc_manager.as_ref() else {
        return;
    };

    if !pdc.is_enabled() {
        return;
    }

    let snapshot = pdc.get_snapshot();

    for mut entry in stream_states.iter_mut() {
        let channel_index = *entry.key();
        let stream_state = entry.value_mut();

        let current_preroll = stream_state.pdc_preroll();
        let new_preroll = snapshot
            .channel_compensations()
            .get(channel_index)
            .copied()
            .unwrap_or(0) as u64;

        if new_preroll == current_preroll {
            continue;
        }

        let Some(region_id) = stream_state.region_id() else {
            continue;
        };

        let Some(&idx) = producer_index.get(&region_id) else {
            continue;
        };
        let producer = &mut producers[idx];

        let current_pos = producer.file_position();
        let new_pos = if new_preroll > current_preroll {
            current_pos.saturating_sub(new_preroll - current_preroll)
        } else {
            current_pos + (current_preroll - new_preroll)
        };

        let crossfade_len = config.seek_crossfade_samples;
        let fadeout = capture_fadeout_samples(stream_state, crossfade_len);

        stream_state.set_seeking(true);
        stream_state.flush_buffer();
        producer.set_file_position(new_pos);

        let fadein = capture_fadein_samples(
            sample_cache,
            metrics,
            producer.file_path(),
            new_pos,
            crossfade_len,
        );

        if !fadeout.is_empty() && !fadein.is_empty() {
            stream_state
                .shared_state()
                .start_seek_crossfade(fadeout, fadein);
        }

        stream_state.set_pdc_preroll(new_preroll);
        stream_state.set_seeking(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::butler::prefetch::RegionBuffer;
    use std::path::PathBuf;

    fn create_test_fixtures(
        channel_count: usize,
    ) -> (
        DashMap<usize, ChannelStreamState>,
        Vec<RegionBufferProducer>,
        std::collections::HashMap<RegionId, usize>,
        LruCache,
        IOMetrics,
        BufferConfig,
    ) {
        let stream_states = DashMap::new();
        let mut producers = Vec::new();
        let mut producer_index = std::collections::HashMap::new();

        for i in 0..channel_count {
            let region_id = RegionId::generate();
            let (producer, consumer) =
                RegionBuffer::with_capacity(region_id, PathBuf::from("test.wav"), 4096);

            producer_index.insert(region_id, producers.len());
            producers.push(producer);

            let mut state = ChannelStreamState::default();
            state.start_streaming(Arc::new(parking_lot::Mutex::new(consumer)));
            stream_states.insert(i, state);
        }

        let cache = LruCache::new(10, 1024 * 1024);
        let metrics = IOMetrics::new();
        let config = BufferConfig::default();

        (
            stream_states,
            producers,
            producer_index,
            cache,
            metrics,
            config,
        )
    }

    #[test]
    fn test_no_pdc_manager_is_noop() {
        let (stream_states, mut producers, producer_index, cache, metrics, config) =
            create_test_fixtures(1);

        // Set initial file position
        producers[0].set_file_position(1000);

        // Call with None PDC manager
        check_pdc_updates(
            &None,
            &stream_states,
            &mut producers,
            &producer_index,
            &cache,
            &metrics,
            &config,
        );

        // Position should be unchanged
        assert_eq!(producers[0].file_position(), 1000);
    }

    #[test]
    fn test_pdc_disabled_is_noop() {
        let (stream_states, mut producers, producer_index, cache, metrics, config) =
            create_test_fixtures(1);

        let pdc = Arc::new(PdcManager::new(4, 0));
        pdc.set_enabled(false);
        pdc.set_channel_latency(0, 500); // This would require compensation

        producers[0].set_file_position(1000);

        check_pdc_updates(
            &Some(pdc),
            &stream_states,
            &mut producers,
            &producer_index,
            &cache,
            &metrics,
            &config,
        );

        // Position should be unchanged because PDC is disabled
        assert_eq!(producers[0].file_position(), 1000);
    }

    #[test]
    fn test_preroll_unchanged_no_seek() {
        let (stream_states, mut producers, producer_index, cache, metrics, config) =
            create_test_fixtures(1);

        let pdc = Arc::new(PdcManager::new(4, 0));
        // No latency set, so compensation is 0

        producers[0].set_file_position(1000);

        // Stream state starts with pdc_preroll = 0, and PDC compensation is also 0
        check_pdc_updates(
            &Some(pdc),
            &stream_states,
            &mut producers,
            &producer_index,
            &cache,
            &metrics,
            &config,
        );

        // Position should be unchanged (preroll didn't change)
        assert_eq!(producers[0].file_position(), 1000);
    }

    #[test]
    fn test_preroll_increased_seeks_backward() {
        let (stream_states, mut producers, producer_index, cache, metrics, config) =
            create_test_fixtures(2);

        let pdc = Arc::new(PdcManager::new(4, 0));

        // Set up: channel 0 has no latency, channel 1 has 500 sample latency
        // This means channel 0 needs 500 samples of compensation (read earlier)
        pdc.set_channel_latency(1, 500);

        // Set initial positions
        producers[0].set_file_position(1000);
        producers[1].set_file_position(1000);

        // First call - this sets the preroll for channel 0
        check_pdc_updates(
            &Some(pdc.clone()),
            &stream_states,
            &mut producers,
            &producer_index,
            &cache,
            &metrics,
            &config,
        );

        // Channel 0: new_preroll=500 (compensation), current_preroll=0
        // new_preroll > current_preroll, so seek backward by 500
        // new_pos = 1000 - 500 = 500
        assert_eq!(producers[0].file_position(), 500);

        // Channel 1: new_preroll=0 (has the max latency), current_preroll=0
        // No change expected
        assert_eq!(producers[1].file_position(), 1000);

        // Verify preroll was updated
        assert_eq!(stream_states.get(&0).unwrap().pdc_preroll(), 500);
        assert_eq!(stream_states.get(&1).unwrap().pdc_preroll(), 0);
    }

    #[test]
    fn test_preroll_decreased_seeks_forward() {
        let (stream_states, mut producers, producer_index, cache, metrics, config) =
            create_test_fixtures(1);

        let pdc = Arc::new(PdcManager::new(4, 0));

        // Set up initial state with high latency on another channel
        pdc.set_channel_latency(1, 500);
        producers[0].set_file_position(1000);

        // First call to establish the preroll
        check_pdc_updates(
            &Some(pdc.clone()),
            &stream_states,
            &mut producers,
            &producer_index,
            &cache,
            &metrics,
            &config,
        );

        // Channel 0 should now have preroll=500 and position=500
        assert_eq!(stream_states.get(&0).unwrap().pdc_preroll(), 500);
        assert_eq!(producers[0].file_position(), 500);

        // Now remove the latency from channel 1, reducing compensation needed
        pdc.remove_channel(1);

        // Second call - preroll should decrease
        check_pdc_updates(
            &Some(pdc),
            &stream_states,
            &mut producers,
            &producer_index,
            &cache,
            &metrics,
            &config,
        );

        // new_preroll=0, current_preroll=500
        // new_preroll < current_preroll, so seek forward by 500
        // new_pos = 500 + 500 = 1000
        assert_eq!(producers[0].file_position(), 1000);
        assert_eq!(stream_states.get(&0).unwrap().pdc_preroll(), 0);
    }

    #[test]
    fn test_seeking_flag_set_during_update() {
        let (stream_states, mut producers, producer_index, cache, metrics, config) =
            create_test_fixtures(1);

        let pdc = Arc::new(PdcManager::new(4, 0));
        pdc.set_channel_latency(1, 500); // Causes channel 0 to need compensation

        producers[0].set_file_position(1000);

        // Before update, seeking should be false
        assert!(!stream_states.get(&0).unwrap().shared_state().is_seeking());

        check_pdc_updates(
            &Some(pdc),
            &stream_states,
            &mut producers,
            &producer_index,
            &cache,
            &metrics,
            &config,
        );

        // After update completes, seeking should be false again
        assert!(!stream_states.get(&0).unwrap().shared_state().is_seeking());
    }

    #[test]
    fn test_multiple_channels_independent_compensation() {
        let (stream_states, mut producers, producer_index, cache, metrics, config) =
            create_test_fixtures(3);

        let pdc = Arc::new(PdcManager::new(4, 0));

        // Channel 0: 100 latency -> 200 compensation (max is 300)
        // Channel 1: 300 latency -> 0 compensation (is max)
        // Channel 2: 200 latency -> 100 compensation
        pdc.set_channel_latency(0, 100);
        pdc.set_channel_latency(1, 300);
        pdc.set_channel_latency(2, 200);

        producers[0].set_file_position(1000);
        producers[1].set_file_position(1000);
        producers[2].set_file_position(1000);

        check_pdc_updates(
            &Some(pdc),
            &stream_states,
            &mut producers,
            &producer_index,
            &cache,
            &metrics,
            &config,
        );

        // Channel 0: compensation=200, seeks back 200
        assert_eq!(producers[0].file_position(), 800);
        assert_eq!(stream_states.get(&0).unwrap().pdc_preroll(), 200);

        // Channel 1: compensation=0, no seek
        assert_eq!(producers[1].file_position(), 1000);
        assert_eq!(stream_states.get(&1).unwrap().pdc_preroll(), 0);

        // Channel 2: compensation=100, seeks back 100
        assert_eq!(producers[2].file_position(), 900);
        assert_eq!(stream_states.get(&2).unwrap().pdc_preroll(), 100);
    }

    #[test]
    fn test_channel_not_in_pdc_snapshot() {
        let (stream_states, mut producers, producer_index, cache, metrics, config) =
            create_test_fixtures(1);

        // Create PDC with only 0 channels (empty)
        let pdc = Arc::new(PdcManager::new(0, 0));

        producers[0].set_file_position(1000);

        check_pdc_updates(
            &Some(pdc),
            &stream_states,
            &mut producers,
            &producer_index,
            &cache,
            &metrics,
            &config,
        );

        // Channel 0 is not in the PDC snapshot, so new_preroll defaults to 0
        // current_preroll is also 0, so no change
        assert_eq!(producers[0].file_position(), 1000);
    }
}
