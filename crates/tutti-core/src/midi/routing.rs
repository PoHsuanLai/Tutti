//! RT-safe MIDI routing system for channel-based, port-based, and layered routing.
//!
//! This module provides a lock-free routing table that maps MIDI events to target
//! audio units based on channel, port, or custom routing rules.
//!
//! # Architecture
//!
//! ```text
//! UI Thread                          Audio Thread
//!     │                                   │
//!     ▼                                   ▼
//! ┌─────────────────┐              ┌─────────────────────┐
//! │ MidiRoutingTable│──ArcSwap────▶│ MidiRoutingSnapshot │
//! │   (RwLock)      │              │   (immutable)       │
//! │ - routes[]      │              │ - channel_to_units  │
//! │ - modify()      │              │ - route()           │
//! └─────────────────┘              └─────────────────────┘
//! ```
//!
//! # RT Safety
//!
//! - `MidiRoutingSnapshot::route()`: Called on the audio thread. Zero allocations,
//!   returns an iterator over target unit IDs.
//! - `MidiRoutingTable::commit()`: Called from the UI thread. Atomically swaps
//!   the snapshot using `arc_swap::ArcSwap`.

use crate::compat::{Arc, Vec};
use arc_swap::ArcSwap;

use tutti_midi::MidiEvent;

/// Maximum number of targets per routing rule.
/// Supports layering up to 8 synths on a single channel.
const MAX_TARGETS_PER_ROUTE: usize = 8;

/// A single MIDI routing rule.
///
/// Routes can filter by port, channel, or both. Events matching the filter
/// are sent to all target units.
#[derive(Clone, Debug)]
pub struct MidiRoute {
    /// Port filter: `None` = any port, `Some(n)` = port n only
    pub port: Option<usize>,
    /// Channel filter: `None` = any channel, `Some(n)` = channel n only (0-15)
    pub channel: Option<u8>,
    /// Target unit IDs to receive matching events
    pub targets: Vec<u64>,
    /// Whether this route is enabled
    pub enabled: bool,
}

impl MidiRoute {
    pub fn new() -> Self {
        Self {
            port: None,
            channel: None,
            targets: Vec::new(),
            enabled: true,
        }
    }

    pub fn for_channel(channel: u8) -> Self {
        Self {
            port: None,
            channel: Some(channel),
            targets: Vec::new(),
            enabled: true,
        }
    }

    pub fn for_port(port: usize) -> Self {
        Self {
            port: Some(port),
            channel: None,
            targets: Vec::new(),
            enabled: true,
        }
    }

    pub fn for_port_channel(port: usize, channel: u8) -> Self {
        Self {
            port: Some(port),
            channel: Some(channel),
            targets: Vec::new(),
            enabled: true,
        }
    }

    pub fn with_target(mut self, unit_id: u64) -> Self {
        if self.targets.len() < MAX_TARGETS_PER_ROUTE {
            self.targets.push(unit_id);
        }
        self
    }

    pub fn with_targets(mut self, unit_ids: &[u64]) -> Self {
        for &id in unit_ids {
            if self.targets.len() >= MAX_TARGETS_PER_ROUTE {
                break;
            }
            self.targets.push(id);
        }
        self
    }

    #[inline]
    pub fn matches(&self, port: usize, event: &MidiEvent) -> bool {
        if !self.enabled {
            return false;
        }
        let port_matches = self.port.is_none_or(|p| p == port);
        let channel_matches = self.channel.is_none_or(|c| c == event.channel_num());
        port_matches && channel_matches
    }
}

impl Default for MidiRoute {
    fn default() -> Self {
        Self::new()
    }
}

/// Immutable snapshot of routing configuration for RT-safe access.
///
/// Precomputes channel→units lookup for O(1) channel routing.
/// Created by `MidiRoutingTable::commit()` and loaded atomically on the audio thread.
#[derive(Clone, Debug)]
pub struct MidiRoutingSnapshot {
    /// All routing rules
    routes: Vec<MidiRoute>,
    /// Precomputed channel→targets lookup (17 entries: 0-15 + "any channel")
    /// Index 16 is for routes that match any channel.
    channel_lookup: [Vec<u64>; 17],
    /// Fallback target when no routes match (backwards compatibility)
    fallback_target: Option<u64>,
}

impl MidiRoutingSnapshot {
    pub fn empty() -> Self {
        Self {
            routes: Vec::new(),
            channel_lookup: Default::default(),
            fallback_target: None,
        }
    }

    fn from_routes(routes: Vec<MidiRoute>, fallback: Option<u64>) -> Self {
        let mut snapshot = Self {
            routes,
            channel_lookup: Default::default(),
            fallback_target: fallback,
        };
        snapshot.rebuild_lookup();
        snapshot
    }

    fn rebuild_lookup(&mut self) {
        // Clear existing lookup
        for lookup in self.channel_lookup.iter_mut() {
            lookup.clear();
        }

        // Build lookup from routes
        for route in &self.routes {
            if !route.enabled {
                continue;
            }
            // Only add to lookup if route has no port filter (port-filtered routes
            // must go through the full route matching path)
            if route.port.is_some() {
                continue;
            }

            let channel_idx = route.channel.map_or(16, |c| c as usize);
            for &target in &route.targets {
                if !self.channel_lookup[channel_idx].contains(&target) {
                    self.channel_lookup[channel_idx].push(target);
                }
            }
        }
    }

    /// RT-safe. Returns an iterator over target unit IDs. Zero allocations.
    #[inline]
    pub fn route<'a>(&'a self, port: usize, event: &'a MidiEvent) -> RouteIterator<'a> {
        RouteIterator {
            snapshot: self,
            port,
            event,
            phase: RoutePhase::ChannelLookup,
            route_idx: 0,
            target_idx: 0,
            seen: [0u64; 16], // Track seen targets to avoid duplicates
            seen_count: 0,
        }
    }

    /// Returns the first matching target, or the fallback if none match.
    #[inline]
    pub fn route_single(&self, port: usize, event: &MidiEvent) -> Option<u64> {
        // Fast path: check channel lookup first
        let channel = event.channel_num() as usize;

        // Check specific channel targets
        if let Some(&target) = self.channel_lookup[channel].first() {
            return Some(target);
        }

        // Check "any channel" targets
        if let Some(&target) = self.channel_lookup[16].first() {
            return Some(target);
        }

        // Check port-filtered routes
        for route in &self.routes {
            if route.matches(port, event) {
                if let Some(&target) = route.targets.first() {
                    return Some(target);
                }
            }
        }

        // Fallback
        self.fallback_target
    }

    #[inline]
    pub fn has_routes(&self) -> bool {
        !self.routes.is_empty() || self.fallback_target.is_some()
    }

    #[inline]
    pub fn fallback(&self) -> Option<u64> {
        self.fallback_target
    }
}

impl Default for MidiRoutingSnapshot {
    fn default() -> Self {
        Self::empty()
    }
}

/// Phase of route iteration.
#[derive(Clone, Copy, Debug)]
enum RoutePhase {
    /// Checking precomputed channel lookup
    ChannelLookup,
    /// Checking "any channel" lookup
    AnyChannelLookup,
    /// Iterating through port-filtered routes
    PortRoutes,
    /// Checking fallback target
    Fallback,
    /// Done iterating
    Done,
}

/// RT-safe iterator over routing targets.
///
/// Zero allocations - uses stack-allocated seen buffer.
pub struct RouteIterator<'a> {
    snapshot: &'a MidiRoutingSnapshot,
    port: usize,
    event: &'a MidiEvent,
    phase: RoutePhase,
    route_idx: usize,
    target_idx: usize,
    seen: [u64; 16], // Small buffer for deduplication
    seen_count: usize,
}

impl<'a> RouteIterator<'a> {
    #[inline]
    fn is_seen(&self, target: u64) -> bool {
        self.seen[..self.seen_count].contains(&target)
    }

    #[inline]
    fn mark_seen(&mut self, target: u64) {
        if self.seen_count < self.seen.len() {
            self.seen[self.seen_count] = target;
            self.seen_count += 1;
        }
    }
}

impl Iterator for RouteIterator<'_> {
    type Item = u64;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.phase {
                RoutePhase::ChannelLookup => {
                    let channel = self.event.channel_num() as usize;
                    let targets = &self.snapshot.channel_lookup[channel];
                    while self.target_idx < targets.len() {
                        let target = targets[self.target_idx];
                        self.target_idx += 1;
                        if !self.is_seen(target) {
                            self.mark_seen(target);
                            return Some(target);
                        }
                    }
                    self.target_idx = 0;
                    self.phase = RoutePhase::AnyChannelLookup;
                }
                RoutePhase::AnyChannelLookup => {
                    let targets = &self.snapshot.channel_lookup[16];
                    while self.target_idx < targets.len() {
                        let target = targets[self.target_idx];
                        self.target_idx += 1;
                        if !self.is_seen(target) {
                            self.mark_seen(target);
                            return Some(target);
                        }
                    }
                    self.target_idx = 0;
                    self.phase = RoutePhase::PortRoutes;
                }
                RoutePhase::PortRoutes => {
                    while self.route_idx < self.snapshot.routes.len() {
                        let route = &self.snapshot.routes[self.route_idx];
                        // Only process port-filtered routes (non-port routes are in lookup)
                        if route.port.is_some() && route.matches(self.port, self.event) {
                            while self.target_idx < route.targets.len() {
                                let target = route.targets[self.target_idx];
                                self.target_idx += 1;
                                if !self.is_seen(target) {
                                    self.mark_seen(target);
                                    return Some(target);
                                }
                            }
                        }
                        self.target_idx = 0;
                        self.route_idx += 1;
                    }
                    self.phase = RoutePhase::Fallback;
                }
                RoutePhase::Fallback => {
                    self.phase = RoutePhase::Done;
                    // Only use fallback if no routes matched
                    if self.seen_count == 0 {
                        if let Some(target) = self.snapshot.fallback_target {
                            return Some(target);
                        }
                    }
                }
                RoutePhase::Done => {
                    return None;
                }
            }
        }
    }
}

/// Mutable routing table for configuration from the UI thread.
///
/// Changes are staged and then atomically committed via `commit()`.
pub struct MidiRoutingTable {
    /// Current routes (mutable, UI thread only)
    routes: Vec<MidiRoute>,
    /// Fallback target for backwards compatibility
    fallback_target: Option<u64>,
    /// Atomic snapshot for RT access
    snapshot: Arc<ArcSwap<MidiRoutingSnapshot>>,
    /// Track if changes are pending
    dirty: bool,
}

impl MidiRoutingTable {
    pub fn new() -> Self {
        let snapshot = MidiRoutingSnapshot::empty();
        Self {
            routes: Vec::new(),
            fallback_target: None,
            snapshot: Arc::new(ArcSwap::from_pointee(snapshot)),
            dirty: false,
        }
    }

    pub fn snapshot_arc(&self) -> Arc<ArcSwap<MidiRoutingSnapshot>> {
        self.snapshot.clone()
    }

    #[inline]
    pub fn load(&self) -> arc_swap::Guard<Arc<MidiRoutingSnapshot>> {
        self.snapshot.load()
    }

    pub fn fallback(&mut self, target: u64) -> &mut Self {
        self.fallback_target = Some(target);
        self.dirty = true;
        self
    }

    pub fn clear_fallback(&mut self) -> &mut Self {
        self.fallback_target = None;
        self.dirty = true;
        self
    }

    /// Route a MIDI channel to a target unit.
    ///
    /// All events on the specified channel will be routed to the target unit.
    /// Can be called multiple times to add multiple targets to the same channel.
    pub fn channel(&mut self, channel: u8, unit_id: u64) -> &mut Self {
        // Check if a route for this channel already exists
        for route in &mut self.routes {
            if route.port.is_none() && route.channel == Some(channel) {
                if !route.targets.contains(&unit_id) {
                    route.targets.push(unit_id);
                }
                self.dirty = true;
                return self;
            }
        }

        // Create new route
        self.routes
            .push(MidiRoute::for_channel(channel).with_target(unit_id));
        self.dirty = true;
        self
    }

    /// Route a MIDI port to a target unit.
    ///
    /// All events from the specified port will be routed to the target unit.
    pub fn port(&mut self, port: usize, unit_id: u64) -> &mut Self {
        // Check if a route for this port already exists
        for route in &mut self.routes {
            if route.port == Some(port) && route.channel.is_none() {
                if !route.targets.contains(&unit_id) {
                    route.targets.push(unit_id);
                }
                self.dirty = true;
                return self;
            }
        }

        // Create new route
        self.routes
            .push(MidiRoute::for_port(port).with_target(unit_id));
        self.dirty = true;
        self
    }

    /// Route a specific port+channel combination to a target unit.
    pub fn port_channel(&mut self, port: usize, channel: u8, unit_id: u64) -> &mut Self {
        // Check if a route for this port+channel already exists
        for route in &mut self.routes {
            if route.port == Some(port) && route.channel == Some(channel) {
                if !route.targets.contains(&unit_id) {
                    route.targets.push(unit_id);
                }
                self.dirty = true;
                return self;
            }
        }

        // Create new route
        self.routes
            .push(MidiRoute::for_port_channel(port, channel).with_target(unit_id));
        self.dirty = true;
        self
    }

    /// Route all MIDI to multiple targets (global layer).
    ///
    /// All incoming MIDI (any port, any channel) will be routed to all targets.
    /// Replaces any existing global layer.
    pub fn layer(&mut self, targets: &[u64]) -> &mut Self {
        // Remove existing "any" routes
        self.routes
            .retain(|r| r.port.is_some() || r.channel.is_some());

        // Add new layer route
        if !targets.is_empty() {
            self.routes.push(MidiRoute::new().with_targets(targets));
        }
        self.dirty = true;
        self
    }

    /// Route a MIDI channel to multiple targets (channel layer).
    ///
    /// Events on the specified channel will be routed to all targets.
    /// Replaces any existing routes for this channel.
    pub fn channel_layer(&mut self, channel: u8, targets: &[u64]) -> &mut Self {
        // Remove existing routes for this channel
        self.routes
            .retain(|r| !(r.port.is_none() && r.channel == Some(channel)));

        // Add new layer route
        if !targets.is_empty() {
            self.routes
                .push(MidiRoute::for_channel(channel).with_targets(targets));
        }
        self.dirty = true;
        self
    }

    /// Remove all routes to a specific unit.
    ///
    /// Call this when removing a unit from the audio graph.
    pub fn remove_unit(&mut self, unit_id: u64) -> &mut Self {
        for route in &mut self.routes {
            route.targets.retain(|&id| id != unit_id);
        }
        // Remove empty routes
        self.routes.retain(|r| !r.targets.is_empty());

        if self.fallback_target == Some(unit_id) {
            self.fallback_target = None;
        }
        self.dirty = true;
        self
    }

    /// Clear all routes.
    pub fn clear(&mut self) -> &mut Self {
        self.routes.clear();
        self.fallback_target = None;
        self.dirty = true;
        self
    }

    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Commit changes to the atomic snapshot.
    ///
    /// This creates a new immutable snapshot and atomically swaps it in.
    /// The audio thread will see the new configuration on its next load().
    pub fn commit(&mut self) {
        if !self.dirty {
            return;
        }

        let snapshot = MidiRoutingSnapshot::from_routes(self.routes.clone(), self.fallback_target);
        self.snapshot.store(Arc::new(snapshot));
        self.dirty = false;
    }
}

impl Default for MidiRoutingTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note_on(channel: u8, note: u8) -> MidiEvent {
        MidiEvent::note_on(0, channel, note, 100)
    }

    #[test]
    fn test_empty_routing() {
        let snapshot = MidiRoutingSnapshot::empty();
        let event = note_on(0, 60);
        let targets: Vec<_> = snapshot.route(0, &event).collect();
        assert!(targets.is_empty());
    }

    #[test]
    fn test_fallback_routing() {
        let mut table = MidiRoutingTable::new();
        table.fallback(42);
        table.commit();

        let snapshot = table.load();
        let event = note_on(0, 60);
        let targets: Vec<_> = snapshot.route(0, &event).collect();
        assert_eq!(targets, vec![42]);
    }

    #[test]
    fn test_channel_routing() {
        let mut table = MidiRoutingTable::new();
        table.channel(0, 100).channel(1, 200);
        table.commit();

        let snapshot = table.load();

        // Channel 0 → unit 100
        let event = note_on(0, 60);
        let targets: Vec<_> = snapshot.route(0, &event).collect();
        assert_eq!(targets, vec![100]);

        // Channel 1 → unit 200
        let event = note_on(1, 60);
        let targets: Vec<_> = snapshot.route(0, &event).collect();
        assert_eq!(targets, vec![200]);

        // Channel 2 → no targets (no routes configured)
        let event = note_on(2, 60);
        let targets: Vec<_> = snapshot.route(0, &event).collect();
        assert!(targets.is_empty());
    }

    #[test]
    fn test_channel_layering() {
        let mut table = MidiRoutingTable::new();
        table.channel_layer(0, &[100, 200, 300]);
        table.commit();

        let snapshot = table.load();
        let event = note_on(0, 60);
        let targets: Vec<_> = snapshot.route(0, &event).collect();
        assert_eq!(targets, vec![100, 200, 300]);
    }

    #[test]
    fn test_port_routing() {
        let mut table = MidiRoutingTable::new();
        table.port(0, 100).port(1, 200);
        table.commit();

        let snapshot = table.load();
        let event = note_on(0, 60);

        // Port 0 → unit 100
        let targets: Vec<_> = snapshot.route(0, &event).collect();
        assert_eq!(targets, vec![100]);

        // Port 1 → unit 200
        let targets: Vec<_> = snapshot.route(1, &event).collect();
        assert_eq!(targets, vec![200]);
    }

    #[test]
    fn test_port_channel_routing() {
        let mut table = MidiRoutingTable::new();
        table
            .port_channel(0, 0, 100) // Port 0, Ch 0 → 100
            .port_channel(0, 1, 200) // Port 0, Ch 1 → 200
            .port_channel(1, 0, 300); // Port 1, Ch 0 → 300
        table.commit();

        let snapshot = table.load();

        // Port 0, Channel 0 → 100
        let event = note_on(0, 60);
        let targets: Vec<_> = snapshot.route(0, &event).collect();
        assert_eq!(targets, vec![100]);

        // Port 0, Channel 1 → 200
        let event = note_on(1, 60);
        let targets: Vec<_> = snapshot.route(0, &event).collect();
        assert_eq!(targets, vec![200]);

        // Port 1, Channel 0 → 300
        let event = note_on(0, 60);
        let targets: Vec<_> = snapshot.route(1, &event).collect();
        assert_eq!(targets, vec![300]);
    }

    #[test]
    fn test_global_layer() {
        let mut table = MidiRoutingTable::new();
        table.layer(&[100, 200]);
        table.commit();

        let snapshot = table.load();

        // Any channel, any port → both units
        let event = note_on(5, 60);
        let targets: Vec<_> = snapshot.route(2, &event).collect();
        assert_eq!(targets, vec![100, 200]);
    }

    #[test]
    fn test_remove_unit() {
        let mut table = MidiRoutingTable::new();
        table.channel(0, 100).channel(0, 200).fallback(100);
        table.commit();

        // Remove unit 100
        table.remove_unit(100);
        table.commit();

        let snapshot = table.load();
        let event = note_on(0, 60);
        let targets: Vec<_> = snapshot.route(0, &event).collect();
        assert_eq!(targets, vec![200]);
        assert_eq!(snapshot.fallback(), None);
    }

    #[test]
    fn test_route_single() {
        let mut table = MidiRoutingTable::new();
        table.channel(0, 100).fallback(999);
        table.commit();

        let snapshot = table.load();

        // Channel 0 → 100 (via route)
        let event = note_on(0, 60);
        assert_eq!(snapshot.route_single(0, &event), Some(100));

        // Channel 5 → 999 (via fallback)
        let event = note_on(5, 60);
        assert_eq!(snapshot.route_single(0, &event), Some(999));
    }

    #[test]
    fn test_dirty_flag() {
        let mut table = MidiRoutingTable::new();
        assert!(!table.is_dirty());

        table.channel(0, 100);
        assert!(table.is_dirty());

        table.commit();
        assert!(!table.is_dirty());
    }

    #[test]
    fn test_no_duplicate_targets() {
        let mut table = MidiRoutingTable::new();
        // Add same target via channel route and global layer
        table.channel(0, 100).layer(&[100, 200]);
        table.commit();

        let snapshot = table.load();
        let event = note_on(0, 60);
        let targets: Vec<_> = snapshot.route(0, &event).collect();

        // Should not have duplicates
        assert_eq!(targets.len(), 2);
        assert!(targets.contains(&100));
        assert!(targets.contains(&200));
    }

    #[test]
    fn test_chainable_api() {
        let mut table = MidiRoutingTable::new();

        // All methods should be chainable
        table
            .channel(0, 100)
            .channel(1, 200)
            .channel(9, 300)
            .port(0, 400)
            .port_channel(1, 0, 500)
            .fallback(999);

        table.commit();

        let snapshot = table.load();

        // Verify channel routes
        let event = note_on(0, 60);
        let targets: Vec<_> = snapshot.route(0, &event).collect();
        assert!(targets.contains(&100));
        assert!(targets.contains(&400)); // port 0 matches

        // Verify fallback for unmatched
        let event = note_on(15, 60);
        let targets: Vec<_> = snapshot.route(99, &event).collect();
        assert_eq!(targets, vec![999]);
    }
}
