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
            .channel_compensations
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
