//! Transport state machine.

use super::position::{LoopRange, MusicalPosition};
use crate::compat::Arc;
use crate::AtomicFlag;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MotionState {
    #[default]
    Stopped,
    Rolling,
    FastForward,
    Rewind,
    DeclickToStop,
    DeclickToLocate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Forwards,
    Backwards,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeclickState {
    pub gain: f32,
    pub fading_out: bool,
    pub samples_remaining: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocateState {
    #[default]
    Idle,
    LocateAndStop,
    LocateAndRoll,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransportEvent {
    Play,
    Stop,
    StopWithDeclick,
    Locate(MusicalPosition),
    LocateWithDeclick(MusicalPosition),
    LocateAndPlay(MusicalPosition),
    ToggleLoop,
    SetLoopRange(LoopRange),
    ClearLoop,
    FastForward,
    Rewind,
    EndScrub,
    Reverse,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransitionResult {
    None,
    MotionChanged(MotionState),
    Locating(MusicalPosition),
    LoopModeChanged(bool),
    DirectionChanged(Direction),
    DeclickStarted,
}

pub const DEFAULT_DECLICK_SAMPLES: usize = 480;

pub struct TransportFSM {
    motion: MotionState,
    locate: LocateState,
    pending_locate: Option<MusicalPosition>,
    loop_range: Option<LoopRange>,
    loop_enabled: bool,
    state_changed: Arc<AtomicFlag>,
    prev_motion: MotionState,
    direction: Direction,
    declick: Option<DeclickState>,
    declick_samples: usize,
}

impl TransportFSM {
    pub fn new() -> Self {
        Self {
            motion: MotionState::Stopped,
            locate: LocateState::Idle,
            pending_locate: None,
            loop_range: None,
            loop_enabled: false,
            state_changed: Arc::new(AtomicFlag::new(false)),
            prev_motion: MotionState::Stopped,
            direction: Direction::Forwards,
            declick: None,
            declick_samples: DEFAULT_DECLICK_SAMPLES,
        }
    }

    pub fn transition(&mut self, event: TransportEvent) -> TransitionResult {
        use TransportEvent::*;

        let result = match event {
            Play => match self.motion {
                MotionState::Stopped
                | MotionState::FastForward
                | MotionState::Rewind
                | MotionState::DeclickToStop => {
                    self.motion = MotionState::Rolling;
                    self.locate = LocateState::Idle;
                    self.declick = None;
                    TransitionResult::MotionChanged(MotionState::Rolling)
                }
                MotionState::Rolling | MotionState::DeclickToLocate => TransitionResult::None,
            },

            Stop => match self.motion {
                MotionState::Rolling | MotionState::FastForward | MotionState::Rewind => {
                    self.motion = MotionState::Stopped;
                    self.locate = LocateState::Idle;
                    TransitionResult::MotionChanged(MotionState::Stopped)
                }
                MotionState::DeclickToStop | MotionState::DeclickToLocate => {
                    // Already stopping, just complete immediately
                    self.motion = MotionState::Stopped;
                    self.declick = None;
                    TransitionResult::MotionChanged(MotionState::Stopped)
                }
                MotionState::Stopped => TransitionResult::None,
            },

            StopWithDeclick => match self.motion {
                MotionState::Rolling => {
                    self.motion = MotionState::DeclickToStop;
                    self.declick = Some(DeclickState {
                        gain: 1.0,
                        fading_out: true,
                        samples_remaining: self.declick_samples,
                    });
                    TransitionResult::DeclickStarted
                }
                MotionState::FastForward | MotionState::Rewind => {
                    // No declick for scrub modes
                    self.motion = MotionState::Stopped;
                    TransitionResult::MotionChanged(MotionState::Stopped)
                }
                _ => TransitionResult::None,
            },

            Locate(pos) => {
                self.pending_locate = Some(pos);
                self.locate = LocateState::LocateAndStop;
                TransitionResult::Locating(pos)
            }

            LocateWithDeclick(pos) => match self.motion {
                MotionState::Rolling => {
                    self.pending_locate = Some(pos);
                    self.locate = LocateState::LocateAndStop;
                    self.motion = MotionState::DeclickToLocate;
                    self.declick = Some(DeclickState {
                        gain: 1.0,
                        fading_out: true,
                        samples_remaining: self.declick_samples,
                    });
                    TransitionResult::DeclickStarted
                }
                _ => {
                    self.pending_locate = Some(pos);
                    self.locate = LocateState::LocateAndStop;
                    TransitionResult::Locating(pos)
                }
            },

            LocateAndPlay(pos) => {
                self.pending_locate = Some(pos);
                self.locate = LocateState::LocateAndRoll;
                TransitionResult::Locating(pos)
            }

            ToggleLoop => {
                self.loop_enabled = !self.loop_enabled;
                self.state_changed.set(true);
                TransitionResult::LoopModeChanged(self.loop_enabled)
            }

            SetLoopRange(range) => {
                self.loop_range = Some(range);
                self.loop_enabled = true;
                self.state_changed.set(true);
                TransitionResult::LoopModeChanged(true)
            }

            ClearLoop => {
                self.loop_range = None;
                self.loop_enabled = false;
                self.state_changed.set(true);
                TransitionResult::LoopModeChanged(false)
            }

            FastForward => {
                self.prev_motion = self.motion;
                self.motion = MotionState::FastForward;
                TransitionResult::MotionChanged(MotionState::FastForward)
            }

            Rewind => {
                self.prev_motion = self.motion;
                self.motion = MotionState::Rewind;
                TransitionResult::MotionChanged(MotionState::Rewind)
            }

            EndScrub => {
                self.motion = self.prev_motion;
                TransitionResult::MotionChanged(self.motion)
            }

            Reverse => {
                self.direction = match self.direction {
                    Direction::Forwards => Direction::Backwards,
                    Direction::Backwards => Direction::Forwards,
                };
                TransitionResult::DirectionChanged(self.direction)
            }
        };

        if !matches!(result, TransitionResult::None) {
            self.state_changed.set(true);
        }

        result
    }
}

impl Default for TransportFSM {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_play_stop_transitions() {
        let mut fsm = TransportFSM::new();

        // Play from stopped
        let result = fsm.transition(TransportEvent::Play);
        assert!(matches!(
            result,
            TransitionResult::MotionChanged(MotionState::Rolling)
        ));

        // Stop while rolling
        let result = fsm.transition(TransportEvent::Stop);
        assert!(matches!(
            result,
            TransitionResult::MotionChanged(MotionState::Stopped)
        ));

        // Play again (idempotent)
        fsm.transition(TransportEvent::Play);
        let result = fsm.transition(TransportEvent::Play);
        assert!(matches!(result, TransitionResult::None));
    }

    #[test]
    fn test_locate_events() {
        let mut fsm = TransportFSM::new();

        let target = MusicalPosition::from_beats(8.0);
        let result = fsm.transition(TransportEvent::Locate(target));
        assert!(matches!(result, TransitionResult::Locating(_)));

        let target = MusicalPosition::from_beats(4.0);
        let result = fsm.transition(TransportEvent::LocateAndPlay(target));
        assert!(matches!(result, TransitionResult::Locating(_)));
    }

    #[test]
    fn test_loop_events() {
        let mut fsm = TransportFSM::new();

        // Set loop range
        let result = fsm.transition(TransportEvent::SetLoopRange(LoopRange::new(0.0, 8.0)));
        assert!(matches!(result, TransitionResult::LoopModeChanged(true)));

        // Toggle off
        let result = fsm.transition(TransportEvent::ToggleLoop);
        assert!(matches!(result, TransitionResult::LoopModeChanged(false)));

        // Toggle on
        let result = fsm.transition(TransportEvent::ToggleLoop);
        assert!(matches!(result, TransitionResult::LoopModeChanged(true)));

        // Clear loop
        let result = fsm.transition(TransportEvent::ClearLoop);
        assert!(matches!(result, TransitionResult::LoopModeChanged(false)));
    }

    #[test]
    fn test_scrub_events() {
        let mut fsm = TransportFSM::new();
        fsm.transition(TransportEvent::Play);

        // Fast forward
        let result = fsm.transition(TransportEvent::FastForward);
        assert!(matches!(
            result,
            TransitionResult::MotionChanged(MotionState::FastForward)
        ));

        // End scrub - should return to rolling
        let result = fsm.transition(TransportEvent::EndScrub);
        assert!(matches!(
            result,
            TransitionResult::MotionChanged(MotionState::Rolling)
        ));

        // Rewind
        let result = fsm.transition(TransportEvent::Rewind);
        assert!(matches!(
            result,
            TransitionResult::MotionChanged(MotionState::Rewind)
        ));

        // End scrub again
        let result = fsm.transition(TransportEvent::EndScrub);
        assert!(matches!(
            result,
            TransitionResult::MotionChanged(MotionState::Rolling)
        ));
    }

    #[test]
    fn test_declick_events() {
        let mut fsm = TransportFSM::new();
        fsm.transition(TransportEvent::Play);

        // Stop with declick
        let result = fsm.transition(TransportEvent::StopWithDeclick);
        assert!(matches!(result, TransitionResult::DeclickStarted));

        // Locate with declick
        fsm.transition(TransportEvent::Play);
        let target = MusicalPosition::from_beats(4.0);
        let result = fsm.transition(TransportEvent::LocateWithDeclick(target));
        assert!(matches!(result, TransitionResult::DeclickStarted));
    }

    #[test]
    fn test_reverse_direction() {
        let mut fsm = TransportFSM::new();

        let result = fsm.transition(TransportEvent::Reverse);
        assert!(matches!(
            result,
            TransitionResult::DirectionChanged(Direction::Backwards)
        ));

        let result = fsm.transition(TransportEvent::Reverse);
        assert!(matches!(
            result,
            TransitionResult::DirectionChanged(Direction::Forwards)
        ));
    }
}
