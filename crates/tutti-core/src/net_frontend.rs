use crate::compat::{any, Box, HashMap, String, ToString, Vec};
use crate::pdc;

#[cfg(feature = "neural")]
use crate::compat::Arc;
use fundsp::net::{Net, NodeId, Source};
use fundsp::prelude::AudioUnit;
use fundsp::realnet::NetBackend;

#[cfg(feature = "neural")]
use crate::neural::{
    BatchingStrategy, GraphAnalyzer, NeuralModelId, NeuralNodeManager, SharedNeuralNodeManager,
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
        let m = $net.add(join::<typenum::U2>()).id();
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(join::<typenum::U3>()).id();
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr, $s4:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(join::<typenum::U4>()).id();
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        $net.connect_ports($s4, 0, m, 3);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr, $s4:expr, $s5:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(join::<typenum::U5>()).id();
        $net.connect_ports($s1, 0, m, 0);
        $net.connect_ports($s2, 0, m, 1);
        $net.connect_ports($s3, 0, m, 2);
        $net.connect_ports($s4, 0, m, 3);
        $net.connect_ports($s5, 0, m, 4);
        m
    }};
    ($net:expr, $s1:expr, $s2:expr, $s3:expr, $s4:expr, $s5:expr, $s6:expr) => {{
        use $crate::dsp::*;
        let m = $net.add(join::<typenum::U6>()).id();
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
        let m = $net.add(join::<typenum::U7>()).id();
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
        let m = $net.add(join::<typenum::U8>()).id();
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

/// Metadata about a node in the graph
#[derive(Debug, Clone)]
pub struct NodeInfo {
    /// Node ID
    pub id: NodeId,

    /// Number of input channels
    pub inputs: usize,

    /// Number of output channels
    pub outputs: usize,

    /// Reported latency in samples (for PDC)
    pub latency: usize,

    /// Type name (from std::any::type_name)
    pub type_name: String,
}

/// Fluent handle for chaining node connections.
///
/// Returned by [`TuttiNet::add`] for Web Audio-style method chaining.
///
/// # Example
/// ```ignore
/// net.add(saw_hz(110.0))
///    .connect(lowpass_hz(800.0))
///    .connect(reverb_stereo(10.0, 2.0, 0.5))
///    .to_master();
/// ```
pub struct NodeHandle<'a> {
    net: &'a mut TuttiNet,
    id: NodeId,
}

impl<'a> NodeHandle<'a> {
    /// Chain another audio unit after this node.
    ///
    /// Creates a new node and pipes this node's output to its input.
    pub fn connect<U: AudioUnit + 'static>(self, unit: U) -> NodeHandle<'a> {
        let next_id = self.net.net.push(Box::new(unit));
        self.net.net.connect(self.id, 0, next_id, 0);
        NodeHandle {
            net: self.net,
            id: next_id,
        }
    }

    /// Connect this node to the graph output.
    ///
    /// Returns the final node's ID.
    pub fn to_master(self) -> NodeId {
        self.net.net.pipe_output(self.id);
        self.id
    }

    /// Get the node ID without connecting to output.
    ///
    /// Use when you need to manually wire connections.
    pub fn id(self) -> NodeId {
        self.id
    }
}

pub struct TuttiNet {
    net: Net,

    /// MIDI event registry for routing events to nodes
    #[cfg(feature = "midi")]
    midi_registry: crate::midi::MidiRegistry,

    #[cfg(feature = "neural")]
    neural_manager: SharedNeuralNodeManager,

    #[cfg(feature = "neural")]
    batching_strategy: Option<BatchingStrategy>,

    /// PDC delay nodes inserted during commit (tracked for removal on next commit)
    pdc_delay_nodes: Vec<NodeId>,

    /// Whether automatic PDC is enabled
    pdc_enabled: bool,

    /// Total graph latency from the last PDC analysis
    total_latency: usize,

    /// Per-node latency cache (populated during PDC analysis)
    node_latency_cache: HashMap<NodeId, usize>,
}

impl TuttiNet {
    pub(crate) fn new(inputs: usize, outputs: usize) -> Self {
        Self {
            net: Net::new(inputs, outputs),
            #[cfg(feature = "midi")]
            midi_registry: crate::midi::MidiRegistry::new(),
            #[cfg(feature = "neural")]
            neural_manager: Arc::new(NeuralNodeManager::new()),
            #[cfg(feature = "neural")]
            batching_strategy: None,
            pdc_delay_nodes: Vec::new(),
            pdc_enabled: true,
            total_latency: 0,
            node_latency_cache: HashMap::new(),
        }
    }

    /// Extract the real-time backend for the audio callback.
    ///
    /// Can only be called once (panics on second call).
    pub(crate) fn backend(&mut self) -> NetBackend {
        self.net.backend()
    }

    /// Add a node to the graph.
    ///
    /// Returns a [`NodeHandle`] for fluent chaining:
    ///
    /// ```ignore
    /// // Simple: direct to output
    /// net.add(sine_hz(440.0) * 0.5).to_master();
    ///
    /// // Chain: connect multiple effects
    /// net.add(saw_hz(110.0))
    ///    .connect(lowpass_hz(800.0))
    ///    .connect(reverb_stereo(10.0, 2.0, 0.5))
    ///    .to_master();
    ///
    /// // Get ID for manual wiring
    /// let osc = net.add(sine_hz(440.0)).id();
    /// ```
    pub fn add<U: AudioUnit + 'static>(&mut self, unit: U) -> NodeHandle<'_> {
        let id = self.net.push(Box::new(unit));
        NodeHandle { net: self, id }
    }

    /// Add a pre-boxed node to the graph.
    ///
    /// Use this when you have a `Box<dyn AudioUnit>` from external sources.
    /// For most cases, prefer [`add`](Self::add) which auto-boxes.
    pub fn add_boxed(&mut self, unit: Box<dyn AudioUnit>) -> NodeHandle<'_> {
        let id = self.net.push(unit);
        NodeHandle { net: self, id }
    }

    /// Add a neural AudioUnit and register it with the NeuralNodeManager.
    ///
    /// Equivalent to `add()` followed by `neural_manager().register()`.
    /// The registration enables `GraphAnalyzer` to batch this node with
    /// others sharing the same model for GPU-optimized inference.
    ///
    /// # Example
    /// ```ignore
    /// system.graph(|net| {
    ///     let voice = builder.build_voice().unwrap();
    ///     net.add_neural(voice, builder.model_id()).to_master();
    /// });
    /// ```
    #[cfg(feature = "neural")]
    pub fn add_neural<U: AudioUnit + 'static>(
        &mut self,
        unit: U,
        model_id: NeuralModelId,
    ) -> NodeHandle<'_> {
        let id = self.net.push(Box::new(unit));
        self.neural_manager.register(id, model_id);
        NodeHandle { net: self, id }
    }

    /// Add a pre-boxed neural AudioUnit and register it with the NeuralNodeManager.
    ///
    /// Use this when you already have a `Box<dyn AudioUnit>` (e.g., from a node registry).
    #[cfg(feature = "neural")]
    pub fn add_neural_boxed(
        &mut self,
        unit: Box<dyn AudioUnit>,
        model_id: NeuralModelId,
    ) -> NodeHandle<'_> {
        let id = self.net.push(unit);
        self.neural_manager.register(id, model_id);
        NodeHandle { net: self, id }
    }

    /// Add a node with fade-in for click-free insertion during playback.
    pub fn add_with_fade<U: AudioUnit + 'static>(
        &mut self,
        fade: fundsp::sequencer::Fade,
        fade_time: f32,
        unit: U,
    ) -> NodeHandle<'_> {
        let id = self.net.fade_in(fade, fade_time, Box::new(unit));
        NodeHandle { net: self, id }
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
    pub fn replace<U: AudioUnit + 'static>(&mut self, node: NodeId, unit: U) -> Box<dyn AudioUnit> {
        self.net.replace(node, Box::new(unit))
    }

    /// Replace a node with crossfade for click-free hot-swapping.
    pub fn crossfade<U: AudioUnit + 'static>(
        &mut self,
        node: NodeId,
        fade: fundsp::sequencer::Fade,
        fade_time: f32,
        unit: U,
    ) {
        self.net.crossfade(node, fade, fade_time, Box::new(unit));
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
            inputs: unit.inputs(),
            outputs: unit.outputs(),
            latency: self.node_latency_cache.get(&id).copied().unwrap_or(0),
            type_name: any::type_name_of_val(unit).to_string(),
        })
    }

    /// Commit pending changes to the backend for real-time playback.
    ///
    /// When PDC is enabled, this analyzes the graph for latency mismatches and
    /// automatically inserts delay nodes to align signals at merge points.
    pub fn commit(&mut self) {
        for node_id in self.pdc_delay_nodes.drain(..) {
            if self.net.contains(node_id) {
                self.net.remove(node_id);
            }
        }

        if self.pdc_enabled {
            let analysis = pdc::graph_compensator::analyze(&mut self.net);

            // Cache per-node latencies for node_info()
            self.node_latency_cache = analysis.node_latencies;
            self.total_latency = analysis.total_latency;

            // Insert delay nodes for internal merge points
            for comp in &analysis.compensations {
                if comp.delay_samples == 0 {
                    continue;
                }
                self.insert_pdc_delay(comp.node_id, comp.input_port, comp.delay_samples);
            }

            // Insert delay nodes for graph output channels
            for comp in &analysis.output_compensations {
                if comp.delay_samples == 0 {
                    continue;
                }
                self.insert_output_pdc_delay(comp.output_channel, comp.delay_samples);
            }
        } else {
            self.node_latency_cache.clear();
            self.total_latency = 0;
        }

        self.net.commit();

        #[cfg(feature = "neural")]
        {
            if !self.neural_manager.is_empty() {
                let analyzer = GraphAnalyzer::new(&self.net, &self.neural_manager);
                self.batching_strategy = Some(analyzer.analyze());

                // Batching strategy available via batching_strategy()
            } else {
                self.batching_strategy = None;
            }
        }
    }

    /// Insert a PDC delay node on a specific input port of a node.
    fn insert_pdc_delay(&mut self, target: NodeId, input_port: usize, delay_samples: usize) {
        let source = self.net.source(target, input_port);
        if let Source::Local(src_id, src_port) = source {
            // Determine channel width from source node's output count
            let src_outputs = self.net.outputs_in(src_id);
            let target_inputs = self.net.inputs_in(target);

            // Use mono delay for 1-channel connections, stereo for 2-channel
            let pdc_node = if src_outputs == 1 || target_inputs == 1 {
                self.net
                    .push(Box::new(pdc::MonoPdcDelayUnit::new(delay_samples)))
            } else {
                self.net
                    .push(Box::new(pdc::PdcDelayUnit::new(delay_samples)))
            };

            // Rewire: src → pdc → target
            self.net.connect(src_id, src_port, pdc_node, 0);
            self.net
                .set_source(target, input_port, Source::Local(pdc_node, 0));

            self.pdc_delay_nodes.push(pdc_node);
        }
    }

    /// Insert a PDC delay node on a graph output channel.
    fn insert_output_pdc_delay(&mut self, output_channel: usize, delay_samples: usize) {
        let source = self.net.output_source(output_channel);
        if let Source::Local(src_id, src_port) = source {
            // Output channels are typically mono (one channel each)
            let pdc_node = self
                .net
                .push(Box::new(pdc::MonoPdcDelayUnit::new(delay_samples)));

            // Rewire: src → pdc → output
            self.net.connect(src_id, src_port, pdc_node, 0);
            self.net
                .set_output_source(output_channel, Source::Local(pdc_node, 0));

            self.pdc_delay_nodes.push(pdc_node);
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

    /// Enable or disable automatic PDC (Plugin Delay Compensation).
    ///
    /// When enabled (default), `commit()` analyzes the graph for latency
    /// mismatches and inserts delay nodes to align signals at merge points.
    pub fn set_pdc_enabled(&mut self, enabled: bool) {
        self.pdc_enabled = enabled;
    }

    /// Whether automatic PDC is enabled.
    pub fn pdc_enabled(&self) -> bool {
        self.pdc_enabled
    }

    /// Total graph latency in samples from the last `commit()`.
    ///
    /// This is the worst-case latency across all paths from any source to the output.
    pub fn total_latency(&self) -> usize {
        self.total_latency
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fundsp::prelude::*;

    fn create_net() -> (TuttiNet, NetBackend) {
        create_net_with_io(0, 2)
    }

    fn create_net_with_io(inputs: usize, outputs: usize) -> (TuttiNet, NetBackend) {
        let mut net = TuttiNet::new(inputs, outputs);
        let backend = net.backend();
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

        // Add two nodes using fluent API
        let synth = net.add(sine_hz::<f32>(440.0)).id();
        let filter = net.add(lowpass_hz::<f32>(1000.0, 1.0)).id();

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
    fn test_fluent_chain() {
        let (mut net, _backend) = create_net();

        // Test fluent chaining API
        net.add(sine_hz::<f32>(440.0))
            .connect(lowpass_hz::<f32>(1000.0, 1.0))
            .to_master();

        assert_eq!(net.size(), 2);
        assert!(net.error().is_none());
    }

    #[test]
    fn test_remove_node() {
        let (mut net, _backend) = create_net();

        let synth = net.add(sine_hz::<f32>(440.0)).id();
        assert_eq!(net.size(), 1);

        let _removed = net.remove(synth);
        assert_eq!(net.size(), 0);
        assert!(!net.contains(synth));
    }

    #[test]
    fn test_replace_node() {
        let (mut net, _backend) = create_net();

        let synth = net.add(sine_hz::<f32>(440.0)).to_master();

        // Replace with a different frequency
        let _old = net.replace(synth, sine_hz::<f32>(880.0));

        // Node should still exist with same ID
        assert!(net.contains(synth));
    }

    #[test]
    fn test_commit_and_backend() {
        let (mut net, mut backend) = create_net();

        // Add a constant generator using fluent API
        net.add(dc((0.5f32, 0.5f32))).to_master();
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

        // Add a sine wave generator using fluent API
        net.add(sine_hz::<f32>(440.0) * 0.5).to_master();

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
        // Using fluent API
        net.add((sine_hz::<f32>(440.0) * 0.8) >> pan(0.0) >> limiter_stereo(0.5, 0.5))
            .to_master();

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
