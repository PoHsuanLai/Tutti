//! Plugin instance trait and processing types.
//!
//! Re-exported from `tutti-plugin::instance`. The canonical definitions live there
//! so both the client (in-process bridge) and server can use them.

pub use tutti_plugin::instance::*;

#[cfg(test)]
mod tests {
    use super::*;
    use tutti_midi_io::{Channel, ChannelVoiceMsg};
    use tutti_plugin::protocol::{
        MidiEvent, NoteExpressionChanges, ParameterChanges, TransportInfo,
    };

    #[test]
    fn test_process_context_default() {
        let ctx = ProcessContext::new();
        assert!(ctx.midi_events.is_empty());
        assert!(ctx.param_changes.is_none());
        assert!(ctx.note_expression.is_none());
        assert!(ctx.transport.is_none());
    }

    #[test]
    fn test_process_context_builder_chain() {
        let events = [MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::NoteOn {
                note: 60,
                velocity: 100,
            },
        }];
        let params = ParameterChanges::new();
        let note_expr = NoteExpressionChanges::new();
        let transport = TransportInfo::default();

        let ctx = ProcessContext::new()
            .midi(&events)
            .params(&params)
            .note_expression(&note_expr)
            .transport(&transport);

        assert_eq!(ctx.midi_events.len(), 1);
        assert!(ctx.param_changes.is_some());
        assert!(ctx.note_expression.is_some());
        assert!(ctx.transport.is_some());
    }

    #[test]
    fn test_process_output_default() {
        let output = ProcessOutput::default();
        assert!(output.midi_events.is_empty());
        assert!(output.param_changes.is_empty());
        assert!(output.note_expression.is_empty());
    }

    #[test]
    fn test_process_context_midi_only() {
        let events = [MidiEvent {
            frame_offset: 0,
            channel: Channel::Ch1,
            msg: ChannelVoiceMsg::NoteOn {
                note: 60,
                velocity: 100,
            },
        }];

        let ctx = ProcessContext::new().midi(&events);

        assert_eq!(ctx.midi_events.len(), 1);
        assert!(ctx.param_changes.is_none());
        assert!(ctx.note_expression.is_none());
        assert!(ctx.transport.is_none());
    }
}
