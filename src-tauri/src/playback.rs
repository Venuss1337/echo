//! Local playback domain. The native owner implements effects and supplies confirmed events.
pub mod history;
pub mod queue;

use queue::{EntryId, Queue, Repeat};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Loading,
    Playing,
    Paused,
    Buffering,
    Stopping,
    Ended,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Started,
    Paused,
    Buffering,
    Position {
        milliseconds: u64,
        duration: Option<u64>,
        seekable: bool,
    },
    Eof,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Load { generation: u64, entry: EntryId },
    Pause(bool),
    Seek(u64),
    Stop,
    Volume(u8),
    Mute(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackError {
    UnknownEntry,
    InvalidState,
    InvalidPosition,
    InvalidVolume,
    Exhausted,
}

#[derive(Debug)]
pub struct Playback {
    pub queue: Queue,
    state: State,
    generation: u64,
    revision: u64,
    position: u64,
    duration: Option<u64>,
    seekable: bool,
    volume: u8,
    muted: bool,
}

/// Decoder-confirmed state. Consumers must serialize the counters losslessly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub state: State,
    pub generation: u64,
    pub revision: u64,
    pub position: u64,
    pub duration: Option<u64>,
    pub seekable: bool,
    pub volume: u8,
    pub muted: bool,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            queue: Queue::default(),
            state: State::Idle,
            generation: 0,
            revision: 0,
            position: 0,
            duration: None,
            seekable: false,
            volume: 75,
            muted: false,
        }
    }
}

impl Playback {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            state: self.state,
            generation: self.generation,
            revision: self.revision,
            position: self.position,
            duration: self.duration,
            seekable: self.seekable,
            volume: self.volume,
            muted: self.muted,
        }
    }

    pub fn load(&mut self, entry: EntryId) -> Result<Effect, PlaybackError> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(PlaybackError::Exhausted)?;
        self.queue
            .select(entry)
            .ok_or(PlaybackError::UnknownEntry)?;
        self.generation = generation;
        self.state = State::Loading;
        self.position = 0;
        self.duration = None;
        self.seekable = false;
        self.changed();
        Ok(Effect::Load { generation, entry })
    }

    #[cfg(windows)]
    fn start_live(&mut self) -> Result<u64, PlaybackError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(PlaybackError::Exhausted)?;
        // A station owns its own presentation state. Keep the local position
        // for explicit local resume, but never surface it as live progress.
        self.state = State::Idle;
        self.duration = None;
        self.seekable = false;
        self.changed();
        Ok(self.generation)
    }

    pub fn pause(&self, paused: bool) -> Result<Effect, PlaybackError> {
        match (self.state, paused) {
            (State::Playing | State::Loading | State::Buffering, true)
            | (State::Paused | State::Loading, false) => Ok(Effect::Pause(paused)),
            _ => Err(PlaybackError::InvalidState),
        }
    }

    pub fn seek(&self, milliseconds: u64) -> Result<Effect, PlaybackError> {
        if !matches!(
            self.state,
            State::Playing | State::Paused | State::Buffering
        ) || !self.seekable
        {
            return Err(PlaybackError::InvalidState);
        }
        if self.duration.is_none_or(|duration| milliseconds > duration) {
            return Err(PlaybackError::InvalidPosition);
        }
        Ok(Effect::Seek(milliseconds))
    }

    pub fn stop(&mut self) -> Result<Effect, PlaybackError> {
        if !matches!(
            self.state,
            State::Loading | State::Playing | State::Paused | State::Buffering
        ) {
            return Err(PlaybackError::InvalidState);
        }
        self.state = State::Stopping;
        self.changed();
        Ok(Effect::Stop)
    }

    pub fn next_track(&mut self) -> Result<Effect, PlaybackError> {
        self.navigate(false)
    }

    pub fn previous(&mut self) -> Result<Effect, PlaybackError> {
        self.navigate(true)
    }

    fn navigate(&mut self, previous: bool) -> Result<Effect, PlaybackError> {
        let entry = self
            .queue
            .neighbor(previous, false)
            .ok_or(PlaybackError::InvalidState)?;
        self.load(entry)
    }

    pub fn volume(&self, volume: u8) -> Result<Effect, PlaybackError> {
        if volume > 100 {
            return Err(PlaybackError::InvalidVolume);
        }
        Ok(Effect::Volume(volume))
    }

    pub fn mute(&self, muted: bool) -> Effect {
        Effect::Mute(muted)
    }

    /// Only call after the native owner confirms the mixer values.
    pub fn mixer_confirmed(&mut self, volume: u8, muted: bool) {
        if volume <= 100 && (volume != self.volume || muted != self.muted) {
            self.volume = volume;
            self.muted = muted;
            self.changed();
        }
    }

    /// The generation must come from the original decoder session or a native
    /// correlation key. Never copy the currently displayed generation here.
    pub fn event(&mut self, generation: u64, event: Event) -> Option<Effect> {
        if generation != self.generation || generation == 0 {
            return None;
        }
        let before = self.snapshot();
        let active = matches!(
            self.state,
            State::Loading | State::Playing | State::Paused | State::Buffering
        );
        match event {
            Event::Started if active => self.state = State::Playing,
            Event::Paused if active => self.state = State::Paused,
            Event::Buffering if matches!(self.state, State::Loading | State::Playing) => {
                self.state = State::Buffering
            }
            Event::Position {
                milliseconds,
                duration,
                seekable,
            } if active => {
                self.duration = duration;
                self.position = milliseconds.min(duration.unwrap_or(u64::MAX));
                self.seekable = seekable;
            }
            Event::Stopped if self.state == State::Stopping => {
                self.state = State::Idle;
                self.position = 0;
                self.duration = None;
                self.seekable = false;
            }
            Event::Eof
                if matches!(
                    self.state,
                    State::Playing | State::Paused | State::Buffering
                ) =>
            {
                self.state = State::Ended;
                self.changed();
                return self
                    .queue
                    .neighbor(false, true)
                    .and_then(|entry| self.load(entry).ok());
            }
            // Fail in place. A manual Next can recover and repeat-one never
            // retries a permanent error.
            Event::Failed if active || self.state == State::Stopping => self.state = State::Error,
            _ => return None,
        }
        if self.snapshot() != before {
            self.changed();
        }
        None
    }

    pub fn settings(&mut self, repeat: Repeat, shuffle: Option<u64>) {
        self.queue.repeat = repeat;
        self.queue.shuffle(shuffle);
        self.changed();
    }

    fn changed(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

#[cfg(windows)]
pub mod decoder;

#[cfg(windows)]
pub mod persistence;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn pair() -> (Playback, EntryId, EntryId) {
        let mut player = Playback::default();
        let first = player.queue.append("test.wav".into()).unwrap();
        let second = player.queue.append("test.wav".into()).unwrap();
        (player, first, second)
    }

    #[test]
    fn play_replacement_never_labels_old_events_current() {
        let (mut player, first, second) = pair();
        assert_ne!(first, second);
        assert_eq!(
            player.load(first),
            Ok(Effect::Load {
                generation: 1,
                entry: first
            })
        );
        assert_eq!(player.pause(true), Ok(Effect::Pause(true)));
        assert_eq!(player.snapshot().state, State::Loading);
        assert_eq!(
            player.load(second),
            Ok(Effect::Load {
                generation: 2,
                entry: second
            })
        );
        let loading = player.snapshot();
        for event in [
            Event::Started,
            Event::Paused,
            Event::Eof,
            Event::Failed,
            Event::Position {
                milliseconds: 800,
                duration: Some(900),
                seekable: true,
            },
        ] {
            assert_eq!(player.event(1, event), None);
            assert_eq!(player.snapshot(), loading);
        }
        assert_eq!(player.seek(500), Err(PlaybackError::InvalidState));
        player.event(2, Event::Started);
        assert_eq!(player.snapshot().state, State::Playing);
        player.event(
            2,
            Event::Position {
                milliseconds: 60,
                duration: Some(1000),
                seekable: true,
            },
        );
        assert_eq!(player.seek(1001), Err(PlaybackError::InvalidPosition));
        assert_eq!(player.seek(1000), Ok(Effect::Seek(1000)));
        assert_eq!(player.volume(101), Err(PlaybackError::InvalidVolume));
    }

    #[test]
    fn natural_eof_advances_once_and_error_never_loops() {
        let (mut player, first, second) = pair();
        player.load(first).unwrap();
        player.event(1, Event::Started);
        assert_eq!(
            player.event(1, Event::Eof),
            Some(Effect::Load {
                generation: 2,
                entry: second
            })
        );
        assert_eq!(player.event(1, Event::Eof), None);
        player.settings(Repeat::One, None);
        assert_eq!(player.event(2, Event::Failed), None);
        assert_eq!(player.event(2, Event::Eof), None);
        assert_eq!(player.snapshot().state, State::Error);
        assert_eq!(
            player.previous(),
            Ok(Effect::Load {
                generation: 3,
                entry: first
            })
        );
    }

    #[test]
    fn stop_during_loading_waits_for_confirmation() {
        let (mut player, first, _) = pair();
        player.load(first).unwrap();
        assert_eq!(player.stop(), Ok(Effect::Stop));
        assert_eq!(player.event(1, Event::Started), None);
        assert_eq!(player.snapshot().state, State::Stopping);
        assert_eq!(player.stop(), Err(PlaybackError::InvalidState));
        player.event(1, Event::Stopped);
        assert_eq!(player.snapshot().state, State::Idle);
    }

    #[test]
    fn play_next_shuffle_and_mutation_respect_current_identity() {
        let (mut player, first, second) = pair();
        player.load(first).unwrap();
        assert!(!player.queue.remove(first));
        let next = player.queue.play_next(PathBuf::from("next.m4a")).unwrap();
        assert_eq!(player.queue.order(), &[first, next, second]);
        player.settings(Repeat::All, Some(90));
        let order = player.queue.order().to_vec();
        player.settings(Repeat::All, Some(91));
        assert_eq!(player.queue.order(), order);
        let upcoming = order[1];
        assert_eq!(
            player.next_track(),
            Ok(Effect::Load {
                generation: 2,
                entry: upcoming
            })
        );
        assert_eq!(
            player.previous(),
            Ok(Effect::Load {
                generation: 3,
                entry: first
            })
        );
        player.settings(Repeat::Off, None);
        assert_eq!(player.queue.order(), &[first, next, second]);
        player.queue.clear_upcoming();
        assert_eq!(player.queue.order(), &[first]);
    }
}

#[cfg(windows)]
pub mod source;

#[cfg(windows)]
pub mod service;

#[cfg(windows)]
pub(crate) mod desktop;
