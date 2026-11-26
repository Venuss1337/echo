//! Decides when confirmed playback counts as listening.
//!
//! A play is recorded once per playback generation, after the listener has
//! actually heard 30 seconds or half the track, whichever is shorter. Loading,
//! pausing, buffering and seeking add nothing. Repeating a track starts a new
//! generation and can therefore be recorded again.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const THRESHOLD: Duration = Duration::from_secs(30);
/// A track with no known duration must reach the full threshold.
const MINIMUM: Duration = Duration::from_secs(1);
/// Samples arrive twice a second. Larger position steps are seeks or stalls.
const MAX_STEP: Duration = Duration::from_secs(3);
/// Refresh a recorded play periodically so a crash loses little.
const REFRESH: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListenEvent {
    pub generation: u64,
    pub track_id: u32,
    /// Unix seconds when the generation was first heard.
    pub started_at: i64,
    pub listened_ms: u64,
}

#[derive(Default)]
pub struct Listening {
    generation: u64,
    track: Option<u32>,
    started_at: i64,
    heard: Duration,
    last: Option<(Instant, u64)>,
    reported: Option<Duration>,
}

impl Listening {
    pub fn sample(
        &mut self,
        now: Instant,
        generation: u64,
        track: Option<u32>,
        playing: bool,
        position: u64,
        duration: Option<u64>,
    ) -> Option<ListenEvent> {
        if generation != self.generation || track != self.track {
            *self = Self {
                generation,
                track,
                started_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_secs() as i64),
                ..Self::default()
            };
        }
        if !playing {
            self.last = None;
            return None;
        }
        if let Some((then, before)) = self.last.replace((now, position)) {
            let wall = now.saturating_duration_since(then);
            let moved = Duration::from_millis(position.saturating_sub(before));
            // Credit what both clocks agree on. A forward jump is a seek.
            if moved <= wall + Duration::from_millis(1500) {
                self.heard += moved.min(wall).min(MAX_STEP);
            }
        }
        let needed = duration
            .map_or(THRESHOLD, |total| {
                THRESHOLD.min(Duration::from_millis(total) / 2)
            })
            .max(MINIMUM);
        let due = match self.reported {
            None => self.heard >= needed,
            Some(before) => self.heard >= before + REFRESH,
        };
        due.then(|| self.report()).flatten()
    }

    /// Final total for a play that was already recorded.
    pub fn finish(&mut self) -> Option<ListenEvent> {
        let event = self
            .reported
            .is_some_and(|before| self.heard > before)
            .then(|| self.report())
            .flatten();
        *self = Self::default();
        event
    }

    fn report(&mut self) -> Option<ListenEvent> {
        self.reported = Some(self.heard);
        Some(ListenEvent {
            generation: self.generation,
            track_id: self.track?,
            started_at: self.started_at,
            listened_ms: self.heard.as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Clock(Instant);

    /// Plays `seconds` from `from`, sampling twice a second.
    fn play(
        listening: &mut Listening,
        clock: &mut Clock,
        generation: u64,
        from: u64,
        seconds: u64,
        duration: u64,
    ) -> Vec<ListenEvent> {
        (0..=seconds * 2)
            .filter_map(|step| {
                clock.0 += Duration::from_millis(500);
                listening.sample(
                    clock.0,
                    generation,
                    Some(7),
                    true,
                    from + step * 500,
                    Some(duration),
                )
            })
            .collect()
    }

    #[test]
    fn records_once_after_thirty_seconds_and_reports_the_final_total() {
        let (mut l, mut c) = (Listening::default(), Clock(Instant::now()));
        assert!(play(&mut l, &mut c, 1, 0, 29, 300_000).is_empty());
        let events = play(&mut l, &mut c, 1, 29_000, 10, 300_000);
        assert_eq!(events.len(), 1);
        assert!((30_000..31_000).contains(&events[0].listened_ms));
        let last = l.finish().unwrap();
        assert!(last.listened_ms > events[0].listened_ms);
        assert_eq!(l.finish(), None);
    }

    #[test]
    fn short_tracks_need_half_and_skipped_tracks_record_nothing() {
        let (mut l, mut c) = (Listening::default(), Clock(Instant::now()));
        assert_eq!(play(&mut l, &mut c, 1, 0, 6, 10_000).len(), 1);
        l.finish();
        assert!(play(&mut l, &mut c, 2, 0, 4, 10_000).is_empty());
        assert_eq!(l.finish(), None, "below the threshold leaves no trace");
    }

    #[test]
    fn seeking_pausing_and_stalls_do_not_count() {
        let (mut l, mut c) = (Listening::default(), Clock(Instant::now()));
        // Seeking through the whole track hears nothing.
        for step in 0..20 {
            c.0 += Duration::from_millis(500);
            assert_eq!(
                l.sample(c.0, 1, Some(7), true, step * 14_000, Some(300_000)),
                None
            );
        }
        // Wall time without position, and position without Playing, add nothing.
        for _ in 0..200 {
            c.0 += Duration::from_millis(500);
            assert_eq!(
                l.sample(c.0, 1, Some(7), true, 280_000, Some(300_000)),
                None
            );
            assert_eq!(
                l.sample(c.0, 1, Some(7), false, 290_000, Some(300_000)),
                None
            );
        }
        assert_eq!(l.finish(), None);
    }

    #[test]
    fn repeats_are_separate_generations_and_picked_files_are_ignored() {
        let (mut l, mut c) = (Listening::default(), Clock(Instant::now()));
        let first = play(&mut l, &mut c, 1, 0, 8, 12_000);
        let second = play(&mut l, &mut c, 2, 0, 8, 12_000);
        assert_eq!((first.len(), second.len()), (1, 1));
        assert_ne!(first[0].generation, second[0].generation);
        for step in 0..100 {
            c.0 += Duration::from_millis(500);
            assert_eq!(l.sample(c.0, 3, None, true, step * 500, Some(60_000)), None);
        }
    }
}
