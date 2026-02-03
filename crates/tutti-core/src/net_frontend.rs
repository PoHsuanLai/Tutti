use crate::compat::{any, Box, HashMap, String, ToString, Vec};

#[cfg(feature = "neural")]
use crate::compat::Arc;
use fundsp::net::{Net, NodeId, Source};
use fundsp::prelude::AudioUnit;
use fundsp::realnet::NetBackend;

#[cfg(feature = "neural")]
use crate::neural::{
    BatchingStrategy, GraphAnalyzer, NeuralNodeManager, SharedNeuralNodeManager,
};

/// Chain multiple nodes together in a linear signal flow.
///
/// Use `=> output` to pipe the last node to output.
///
/// # Example
/// ```ignore
/// chain!(net, sine_id, filter_id, gain_id, reverb_id => output);
/// let last = chain!(net, sine_id, filter_id); // Returns filter_id
/// ```
#[macro_export]
macro_rules! chain {
    // Chain with => output at the end
    ($net:expr, $first:expr, $second:expr => output) => {{
        $net.pipe($first, $second);
        $net.pipe_output($second);
    }};

    ($net:expr, $first:expr, $second:expr, $($rest:expr),+ => output) => {{
        $net.pipe($first, $second);
        chain!($net, $second, $($rest),+ => output);
    }};

    // Chain without output (returns last node)
    ($net:expr, $first:expr, $second:expr) => {{
        $net.pipe($first, $second);
        $second
    }};

    ($net:expr, $first:expr, $second:expr, $($rest:expr),+) => {{
        $net.pipe($first, $second);
        chain!($net, $second, $($rest),+)
    }};
}

/// Mix multiple signals into a single node using fundsp's join.
///
/// Supports 2-8 sources (FunDSP uses compile-time sized types).
///
/// # Example
/// ```ignore
/// let mixed = mix!(net, osc1, osc2, osc3);
/// ```
#[macro_export]
macro_rules! mix {
    ($net:expr, $s1:expr, $s2:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(Box::new(join::<typenum::U2>()));
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(Box::new(join::<typenum::U3>()));
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr, $s4:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(Box::new(join::<typenum::U4>()));
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        $net.connect_ports($s4, 0, m, 3);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr, $s4:expr, $s5:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(Box::new(join::<typenum::U5>()));
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        $net.connect_ports($s4, 0, m, 3);
        $net.connect_ports($s5, 0, m, 4);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr, $s4:expr, $s5:expr, $s6:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(Box::new(join::<typenum::U6>()));
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        $net.connect_ports($s4, 0, m, 3);
        $net.connect_ports($s5, 0, m, 4);
        $net.connect_ports($s6, 0, m, 5);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr, $s4:expr, $s5:expr, $s6:expr, $s7:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(Box::new(join::<typenum::U7>()));
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        $net.connect_ports($s4, 0, m, 3);
        $net.connect_ports($s5, 0, m, 4);
        $net.connect_ports($s6, 0, m, 5);
        $net.connect_ports($s7, 0, m, 6);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr, $s4:expr, $s5:expr, $s6:expr, $s7:expr, $s8:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(Box::new(join::<typenum::U8>()));
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        $net.connect_ports($s4, 0, m, 3);
        $net.connect_ports($s5, 0, m, 4);
        $net.connect_ports($s6, 0, m, 5);
        $net.connect_ports($s7, 0, m, 6);
        $net.connect_ports($s8, 0, m, 7);
        m
    }};
}

/// Split a signal to multiple destinations (fan-out).
///
/// # Example
/// ```ignore
/// split!(net, reverb_id => output, analyzer_id);
/// split!(net, reverb_id => output, analyzer_id, meter_id);
/// ```
#[macro_export]
macro_rules! split {
    ($net:expr, $source:expr => output $(, $dest:expr)*) => {{
        $net.pipe_output($source);
        $(
            $net.pipe($source, $dest);
        )*
    }};

    ($net:expr, $source:expr => $first_dest:expr $(, $dest:expr)*) => {{
        $net.pipe($source, $first_dest);
        $(
            $net.pipe($source, $dest);
        )*
    }};
}

/// MIDI connection between two nodes in the graph
#[cfg(feature = "midi")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiConnection {
    pub source: NodeId,
    pub dest: NodeId,
}

/// Metadata about a node in the graph
#[derive(Debug, Clone)]
pub struct NodeInfo {
    /// Node ID
    pub id: NodeId,

    /// Human-readable name (from tag or generated)
    pub name: String,

    /// Number of input channels
    pub inputs: usize,

    /// Number of output channels
    pub outputs: usize,

    /// Reported latency in samples (for PDC)
    pub latency: usize,

    /// Optional user tag
    pub tag: Option<String>,

    /// Type name (from std::any::type_name)
    pub type_name: String,
}

pub struct TuttiNet {
    net: Net,

    /// MIDI connections (source → dest)
    /// Defines how MIDI flows between nodes
    /// Note: Routing is handled externally (e.g., by tutti-midi)
    #[cfg(feature = "midi")]
    midi_connections: Vec<MidiConnection>,

    /// MIDI event registry for routing events to nodes
    #[cfg(feature = "midi")]
    midi_registry: crate::midi::MidiRegistry,

    #[cfg(feature = "neural")]
    neural_manager: SharedNeuralNodeManager,

    #[cfg(feature = "neural")]
    batching_strategy: Option<BatchingStrategy>,

    /// Optional user tags for nodes (for UI organization)
    node_tags: HashMap<NodeId, String>,
}

impl TuttiNet {
    #[cfg(all(test, not(feature = "neural")))]
    pub(crate) fn new() -> (Self, NetBackend) {
        Self::with_io(0, 2)
    }

    #[cfg(all(test, feature = "neural"))]
    pub(crate) fn new() -> (Self, NetBackend, SharedNeuralNodeManager) {
        Self::with_io(0, 2)
    }

    #[cfg(not(feature = "neural"))]
    pub(crate) fn with_io(inputs: usize, outputs: usize) -> (Self, NetBackend) {
        let mut net = Net::new(inputs, outputs);
        let backend = net.backend();
        (
            Self {
                net,
                #[cfg(feature = "midi")]
                midi_connections: Vec::new(),
                #[cfg(feature = "midi")]
                midi_registry: crate::midi::MidiRegistry::new(),
                node_tags: HashMap::new(),
            },
            backend,
        )
    }

    #[cfg(feature = "neural")]
    pub(crate) fn with_io(
        inputs: usize,
        outputs: usize,
    ) -> (Self, NetBackend, SharedNeuralNodeManager) {
        let mut net = Net::new(inputs, outputs);
        let backend = net.backend();
        let manager = Arc::new(NeuralNodeManager::new());
        (
            Self {
                net,
                #[cfg(feature = "midi")]
                midi_connections: Vec::new(),
                #[cfg(feature = "midi")]
                midi_registry: crate::midi::MidiRegistry::new(),
                neural_manager: manager.clone(),
                batching_strategy: None,
                node_tags: HashMap::new(),
            },
            backend,
            manager,
        )
    }

    /// Add a node to the graph.
    pub fn add(&mut self, unit: Box<dyn AudioUnit>) -> NodeId {
        self.net.push(unit)
    }

    /// Add a node with a user tag for later lookup.
    pub fn add_tagged(&mut self, unit: Box<dyn AudioUnit>, tag: impl Into<String>) -> NodeId {
        let id = self.add(unit);
        self.node_tags.insert(id, tag.into());
        id
    }

    /// Add a node with fade-in for click-free insertion during playback.
    pub fn add_with_fade(
        &mut self,
        fade: fundsp::sequencer::Fade,
        fade_time: f32,
        unit: Box<dyn AudioUnit>,
    ) -> NodeId {
        self.net.fade_in(fade, fade_time, unit)
    }

    /// Add a mono-to-stereo splitter node (1 input → 2 identical outputs).
    pub fn add_split(&mut self) -> NodeId {
        use fundsp::prelude::*;
        self.net.push(Box::new(split::<U1>()))
    }

    /// Add a stereo-to-mono joiner node (2 inputs → 1 mixed output).
    pub fn add_join(&mut self) -> NodeId {
        use fundsp::prelude::*;
        self.net.push(Box::new(join::<U2>()))
    }

    /// Remove a node from the graph, returning the removed unit.
    #[cfg(not(feature = "neural"))]
    pub fn remove(&mut self, node: NodeId) -> Box<dyn AudioUnit> {
        self.net.remove(node)
    }

    /// Remove a node from the graph, returning the removed unit.
    #[cfg(feature = "neural")]
    pub fn remove(&mut self, node: NodeId) -> Box<dyn AudioUnit> {
        self.neural_manager.unregister(&node);
        self.net.remove(node)
    }

    /// Replace a node with a new unit, returning the old unit.
    pub fn replace(&mut self, node: NodeId, unit: Box<dyn AudioUnit>) -> Box<dyn AudioUnit> {
        self.net.replace(node, unit)
    }

    /// Replace a node with crossfade for click-free hot-swapping.
    pub fn crossfade(
        &mut self,
        node: NodeId,
        fade: fundsp::sequencer::Fade,
        fade_time: f32,
        unit: Box<dyn AudioUnit>,
    ) {
        self.net.crossfade(node, fade, fade_time, unit);
    }

    /// Connect first output of source to first input of target.
    ///
    /// For connecting specific ports, use [`connect_ports`](Self::connect_ports).
    /// For connecting all outputs to all inputs, use [`pipe_all`](Self::pipe_all).
    pub fn pipe(&mut self, source: NodeId, target: NodeId) {
        self.net.connect(source, 0, target, 0);
    }

    /// Connect specific ports between two nodes.
    pub fn connect_ports(&mut self, from: NodeId, from_port: usize, to: NodeId, to_port: usize) {
        self.net.connect(from, from_port, to, to_port);
    }

    /// Connect all outputs of source to all inputs of target.
    pub fn pipe_all(&mut self, source: NodeId, target: NodeId) {
        self.net.pipe_all(source, target);
    }

    /// Connect a node's output to the graph output.
    pub fn pipe_output(&mut self, source: NodeId) {
        self.net.pipe_output(source);
    }

    /// Connect the graph input to a node's input.
    pub fn pipe_input(&mut self, target: NodeId) {
        self.net.pipe_input(target);
    }

    /// Set a specific source for a node's input channel.
    pub fn set_source(&mut self, node: NodeId, channel: usize, source: Source) {
        self.net.set_source(node, channel, source);
    }

    /// Disconnect a node's input port.
    pub fn disconnect(&mut self, node: NodeId, port: usize) {
        self.net.disconnect(node, port);
    }

    /// Set or update a node's tag.
    pub fn set_tag(&mut self, id: NodeId, tag: impl Into<String>) {
        self.node_tags.insert(id, tag.into());
    }

    /// Get a node's tag.
    pub fn get_tag(&self, id: NodeId) -> Option<&str> {
        self.node_tags.get(&id).map(|s| s.as_str())
    }

    /// Remove a node's tag.
    pub fn remove_tag(&mut self, id: NodeId) {
        self.node_tags.remove(&id);
    }

    /// Find a node by tag (returns first match).
    pub fn find_by_tag(&self, tag: &str) -> Option<NodeId> {
        self.node_tags
            .iter()
            .find(|(_, t)| t.as_str() == tag)
            .map(|(id, _)| *id)
    }

    /// Find all nodes with a given tag.
    pub fn find_all_by_tag(&self, tag: &str) -> Vec<NodeId> {
        self.node_tags
            .iter()
            .filter(|(_, t)| t.as_str() == tag)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Get immutable reference to a node's AudioUnit.
    pub fn node(&self, node: NodeId) -> &dyn AudioUnit {
        self.net.node(node)
    }

    /// Get mutable reference to a node's AudioUnit.
    pub fn node_mut(&mut self, node: NodeId) -> &mut dyn AudioUnit {
        self.net.node_mut(node)
    }

    /// Get mutable reference to a node with automatic downcasting.
    pub fn node_mut_typed<T: AudioUnit + 'static>(&mut self, id: NodeId) -> Option<&mut T> {
        let unit = self.node_mut(id);
        <dyn AudioUnit>::as_any_mut(unit).downcast_mut::<T>()
    }

    /// Get immutable reference to a node with automatic downcasting.
    pub fn node_ref_typed<T: AudioUnit + 'static>(&self, id: NodeId) -> Option<&T> {
        let unit = self.node(id);
        <dyn AudioUnit>::as_any(unit).downcast_ref::<T>()
    }

    /// Try to mutate a node, calling closure if downcasting succeeds.
    pub fn with_node_mut<T, F, R>(&mut self, id: NodeId, f: F) -> Option<R>
    where
        T: AudioUnit + 'static,
        F: FnOnce(&mut T) -> R,
    {
        self.node_mut_typed::<T>(id).map(f)
    }

    /// Get metadata for a specific node.
    pub fn node_info(&self, id: NodeId) -> Option<NodeInfo> {
        if !self.contains(id) {
            return None;
        }

        let unit = self.node(id);
        Some(NodeInfo {
            id,
            name: self
                .node_tags
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("Node {:?}", id)),
            inputs: unit.inputs(),
            outputs: unit.outputs(),
            latency: 0,
            tag: self.node_tags.get(&id).cloned(),
            type_name: any::type_name_of_val(unit).to_string(),
        })
    }

    /// Get metadata for all tagged nodes in the graph.
    pub fn nodes(&self) -> Vec<NodeInfo> {
        let mut infos = Vec::new();
        for node_id in self.node_tags.keys() {
            if let Some(info) = self.node_info(*node_id) {
                infos.push(info);
            }
        }
        infos
    }

    /// Commit pending changes to the backend for real-time playback.
    pub fn commit(&mut self) {
        self.net.commit();

        #[cfg(feature = "neural")]
        {
            if !self.neural_manager.is_empty() {
                let analyzer = GraphAnalyzer::new(&self.net, &self.neural_manager);
                self.batching_strategy = Some(analyzer.analyze());

                if let Some(ref strategy) = self.batching_strategy {
                    tracing::debug!(
                        "Batching strategy: {} models, {} parallel groups, efficiency: {:.1}x",
                        strategy.model_count(),
                        strategy.parallel_group_count(),
                        strategy.batch_efficiency()
                    );
                }
            } else {
                self.batching_strategy = None;
            }
        }
    }

    /// Check if the graph has a backend attached.
    pub fn has_backend(&self) -> bool {
        self.net.has_backend()
    }

    /// Number of graph inputs.
    pub fn inputs(&self) -> usize {
        self.net.inputs()
    }

    /// Number of graph outputs.
    pub fn outputs(&self) -> usize {
        self.net.outputs()
    }

    /// Number of nodes in the graph.
    pub fn size(&self) -> usize {
        self.net.size()
    }

    /// Check if a node exists in the graph.
    pub fn contains(&self, node: NodeId) -> bool {
        self.net.contains(node)
    }

    /// Set the sample rate for all nodes.
    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.net.set_sample_rate(sample_rate);
    }

    /// Reset all nodes to their initial state.
    pub fn reset(&mut self) {
        self.net.reset();
    }

    /// Get the last error from graph validation.
    pub fn error(&mut self) -> &Option<fundsp::net::NetError> {
        self.net.error()
    }

    /// Validate the graph structure (panics on invalid graph).
    pub fn check(&self) {
        self.net.check();
    }

    /// Render the graph offline to a Wave.
    pub fn render_offline(&self, sample_rate: f64, duration: f64) -> fundsp::wave::Wave {
        let mut render_net = self.net.clone();
        render_net.set_sample_rate(sample_rate);
        fundsp::wave::Wave::render(sample_rate, duration, &mut render_net)
    }

    /// Render the graph offline with latency compensation.
    pub fn render_offline_latency(&self, sample_rate: f64, duration: f64) -> fundsp::wave::Wave {
        let mut render_net = self.net.clone();
        render_net.set_sample_rate(sample_rate);
        fundsp::wave::Wave::render_latency(sample_rate, duration, &mut render_net)
    }

    /// Clone the underlying Net for offline export.
    pub fn clone_net(&self) -> Net {
        self.net.clone()
    }

    /// Add a MIDI connection between two nodes.
    #[cfg(feature = "midi")]
    pub fn add_midi_connection(&mut self, source: NodeId, dest: NodeId) {
        self.midi_connections.push(MidiConnection { source, dest });
    }

    /// Remove a MIDI connection between two nodes.
    #[cfg(feature = "midi")]
    pub fn remove_midi_connection(&mut self, source: NodeId, dest: NodeId) {
        self.midi_connections
            .retain(|conn| !(conn.source == source && conn.dest == dest));
    }

    /// Clear all MIDI connections.
    #[cfg(feature = "midi")]
    pub fn clear_midi_connections(&mut self) {
        self.midi_connections.clear();
    }

    /// Get all MIDI connections.
    #[cfg(feature = "midi")]
    pub fn midi_connections(&self) -> &[MidiConnection] {
        &self.midi_connections
    }

    /// Queue MIDI events to be sent to a node.
    #[cfg(feature = "midi")]
    pub fn queue_midi(&mut self, node: NodeId, events: &[crate::midi::MidiEvent]) {
        let unit_id = self.net.node(node).get_id();
        self.midi_registry.register_unit(unit_id);
        self.midi_registry.queue(unit_id, events);
    }

    /// Get a reference to the MIDI registry.
    #[cfg(feature = "midi")]
    pub fn midi_registry(&self) -> &crate::midi::MidiRegistry {
        &self.midi_registry
    }

    /// Get the neural node manager.
    #[cfg(feature = "neural")]
    pub fn neural_manager(&self) -> &SharedNeuralNodeManager {
        &self.neural_manager
    }

    /// Get the current batching strategy for neural nodes.
    #[cfg(feature = "neural")]
    pub fn batching_strategy(&self) -> Option<&BatchingStrategy> {
        self.batching_strategy.as_ref()
    }

    /// Check if a node is a neural audio node.
    #[cfg(feature = "neural")]
    pub fn is_neural(&self, node: NodeId) -> bool {
        self.neural_manager.is_neural(&node)
    }

    /// Get the number of neural nodes in the graph.
    #[cfg(feature = "neural")]
    pub fn neural_count(&self) -> usize {
        self.neural_manager.len()
    }

    /// Direct mutable access to the underlying fundsp `Net`.
    ///
    /// Use as an escape hatch for fundsp features not exposed by TuttiNet.
    ///
    /// # Warning
    ///
    /// Modifications bypass TuttiNet's tracking:
    /// - Neural nodes won't be registered for batching optimization
    /// - Tags won't be synced
    /// - MIDI connections may become invalid
    ///
    /// Prefer TuttiNet methods when possible.
    pub fn inner(&mut self) -> &mut Net {
        &mut self.net
    }

    /// Direct read-only access to the underlying fundsp `Net`.
    ///
    /// Safe for inspection, but see [`inner`](Self::inner) for mutation caveats.
    pub fn inner_ref(&self) -> &Net {
        &self.net
    }
}

impl Default for TuttiNet {
    fn default() -> Self {
        Self {
            net: Net::new(0, 2),
            #[cfg(feature = "midi")]
            midi_connections: Vec::new(),
            #[cfg(feature = "midi")]
            midi_registry: crate::midi::MidiRegistry::new(),
            #[cfg(feature = "neural")]
            neural_manager: Arc::new(NeuralNodeManager::new()),
            #[cfg(feature = "neural")]
            batching_strategy: None,
            node_tags: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fundsp::prelude::*;

    // Helper to create TuttiNet regardless of neural feature
    #[cfg(not(feature = "neural"))]
    fn create_net() -> (TuttiNet, NetBackend) {
        TuttiNet::new()
    }

    #[cfg(feature = "neural")]
    fn create_net() -> (TuttiNet, NetBackend) {
        let (net, backend, _registry) = TuttiNet::new();
        (net, backend)
    }

    #[cfg(not(feature = "neural"))]
    fn create_net_with_io(inputs: usize, outputs: usize) -> (TuttiNet, NetBackend) {
        TuttiNet::with_io(inputs, outputs)
    }

    #[cfg(feature = "neural")]
    fn create_net_with_io(inputs: usize, outputs: usize) -> (TuttiNet, NetBackend) {
        let (net, backend, _registry) = TuttiNet::with_io(inputs, outputs);
        (net, backend)
    }

    #[test]
    fn test_tutti_net_creation() {
        let (net, _backend) = create_net();
        assert_eq!(net.inputs(), 0);
        assert_eq!(net.outputs(), 2);
        assert_eq!(net.size(), 0);
    }

    #[test]
    fn test_tutti_net_with_io() {
        let (net, _backend) = create_net_with_io(2, 4);
        assert_eq!(net.inputs(), 2);
        assert_eq!(net.outputs(), 4);
    }

    #[test]
    fn test_add_and_connect() {
        let (mut net, _backend) = create_net();

        // Add two nodes (specify f32 type for sine_hz)
        let synth = net.add(Box::new(sine_hz::<f32>(440.0)));
        let filter = net.add(Box::new(lowpass_hz::<f32>(1000.0, 1.0)));

        assert_eq!(net.size(), 2);
        assert!(net.contains(synth));
        assert!(net.contains(filter));

        // Connect them
        net.pipe(synth, filter);
        net.pipe_output(filter);

        // Verify no errors
        assert!(net.error().is_none());
    }

    #[test]
    fn test_remove_node() {
        let (mut net, _backend) = create_net();

        let synth = net.add(Box::new(sine_hz::<f32>(440.0)));
        assert_eq!(net.size(), 1);

        let _removed = net.remove(synth);
        assert_eq!(net.size(), 0);
        assert!(!net.contains(synth));
    }

    #[test]
    fn test_replace_node() {
        let (mut net, _backend) = create_net();

        let synth = net.add(Box::new(sine_hz::<f32>(440.0)));
        net.pipe_output(synth);

        // Replace with a different frequency
        let _old = net.replace(synth, Box::new(sine_hz::<f32>(880.0)));

        // Node should still exist with same ID
        assert!(net.contains(synth));
    }

    #[test]
    fn test_commit_and_backend() {
        let (mut net, mut backend) = create_net();

        // Add a constant generator
        let dc_node = net.add(Box::new(dc((0.5f32, 0.5f32))));
        net.pipe_output(dc_node);
        net.commit();

        // Process some samples through the backend
        let (left, right) = backend.get_stereo();

        // Should output approximately 0.5
        assert!((left - 0.5).abs() < 0.001);
        assert!((right - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_render_offline() {
        let (mut net, _backend) = create_net();

        // Add a sine wave generator
        let synth = net.add(Box::new(sine_hz::<f32>(440.0) * 0.5));
        net.pipe_output(synth);

        // Render 0.1 seconds of audio
        let wave = net.render_offline(44100.0, 0.1);

        // Check wave properties
        assert_eq!(wave.channels(), 2); // Stereo output
        assert_eq!(wave.sample_rate(), 44100.0);
        assert_eq!(wave.length(), 4410); // 0.1 * 44100

        // Check that we got actual audio (not silence)
        let amplitude = wave.amplitude();
        assert!(
            amplitude > 0.4,
            "Expected amplitude > 0.4, got {}",
            amplitude
        );
        assert!(
            amplitude <= 0.5,
            "Expected amplitude <= 0.5, got {}",
            amplitude
        );
    }

    #[test]
    fn test_render_offline_latency() {
        let (mut net, _backend) = create_net();

        // Add a stereo generator with a limiter (which has latency)
        // sine_hz is mono, so we use pan to make it stereo, then limit
        let synth = net.add(Box::new(
            (sine_hz::<f32>(440.0) * 0.8) >> pan(0.0) >> limiter_stereo(0.5, 0.5),
        ));
        net.pipe_output(synth);

        // Render with latency compensation
        let wave = net.render_offline_latency(44100.0, 0.1);

        // Check wave properties
        assert_eq!(wave.channels(), 2);
        assert!(wave.length() > 0);

        // Audio should be limited
        let amplitude = wave.amplitude();
        assert!(amplitude <= 1.0, "Limiter should cap amplitude at 1.0");
    }
}
