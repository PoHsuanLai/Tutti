//! # 23 - Plugin Editor GUI
//!
//! Open a plugin's native editor GUI via the engine's fluent API.
//!
//! Tries enabled plugin formats in order (CLAP, VST3, VST2). Uses feature flags
//! to select which formats are available.
//!
//! Logs parameter changes to stdout when you interact with the plugin GUI.
//!
//! ```bash
//! # CLAP
//! cargo run --example 23_plugin_editor --features "clap,midi"
//!
//! # VST3
//! cargo run --example 23_plugin_editor --features "vst3,midi"
//!
//! # VST2
//! cargo run --example 23_plugin_editor --features "vst2,midi"
//!
//! # All formats (tries CLAP first, then VST3, then VST2)
//! cargo run --example 23_plugin_editor --features "clap,vst3,vst2,midi"
//! ```
//!
//! ## Setup
//!
//! Install a free plugin with a GUI:
//! - [TAL-NoiseMaker](https://tal-software.com/products/tal-noisemaker) (CLAP + VST3 + VST2)
//! - [Surge XT](https://surge-synthesizer.github.io/) (CLAP)

use std::path::Path;
use std::time::{Duration, Instant};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use tutti::prelude::*;
use tutti::TuttiNet;

/// Try loading a plugin from known paths using the engine API.
/// Searches CLAP, VST3, and VST2 formats in order (based on enabled features).
fn try_load_plugin(
    engine: &TuttiEngine,
) -> Option<(Box<dyn tutti::AudioUnit>, tutti::plugin::PluginHandle)> {
    // CLAP plugins
    #[cfg(feature = "clap")]
    {
        let clap_paths = [
            "/Library/Audio/Plug-Ins/CLAP/Surge XT.clap",
            "/Library/Audio/Plug-Ins/CLAP/TAL-NoiseMaker.clap",
        ];
        for path in &clap_paths {
            if Path::new(path).exists() {
                println!("  Trying CLAP: {}", path);
                match engine.clap(path).build() {
                    Ok((unit, handle)) => return Some((unit, handle)),
                    Err(e) => eprintln!("    Failed: {:?}", e),
                }
            }
        }
    }

    // VST3 plugins
    #[cfg(feature = "vst3")]
    {
        let vst3_paths = [
            // Local fixture plugins (Voxengo — free, support f64)
            "tests/fixtures/plugins/Boogex.vst3",
            "tests/fixtures/plugins/SPAN.vst3",
            // System-installed plugins
            "/Library/Audio/Plug-Ins/VST3/TAL-NoiseMaker.vst3",
            "/Library/Audio/Plug-Ins/VST3/Surge XT.vst3",
        ];
        for path in &vst3_paths {
            if Path::new(path).exists() {
                println!("  Trying VST3: {}", path);
                match engine.vst3(path).build() {
                    Ok((unit, handle)) => return Some((unit, handle)),
                    Err(e) => eprintln!("    Failed: {:?}", e),
                }
            }
        }
    }

    // VST2 plugins
    #[cfg(feature = "vst2")]
    {
        let vst2_paths = ["/Library/Audio/Plug-Ins/VST/TAL-NoiseMaker.vst"];
        for path in &vst2_paths {
            if Path::new(path).exists() {
                println!("  Trying VST2: {}", path);
                match engine.vst2(path).build() {
                    Ok((unit, handle)) => return Some((unit, handle)),
                    Err(e) => eprintln!("    Failed: {:?}", e),
                }
            }
        }
    }

    None
}

struct App {
    engine: TuttiEngine,
    handle: Option<tutti::plugin::PluginHandle>,
    node_id: Option<tutti::NodeId>,
    window: Option<Window>,
    editor_open: bool,
    notes_sent: bool,
    is_effect: bool,
    param_snapshot: Vec<(u32, String, f32)>,
    last_poll: Instant,
}

const POLL_INTERVAL: Duration = Duration::from_millis(100);

impl App {
    fn new(engine: TuttiEngine) -> Self {
        Self {
            engine,
            handle: None,
            node_id: None,
            window: None,
            editor_open: false,
            notes_sent: false,
            is_effect: false,
            param_snapshot: Vec::new(),
            last_poll: Instant::now(),
        }
    }

    /// Snapshot all parameters via the PluginHandle.
    fn snapshot_params(handle: &tutti::plugin::PluginHandle) -> Vec<(u32, String, f32)> {
        let params = handle.parameters().unwrap_or_default();
        params
            .iter()
            .map(|info| {
                let value = handle.get_parameter(info.id).unwrap_or(0.0);
                (info.id, info.name.clone(), value)
            })
            .collect()
    }

    /// Poll parameters and log any changes.
    fn poll_params(&mut self) {
        let Some(handle) = &self.handle else { return };
        let new_snapshot = Self::snapshot_params(handle);

        for (id, name, new_value) in &new_snapshot {
            if let Some((_, _, old_value)) =
                self.param_snapshot.iter().find(|(pid, _, _)| pid == id)
            {
                if (old_value - new_value).abs() > 1e-6 {
                    println!(
                        "  [param] {} (id={}) : {:.4} -> {:.4}",
                        name, id, old_value, new_value
                    );
                }
            }
        }

        self.param_snapshot = new_snapshot;
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        println!("Searching for plugins...");
        let (unit, handle) = match try_load_plugin(&self.engine) {
            Some(result) => result,
            None => {
                eprintln!("No plugin found. Install TAL-NoiseMaker or Surge XT (CLAP/VST3/VST2).");
                event_loop.exit();
                return;
            }
        };

        let meta = handle.metadata();
        let plugin_name = meta.name.clone();
        println!(
            "Loaded: {} (has_editor: {}, supports_f64: {}, audio_io: {}in/{}out, midi: {})",
            plugin_name,
            meta.has_editor,
            meta.supports_f64,
            meta.audio_io.inputs,
            meta.audio_io.outputs,
            meta.receives_midi,
        );

        if !handle.has_editor() {
            eprintln!("Plugin '{}' has no editor GUI.", plugin_name);
            event_loop.exit();
            return;
        }

        // Add plugin to the audio graph.
        // Effects (audio_io.inputs > 0) need a source signal piped in.
        // Synths (audio_io.inputs == 0) generate their own sound from MIDI.
        let is_effect = meta.audio_io.inputs > 0;
        let node_id = if is_effect {
            println!("Plugin is an effect — feeding stereo saw wave as input.");
            self.engine.graph_mut(|net: &mut TuttiNet| {
                // A saw wave is harmonically rich (like a guitar DI signal),
                // making amp sim effects like Boogex clearly audible.
                let src = net.add(saw_hz(220.0) * 0.3 >> split::<U2>()).id();
                let fx = net.add_boxed(unit).id();
                net.pipe_all(src, fx);
                net.pipe_output(fx);
                fx
            })
        } else {
            println!("Plugin is a synth — will send MIDI notes.");
            self.engine
                .graph_mut(|net: &mut TuttiNet| net.add_boxed(unit).master())
        };
        self.node_id = Some(node_id);
        self.is_effect = is_effect;
        self.engine.transport().play();

        let window_attrs = Window::default_attributes()
            .with_title(format!("{} - Plugin Editor", plugin_name))
            .with_inner_size(winit::dpi::LogicalSize::new(800u32, 600u32));

        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("Failed to create window: {:?}", e);
                event_loop.exit();
                return;
            }
        };

        self.window = Some(window);
        self.handle = Some(handle);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                println!("Closing editor...");
                // Release held notes
                if self.notes_sent {
                    if let Some(nid) = self.node_id {
                        self.engine.note_off(nid, Note::C4);
                        self.engine.note_off(nid, Note::E4);
                        self.engine.note_off(nid, Note::G4);
                    }
                }
                if let Some(handle) = &self.handle {
                    handle.close_editor();
                }
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                if !self.editor_open {
                    let (Some(window), Some(handle)) = (&self.window, &self.handle) else {
                        return;
                    };

                    let wh = match window.window_handle() {
                        Ok(h) => h,
                        Err(e) => {
                            eprintln!("Failed to get window handle: {:?}", e);
                            return;
                        }
                    };

                    let parent_handle: u64 = match wh.as_raw() {
                        #[cfg(target_os = "macos")]
                        RawWindowHandle::AppKit(h) => h.ns_view.as_ptr() as u64,
                        #[cfg(target_os = "windows")]
                        RawWindowHandle::Win32(h) => h.hwnd.get() as u64,
                        #[cfg(target_os = "linux")]
                        RawWindowHandle::Xlib(h) => h.window as u64,
                        _ => {
                            eprintln!("Unsupported window handle type");
                            return;
                        }
                    };

                    println!("Opening plugin editor...");
                    match handle.open_editor(parent_handle) {
                        Some((w, h)) => {
                            println!("Editor opened: {}x{}", w, h);
                            let _ = window.request_inner_size(winit::dpi::LogicalSize::new(w, h));
                            self.editor_open = true;

                            // Take initial parameter snapshot
                            self.param_snapshot = Self::snapshot_params(handle);
                            println!(
                                "Tracking {} parameters. Tweak knobs to see changes.",
                                self.param_snapshot.len()
                            );

                            // Send a held chord so synth plugins produce sound.
                            // Effect plugins already have a sine wave source piped in.
                            if !self.is_effect {
                                if let Some(nid) = self.node_id {
                                    println!("Sending MIDI chord (C4 E4 G4)...");
                                    self.engine.note_on(nid, Note::C4, 100);
                                    self.engine.note_on(nid, Note::E4, 100);
                                    self.engine.note_on(nid, Note::G4, 100);
                                    self.notes_sent = true;
                                }
                            }

                            // Start polling via ControlFlow::WaitUntil
                            event_loop.set_control_flow(ControlFlow::WaitUntil(
                                Instant::now() + POLL_INTERVAL,
                            ));
                        }
                        None => {
                            eprintln!("Failed to open editor");
                        }
                    }
                }
            }
            _ => {}
        }

        // Poll parameters periodically while editor is open
        if self.editor_open && self.last_poll.elapsed() >= POLL_INTERVAL {
            // Call editor_idle so the plugin can process UI events
            if let Some(handle) = &self.handle {
                handle.editor_idle();
            }
            self.poll_params();
            self.last_poll = Instant::now();
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.editor_open {
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + POLL_INTERVAL));
        }
    }
}

fn main() -> tutti::Result<()> {
    println!("Plugin Editor Example");
    println!("=====================");
    println!();

    let engine = TuttiEngine::builder().midi().build()?;

    let event_loop = EventLoop::new().expect("Failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::new(engine);
    event_loop.run_app(&mut app).expect("Event loop error");

    Ok(())
}
