//! Bounded command mailbox and the exclusive native/SQLite playback worker.
use super::{
    Effect, Event, Playback, PlaybackError, Snapshot, State,
    decoder::{Decoder, DecoderError, DecoderEvent, Output, StartOptions},
    history::{ListenEvent, Listening},
    persistence::{CheckpointError, Checkpoints},
    queue::{EntryId, Item, Repeat},
    source::{SourceError, open_selected},
};
use crate::{
    contract::{
        LiveRadio, LiveState, PlayerFault, PlayerState, PlayerView, QueueTrack, RepeatMode, Station,
    },
    native::NativeLibrary,
    radio::{RadioStream, StreamStatus, TransportFailure},
};
use std::{
    collections::{HashSet, hash_map::RandomState},
    hash::BuildHasher,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const PROGRESS: Duration = Duration::from_millis(500);
const CHECKPOINT: Duration = Duration::from_secs(10);

/// Called with a library track id when its queue entry could not be opened.
pub type UnavailableHook = Arc<dyn Fn(u32) + Send + Sync>;

/// Callbacks into the library. Both must return without blocking.
#[derive(Clone, Default)]
pub struct Hooks {
    pub unavailable: Option<UnavailableHook>,
    /// Confirmed listening progress for a library track.
    pub listened: Option<Arc<dyn Fn(ListenEvent) + Send + Sync>>,
    pub(crate) system: Option<tokio::sync::watch::Sender<Option<crate::media_session::Projection>>>,
}

/// Native intents are resolved in mailbox order against the current source.
pub(crate) enum SystemAction {
    Play,
    Pause,
    Next,
    Previous,
    Stop,
    Seek { generation: u64, milliseconds: u64 },
}

/// Intents from a trusted host adapter. This type is deliberately not an IPC
/// payload. A file picker must authorize paths before any queue insertion.
pub enum Command {
    Snapshot,
    Subscribe(tauri::ipc::Channel<PlayerView>),
    AddPicked(Vec<PathBuf>, PickMode),
    /// Rust-resolved station reference. Only the gated radio adapter may call this.
    PlayStation {
        station: Station,
        source: String,
    },
    #[cfg(test)]
    PlayFixtureStation {
        station: Station,
        address: std::net::SocketAddr,
    },
    /// Library rows resolved by Rust. `start` selects which item begins when
    /// the mode replaces the queue.
    AddLibrary {
        items: Vec<Item>,
        mode: PickMode,
        start: usize,
        /// Seed for shuffled playback of the replaced queue.
        shuffle: Option<u64>,
    },
    Replace(Vec<PathBuf>),
    Append(PathBuf),
    PlayNext(PathBuf),
    Remove(EntryId),
    MoveBefore(EntryId, Option<EntryId>),
    ClearUpcoming,
    Play(EntryId),
    Next,
    Previous,
    Pause(bool),
    Seek {
        generation: u64,
        milliseconds: u64,
    },
    Stop,
    Volume(u8),
    Mute(bool),
    Repeat(Repeat),
    Shuffle(Option<u64>),
}

#[derive(Clone, Copy)]
pub enum PickMode {
    Listen,
    Append,
    Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceError {
    Domain(PlaybackError),
    Source(SourceError),
    Decoder(DecoderError),
    Checkpoint,
    Busy,
    Unavailable,
}

impl From<ServiceError> for PlayerFault {
    fn from(value: ServiceError) -> Self {
        match value {
            ServiceError::Domain(_) => Self::Invalid,
            ServiceError::Source(_) => Self::FileUnavailable,
            ServiceError::Decoder(_) => Self::DecodeFailed,
            ServiceError::Busy => Self::Busy,
            ServiceError::Checkpoint | ServiceError::Unavailable => Self::Unavailable,
        }
    }
}

type Reply = Result<PlayerView, ServiceError>;

enum Message {
    Wake,
    System(SystemAction),
    Request(Command, tokio::sync::oneshot::Sender<Reply>),
}

/// Owns a single worker, with independent shutdown signaling even if the
/// mailbox is full. The owning host joins it off the application UI thread.
pub struct Service {
    sender: SyncSender<Message>,
    stopping: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<Result<(), CheckpointError>>>>,
}

impl Service {
    pub fn start(
        library: Arc<NativeLibrary>,
        database: PathBuf,
        output: Output,
    ) -> std::io::Result<Self> {
        Self::start_with_hooks(library, database, output, Hooks::default())
    }

    pub fn start_with_hooks(
        library: Arc<NativeLibrary>,
        database: PathBuf,
        output: Output,
        hooks: Hooks,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(16);
        let wake = sender.clone();
        let stopping = Arc::new(AtomicBool::new(false));
        let flag = stopping.clone();
        let thread = thread::Builder::new()
            .name("echo-playback".to_string())
            .spawn(move || {
                let store = Checkpoints::open(&database)?;
                let playback = match store.restore() {
                    Ok(playback) => playback,
                    Err(CheckpointError::Invalid) => {
                        // Preserve a corrupted record until the user makes a new
                        // explicit mutation. Automatic idle/disconnect does not overwrite it.
                        Playback::default()
                    }
                    Err(error) => return Err(error),
                };
                let restored = playback
                    .queue
                    .current()
                    .zip((playback.position > 0).then_some(playback.position));
                Worker {
                    library,
                    playback,
                    decoder: None,
                    live: None,
                    output,
                    wake,
                    store,
                    dirty: false,
                    last_save: Instant::now(),
                    subscriber: None,
                    sent_revision: None,
                    failure: None,
                    restored,
                    hooks,
                    listen: Listening::default(),
                    unopened: HashSet::new(),
                }
                .run(receiver, &flag)
            })?;
        Ok(Self {
            sender,
            stopping,
            thread: Mutex::new(Some(thread)),
        })
    }

    pub async fn request(&self, command: Command) -> Reply {
        let (send, receive) = tokio::sync::oneshot::channel();
        self.sender
            .try_send(Message::Request(command, send))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => ServiceError::Busy,
                mpsc::TrySendError::Disconnected(_) => ServiceError::Unavailable,
            })?;
        receive.await.map_err(|_| ServiceError::Unavailable)?
    }

    pub fn shutdown(&self) -> Result<(), ServiceError> {
        self.request_shutdown();
        let thread = self
            .thread
            .lock()
            .map_err(|_| ServiceError::Unavailable)?
            .take();
        if let Some(thread) = thread {
            thread
                .join()
                .map_err(|_| ServiceError::Unavailable)?
                .map_err(|_| ServiceError::Checkpoint)?;
        }
        Ok(())
    }

    pub(crate) fn system_action(&self, action: SystemAction) {
        let _ = self.sender.try_send(Message::System(action));
    }

    fn request_shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        let _ = self.sender.try_send(Message::Wake);
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct Worker {
    library: Arc<NativeLibrary>,
    playback: Playback,
    decoder: Option<Decoder>,
    live: Option<LiveSession>,
    output: Output,
    wake: SyncSender<Message>,
    store: Checkpoints,
    dirty: bool,
    last_save: Instant,
    subscriber: Option<tauri::ipc::Channel<PlayerView>>,
    sent_revision: Option<u64>,
    failure: Option<PlayerFault>,
    restored: Option<(EntryId, u64)>,
    hooks: Hooks,
    listen: Listening,
    /// Entries whose last open failed, shown as unavailable in the queue.
    unopened: HashSet<EntryId>,
}

struct LiveSession {
    view: LiveRadio,
    source: String,
    transport: Option<StreamStatus>,
    retries: u8,
    retry_at: Option<Instant>,
    heard: bool,
    started: bool,
    deadline: Option<Instant>,
    #[cfg(test)]
    fixture: Option<std::net::SocketAddr>,
}

#[cfg(test)]
mod live_tests;

impl Worker {
    fn run(
        &mut self,
        receiver: Receiver<Message>,
        stopping: &AtomicBool,
    ) -> Result<(), CheckpointError> {
        let mut next_progress = Instant::now();
        self.publish();
        while !stopping.load(Ordering::Acquire) {
            let timeout = if self.decoder.is_some() {
                Some(next_progress.saturating_duration_since(Instant::now()))
            } else if self.dirty {
                Some(CHECKPOINT.saturating_sub(self.last_save.elapsed()))
            } else {
                None
            };
            let retry = self
                .live
                .as_ref()
                .and_then(|live| live.retry_at)
                .map(|deadline| deadline.saturating_duration_since(Instant::now()));
            let timeout = match (timeout, retry) {
                (Some(tick), Some(retry)) => Some(tick.min(retry)),
                (tick, retry) => tick.or(retry),
            };
            let message = match timeout {
                Some(timeout) => receiver.recv_timeout(timeout),
                None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            if stopping.load(Ordering::Acquire) {
                break;
            }
            match message {
                Ok(Message::Request(command, reply)) => {
                    let _ = reply.send(self.request(command));
                }
                Ok(Message::System(action)) => {
                    if let Some(command) = self.system_command(action) {
                        let _ = self.request(command);
                    }
                }
                Ok(Message::Wake) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.events();
            if self
                .live
                .as_ref()
                .and_then(|live| live.retry_at)
                .is_some_and(|retry| retry <= Instant::now())
            {
                let _ = self.start_live();
            }
            if Instant::now() >= next_progress {
                self.sample();
                next_progress = Instant::now() + PROGRESS;
            }
            if self.dirty && self.last_save.elapsed() >= CHECKPOINT && self.checkpoint().is_err() {
                // Back off after a transient storage failure and keep audio alive.
                self.last_save = Instant::now();
                eprintln!("ECHO playback checkpoint failed");
            }
            self.publish();
        }
        // Persist before native teardown. Destroying a libmpv core can block in
        // a synchronous read on a stalled disk driver, and nothing in-process can
        // reliably interrupt that kernel wait. The saved session is already
        // durable if the host's exit watchdog has to end the process.
        self.finish_listening();
        let saved = self.checkpoint();
        self.decoder.take();
        saved
    }

    fn request(&mut self, command: Command) -> Reply {
        let result = match command {
            Command::Snapshot => return Ok(self.view()),
            Command::Subscribe(channel) => {
                self.subscriber = Some(channel);
                self.sent_revision = None;
                return Ok(self.view());
            }
            Command::PlayStation { station, source } => {
                self.select_station(station, source);
                self.start_live()?;
                Some(())
            }
            #[cfg(test)]
            Command::PlayFixtureStation { station, address } => {
                self.select_station(station, String::new());
                self.live.as_mut().unwrap().fixture = Some(address);
                self.start_live()?;
                Some(())
            }
            Command::Pause(paused) if self.live.is_some() => {
                if paused {
                    self.decoder.take();
                    if let Some(live) = &mut self.live {
                        live.transport = None;
                        live.retry_at = None;
                        live.view.status = LiveState::Paused;
                        live.view.title = None;
                        live.deadline = None;
                    }
                } else {
                    if let Some(live) = &mut self.live {
                        live.retries = 0;
                    }
                    self.start_live()?;
                }
                Some(())
            }
            Command::Stop if self.live.is_some() => {
                self.decoder.take();
                self.live = None;
                Some(())
            }
            Command::Next
            | Command::Previous
            | Command::Seek { .. }
            | Command::Repeat(_)
            | Command::Shuffle(_)
                if self.live.is_some() =>
            {
                return Err(ServiceError::Domain(PlaybackError::InvalidState));
            }
            Command::AddPicked(paths, mode) => {
                self.add_picked(paths, mode)?;
                Some(())
            }
            Command::AddLibrary {
                items,
                mode,
                start,
                shuffle,
            } => {
                self.add_items(items, mode, start, shuffle)?;
                Some(())
            }
            Command::Replace(paths) => {
                if self.decoder.is_some() {
                    return Err(ServiceError::Domain(PlaybackError::InvalidState));
                }
                if self.playback.queue.replace(paths) {
                    self.restored = None;
                    Some(())
                } else {
                    None
                }
            }
            Command::Append(path) => self.playback.queue.append(path).map(drop),
            Command::PlayNext(path) => self.playback.queue.play_next(path).map(drop),
            Command::Remove(id) => self.playback.queue.remove(id).then_some(()),
            Command::MoveBefore(id, before) => {
                self.playback.queue.move_before(id, before).then_some(())
            }
            Command::ClearUpcoming => {
                self.playback.queue.clear_upcoming();
                Some(())
            }
            Command::Repeat(repeat) => {
                self.playback.queue.repeat = repeat;
                Some(())
            }
            Command::Shuffle(seed) => {
                self.playback.queue.shuffle(seed);
                Some(())
            }
            command => {
                let effect = match command {
                    Command::Play(entry) => self.playback.load(entry),
                    Command::Next => self.playback.next_track(),
                    Command::Previous => self.playback.previous(),
                    Command::Pause(paused) => self.playback.pause(paused),
                    Command::Seek {
                        generation,
                        milliseconds,
                    } => {
                        if generation != self.playback.snapshot().generation {
                            return Err(ServiceError::Domain(PlaybackError::InvalidState));
                        }
                        self.playback.seek(milliseconds)
                    }
                    Command::Stop => self.playback.stop(),
                    Command::Volume(volume) => self.playback.volume(volume),
                    Command::Mute(mute) => Ok(self.playback.mute(mute)),
                    _ => return Err(ServiceError::Domain(PlaybackError::InvalidState)),
                }
                .map_err(ServiceError::Domain)?;
                self.apply(effect)?;
                Some(())
            }
        };
        if result.is_none() {
            return Err(ServiceError::Domain(PlaybackError::InvalidState));
        }
        self.playback.changed();
        self.dirty = true;
        Ok(self.view())
    }

    fn add_picked(&mut self, paths: Vec<PathBuf>, mode: PickMode) -> Result<(), ServiceError> {
        let items = paths.into_iter().map(|path| (path, None)).collect();
        self.add_items(items, mode, 0, None)
    }

    fn add_items(
        &mut self,
        items: Vec<Item>,
        mode: PickMode,
        start: usize,
        shuffle: Option<u64>,
    ) -> Result<(), ServiceError> {
        if items.is_empty() {
            return Ok(());
        }
        // The picker validates the selection off this worker, and library paths
        // come from Rust's own index. Either way the file is reopened and
        // reauthorized on actual playback, including restored entries.
        let invalid = ServiceError::Domain(PlaybackError::InvalidState);
        let mut queue = self.playback.queue.clone();
        let first = match mode {
            PickMode::Listen => {
                if !queue.replace_items(items) {
                    return Err(invalid);
                }
                self.restored = None;
                self.unopened.clear();
                match shuffle {
                    Some(seed) => queue.start_shuffled(None, seed),
                    // Shuffle that was already on keeps shuffling what follows.
                    None if queue.shuffled() => {
                        let seed = self.playback.snapshot().revision;
                        queue.start_shuffled(Some(start), seed)
                    }
                    None => queue.order().get(start).or(queue.order().first()).copied(),
                }
            }
            PickMode::Append => {
                for item in items {
                    queue.append_item(item).ok_or(invalid)?;
                }
                None
            }
            PickMode::Next => {
                for item in items.into_iter().rev() {
                    queue.play_next_item(item).ok_or(invalid)?;
                }
                None
            }
        };
        self.playback.queue = queue;
        if let Some(first) = first {
            let effect = self.playback.load(first).map_err(ServiceError::Domain)?;
            self.apply(effect)?;
        }
        Ok(())
    }

    fn publish(&mut self) {
        let revision = self.playback.snapshot().revision;
        if self.sent_revision == Some(revision) {
            return;
        }
        if self.subscriber.is_some() || self.hooks.system.is_some() {
            let view = self.view();
            if let Some(system) = &self.hooks.system {
                let projection = crate::media_session::Projection::from(&view);
                system.send_if_modified(|previous| {
                    if previous.as_ref() == Some(&projection) {
                        return false;
                    }
                    *previous = Some(projection);
                    true
                });
            }
            if let Some(channel) = &self.subscriber
                && channel.send(view).is_err()
            {
                self.subscriber = None;
            }
            self.sent_revision = Some(revision);
        }
    }

    fn system_command(&self, action: SystemAction) -> Option<Command> {
        let state = self.playback.snapshot().state;
        let live = self.live.as_ref().map(|session| session.view.status);
        match action {
            SystemAction::Play => match live {
                Some(LiveState::Paused | LiveState::Offline | LiveState::OutputUnavailable) => {
                    Some(Command::Pause(false))
                }
                Some(_) => None,
                None if state == State::Paused => Some(Command::Pause(false)),
                None if matches!(state, State::Idle | State::Ended | State::Error) => self
                    .playback
                    .queue
                    .current()
                    .or_else(|| self.playback.queue.order().first().copied())
                    .map(Command::Play),
                None => None,
            },
            SystemAction::Pause
                if live.is_none_or(|status| {
                    matches!(
                        status,
                        LiveState::Connecting
                            | LiveState::Buffering
                            | LiveState::Playing
                            | LiveState::Reconnecting
                    )
                }) =>
            {
                Some(Command::Pause(true))
            }
            SystemAction::Next => Some(Command::Next),
            SystemAction::Previous => Some(Command::Previous),
            SystemAction::Stop => Some(Command::Stop),
            SystemAction::Seek {
                generation,
                milliseconds,
            } => Some(Command::Seek {
                generation,
                milliseconds,
            }),
            SystemAction::Pause => None,
        }
    }

    fn view(&self) -> PlayerView {
        let Snapshot {
            revision,
            generation,
            state,
            position,
            duration,
            seekable,
            volume,
            muted,
        } = self.playback.snapshot();
        let queue = &self.playback.queue;
        let live = self.live.as_ref().map(|live| live.view.clone());
        let rows: std::collections::HashMap<_, _> = queue
            .entries()
            .iter()
            .map(|entry| (entry.id, entry))
            .collect();
        PlayerView {
            revision: revision.to_string(),
            generation: generation.to_string(),
            state: match live.as_ref().map(|live| live.status) {
                Some(LiveState::Connecting | LiveState::Reconnecting) => PlayerState::Loading,
                Some(LiveState::Playing) => PlayerState::Playing,
                Some(LiveState::Buffering) => PlayerState::Buffering,
                Some(LiveState::Paused) => PlayerState::Paused,
                Some(_) => PlayerState::Error,
                None => match state {
                    State::Idle => PlayerState::Idle,
                    State::Loading => PlayerState::Loading,
                    State::Playing => PlayerState::Playing,
                    State::Paused => PlayerState::Paused,
                    State::Buffering => PlayerState::Buffering,
                    State::Stopping => PlayerState::Stopping,
                    State::Ended => PlayerState::Ended,
                    State::Error => PlayerState::Error,
                },
            },
            position_ms: if live.is_some() {
                0.0
            } else {
                safe_milliseconds(position)
            },
            duration_ms: duration.filter(|_| live.is_none()).map(safe_milliseconds),
            seekable: live.is_none() && seekable,
            volume,
            muted,
            repeat: match queue.repeat {
                Repeat::Off => RepeatMode::Off,
                Repeat::One => RepeatMode::One,
                Repeat::All => RepeatMode::All,
            },
            shuffled: queue.shuffled(),
            tracks: queue
                .order()
                .iter()
                .filter_map(|id| {
                    rows.get(id)
                        .map(|entry| queue_row(*id, entry, self.unopened.contains(id)))
                })
                .collect(),
            current: queue
                .current()
                .filter(|_| live.is_none())
                .map(EntryId::decimal),
            has_previous: live.is_none() && queue.neighbor(true, false).is_some(),
            has_next: live.is_none() && queue.neighbor(false, false).is_some(),
            failure: self.failure,
            live,
        }
    }

    fn apply(&mut self, effect: Effect) -> Result<(), ServiceError> {
        if let Effect::Load { generation, entry } = effect {
            self.finish_listening();
            self.failure = None;
            self.decoder.take();
            self.live = None;
            let resume_at = self
                .restored
                .take()
                .filter(|(id, _)| *id == entry)
                .map_or(0, |(_, position)| position);
            let start: Result<Decoder, ServiceError> = (|| {
                let entry = self
                    .playback
                    .queue
                    .entries()
                    .iter()
                    .find(|candidate| candidate.id == entry)
                    .ok_or(ServiceError::Domain(PlaybackError::UnknownEntry))?;
                let file = open_selected(entry.path()).map_err(ServiceError::Source)?;
                let mut decoder = Decoder::start_configured(
                    self.library.clone(),
                    file,
                    generation,
                    StartOptions {
                        output: self.output,
                        volume: self.playback.volume,
                        muted: self.playback.muted,
                        resume_at,
                    },
                )
                .map_err(ServiceError::Decoder)?;
                let wake = self.wake.clone();
                decoder.notify(move || {
                    let _ = wake.try_send(Message::Wake);
                });
                Ok(decoder)
            })();
            match start {
                Ok(decoder) => {
                    self.decoder = Some(decoder);
                    self.unopened.remove(&entry);
                    return Ok(());
                }
                Err(error) => {
                    self.failure = Some(error.into());
                    if matches!(error, ServiceError::Source(_)) {
                        self.unopened.insert(entry);
                    }
                    if let (ServiceError::Source(_), Some(hook)) = (error, &self.hooks.unavailable)
                    {
                        let track = self
                            .playback
                            .queue
                            .entries()
                            .iter()
                            .find(|candidate| candidate.id == entry)
                            .and_then(|candidate| candidate.track().map(|t| t.track_id));
                        if let Some(track) = track {
                            hook(track);
                        }
                    }
                    self.playback.event(generation, Event::Failed);
                    self.dirty = true;
                    return Err(error);
                }
            }
        }
        if self.decoder.is_none() {
            match effect {
                Effect::Volume(value) => self.playback.mixer_confirmed(value, self.playback.muted),
                Effect::Mute(value) => self.playback.mixer_confirmed(self.playback.volume, value),
                _ => return Err(ServiceError::Domain(PlaybackError::InvalidState)),
            }
            return Ok(());
        }
        let decoder = self.decoder.as_mut().ok_or(ServiceError::Unavailable)?;
        let result = match effect {
            Effect::Pause(paused) => decoder.pause(paused),
            Effect::Seek(position) => decoder.seek(position),
            Effect::Stop => decoder.stop(),
            Effect::Volume(volume) => decoder.volume(volume),
            Effect::Mute(mute) => decoder.mute(mute),
            Effect::Load { .. } => return Err(ServiceError::Domain(PlaybackError::InvalidState)),
        };
        if let Err(error) = result {
            let snapshot = self.playback.snapshot();
            if snapshot.state == State::Stopping {
                self.decoder.take();
                self.playback.event(snapshot.generation, Event::Failed);
            }
            return Err(ServiceError::Decoder(error));
        }
        if let (Some(volume), Some(muted)) = (decoder.mixer_volume(), decoder.is_muted()) {
            self.playback
                .mixer_confirmed(volume.round().clamp(0.0, 100.0) as u8, muted);
        }
        if let Effect::Pause(_) = effect {
            let snapshot = self.playback.snapshot();
            if let Some(paused) = decoder.is_paused() {
                if paused && snapshot.state != State::Loading {
                    self.playback.event(snapshot.generation, Event::Paused);
                } else if !paused
                    && snapshot.state == State::Paused
                    && decoder.has_restarted()
                    && decoder.audio_ready()
                {
                    self.playback.event(snapshot.generation, Event::Started);
                }
            }
        }
        Ok(())
    }

    fn events(&mut self) {
        // Limit one dispatch turn so a burst of decoder events can't starve input.
        for _ in 0..64 {
            let Some(decoder) = self.decoder.as_mut() else {
                return;
            };
            let (generation, event) = decoder.event(Duration::ZERO);
            if self.live.is_some() {
                if event == DecoderEvent::None {
                    return;
                }
                self.live_event(generation, event);
                continue;
            }
            let state = self.playback.snapshot().state;
            let (event, terminal) = match event {
                DecoderEvent::None => return,
                DecoderEvent::Loaded if decoder.is_paused() == Some(true) => (Event::Paused, false),
                DecoderEvent::Restarted if decoder.audio_ready() => (
                    if decoder.is_paused() == Some(true) {
                        Event::Paused
                    } else {
                        Event::Started
                    },
                    false,
                ),
                DecoderEvent::Buffering(true) => (Event::Buffering, false),
                DecoderEvent::Buffering(false)
                    if state == State::Buffering
                        && decoder.has_restarted()
                        && decoder.audio_ready() =>
                {
                    let event = if decoder.is_paused() == Some(true) {
                        Event::Paused
                    } else {
                        Event::Started
                    };
                    (event, false)
                }
                DecoderEvent::End { .. } if state == State::Stopping => (Event::Stopped, true),
                DecoderEvent::End {
                    reason: 0,
                    error: 0,
                } => (Event::Eof, true),
                DecoderEvent::End { reason: 2, .. } => (Event::Stopped, true),
                DecoderEvent::End { error: -14, .. } => {
                    // MPV_ERROR_AO_INIT_FAILED from the pinned client.h.
                    self.failure = Some(PlayerFault::OutputUnavailable);
                    // A lost or unplugged endpoint is not a property of the track.
                    // Remember where it stopped so a manual Play after the user
                    // restores an output continues instead of restarting.
                    self.restored = output_recovery_point(
                        self.playback.queue.current(),
                        self.playback.snapshot().position,
                    );
                    (Event::Failed, true)
                }
                DecoderEvent::End { .. } | DecoderEvent::Shutdown => {
                    self.failure = Some(PlayerFault::DecodeFailed);
                    (Event::Failed, true)
                }
                _ => continue,
            };
            let follow = self.playback.event(generation, event);
            self.dirty = true;
            if terminal {
                self.finish_listening();
                self.decoder.take();
                if matches!(
                    self.playback.snapshot().state,
                    State::Loading | State::Playing | State::Buffering
                ) && follow.is_none()
                {
                    self.playback.event(generation, Event::Failed);
                }
            }
            if let Some(effect) = follow {
                let _ = self.apply(effect);
            }
        }
        let _ = self.wake.try_send(Message::Wake);
    }

    fn sample(&mut self) {
        if let Some(live) = &mut self.live {
            if let Some(transport) = &live.transport {
                if live
                    .deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    // Bound the total probe time even when regular reads keep
                    // succeeding with data the decoder cannot identify.
                    self.decoder.take();
                    live.transport = None;
                    live.view.status = LiveState::Offline;
                    live.view.title = None;
                    live.deadline = None;
                    self.playback.changed();
                    return;
                }
                let report = transport.read();
                let mut changed = live.view.title != report.title;
                live.view.title = report.title;
                if live.view.status == LiveState::Connecting && report.bytes_ready {
                    live.view.status = LiveState::Buffering;
                    changed = true;
                }
                if changed {
                    self.playback.changed();
                }
            }
            return;
        }
        let Some(decoder) = self.decoder.as_ref() else {
            return;
        };
        let Some(position) = decoder.position() else {
            return;
        };
        let decoder_generation = decoder.generation();
        let before = self.playback.snapshot();
        self.playback.event(
            decoder.generation(),
            Event::Position {
                milliseconds: (position.max(0.0) * 1000.0) as u64,
                duration: decoder
                    .duration()
                    .map(|seconds| (seconds.max(0.0) * 1000.0) as u64),
                seekable: decoder.is_seekable() == Some(true),
            },
        );
        let after = self.playback.snapshot();
        self.dirty |= before.position != after.position;
        self.hear(decoder_generation, after);
    }

    fn select_station(&mut self, station: Station, source: String) {
        let previous = self.playback.snapshot();
        if self.live.is_none() {
            self.restored = self
                .playback
                .queue
                .current()
                .zip((previous.position > 0).then_some(previous.position));
        }
        self.finish_listening();
        self.decoder.take();
        self.failure = None;
        self.live = Some(LiveSession {
            view: LiveRadio {
                station,
                status: LiveState::Connecting,
                title: None,
            },
            source,
            transport: None,
            retries: 0,
            retry_at: None,
            heard: false,
            started: false,
            deadline: None,
            #[cfg(test)]
            fixture: None,
        });
    }

    fn start_live(&mut self) -> Result<(), ServiceError> {
        self.decoder.take();
        let generation = self.playback.start_live().map_err(ServiceError::Domain)?;
        let live = self.live.as_mut().ok_or(ServiceError::Unavailable)?;
        live.retry_at = None;
        live.view.status = if live.retries == 0 {
            LiveState::Connecting
        } else {
            LiveState::Reconnecting
        };
        live.view.title = None;
        live.transport = None;
        live.started = false;
        live.deadline = Some(Instant::now() + Duration::from_secs(15));
        let runtime = tauri::async_runtime::handle();
        let stream = RadioStream::start(runtime.inner(), &live.source);
        #[cfg(test)]
        let stream = live.fixture.map_or(stream, |addr| {
            Ok(crate::radio::fixtures::stream(runtime.inner(), addr))
        });
        let Ok(stream) = stream else {
            live.view.status = LiveState::Rejected;
            return Ok(());
        };
        live.transport = Some(stream.status());
        match Decoder::start_live(
            self.library.clone(),
            stream,
            generation,
            StartOptions {
                output: self.output,
                volume: self.playback.volume,
                muted: self.playback.muted,
                resume_at: 0,
            },
        ) {
            Ok(mut decoder) => {
                let wake = self.wake.clone();
                decoder.notify(move || {
                    let _ = wake.try_send(Message::Wake);
                });
                self.decoder = Some(decoder);
            }
            Err(_) => {
                live.view.status = LiveState::Offline;
                live.transport = None;
            }
        }
        Ok(())
    }

    fn live_event(&mut self, generation: u64, event: DecoderEvent) {
        if generation != self.playback.snapshot().generation {
            return;
        }
        let (Some(live), Some(decoder)) = (&mut self.live, &self.decoder) else {
            return;
        };
        let old = live.view.status;
        let mut terminal = false;
        match event {
            DecoderEvent::Loaded if !decoder.supported_live_format() => {
                live.view.status = LiveState::Unsupported;
                terminal = true;
            }
            DecoderEvent::Restarted | DecoderEvent::Buffering(false)
                if decoder.has_restarted() && decoder.audio_ready() =>
            {
                live.view.status = LiveState::Playing;
                live.started = true;
                live.deadline = None;
                if !live.heard {
                    live.heard = true;
                    if self.store.heard_station(&live.view.station.id).is_err() {
                        eprintln!("ECHO could not save a recent station");
                    }
                }
            }
            DecoderEvent::Buffering(true) => live.view.status = LiveState::Buffering,
            DecoderEvent::End { error: -14, .. } => {
                live.view.status = LiveState::OutputUnavailable;
                terminal = true;
            }
            DecoderEvent::End { reason, .. } => {
                let failure = live
                    .transport
                    .as_ref()
                    .and_then(|transport| transport.read().failure);
                live.view.status = match failure {
                    Some(TransportFailure::Policy) => LiveState::Rejected,
                    Some(TransportFailure::Unsupported) => LiveState::Unsupported,
                    None if !live.started && reason != 0 => LiveState::Unsupported,
                    _ if live.retries < 3 => {
                        let base = 800 * (1_u64 << live.retries);
                        let jitter = RandomState::new().hash_one(generation) % (base / 4);
                        live.retries += 1;
                        live.retry_at = Some(Instant::now() + Duration::from_millis(base + jitter));
                        LiveState::Reconnecting
                    }
                    _ => LiveState::Offline,
                };
                terminal = true;
            }
            DecoderEvent::Shutdown => {
                live.view.status = LiveState::Offline;
                terminal = true;
            }
            _ => {}
        }
        if terminal {
            self.decoder.take();
            live.transport = None;
            live.view.title = None;
        }
        if terminal || live.view.status != old {
            self.playback.changed();
        }
    }

    /// Accumulates time only while the decoder confirms playback of the
    /// generation it belongs to, so a stale event cannot credit another track.
    fn hear(&mut self, generation: u64, snapshot: Snapshot) {
        if generation != snapshot.generation {
            return;
        }
        let track = self
            .playback
            .queue
            .current()
            .and_then(|id| self.playback.queue.entries().iter().find(|e| e.id == id))
            .and_then(|entry| entry.track().map(|track| track.track_id));
        let event = self.listen.sample(
            Instant::now(),
            generation,
            track,
            snapshot.state == State::Playing,
            snapshot.position,
            snapshot.duration,
        );
        if let (Some(event), Some(hook)) = (event, &self.hooks.listened) {
            hook(event);
        }
    }

    fn finish_listening(&mut self) {
        if let (Some(event), Some(hook)) = (self.listen.finish(), &self.hooks.listened) {
            hook(event);
        }
    }

    fn checkpoint(&mut self) -> Result<(), CheckpointError> {
        if self.dirty {
            self.store.save(&self.playback)?;
        }
        self.dirty = false;
        self.last_save = Instant::now();
        Ok(())
    }
}

/// Display text is derived from the library title for indexed rows and from
/// the file name otherwise. Hostile characters never reach the renderer.
fn queue_row(id: EntryId, entry: &super::queue::Entry, unavailable: bool) -> QueueTrack {
    let display = |text: &str| -> String {
        text.chars()
            .filter(|character| {
                !character.is_control()
                    && !matches!(
                        character,
                        '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
                    )
            })
            .take(160)
            .collect()
    };
    match entry.track() {
        Some(track) => QueueTrack {
            id: id.decimal(),
            title: display(&track.title),
            subtitle: Some(display(&track.artist)).filter(|artist| !artist.is_empty()),
            artwork_id: track.artwork_id,
            track_id: Some(track.track_id),
            unavailable,
        },
        None => QueueTrack {
            id: id.decimal(),
            title: entry.path().file_name().map_or_else(
                || "Local audio".into(),
                |name| display(&name.to_string_lossy()),
            ),
            subtitle: None,
            artwork_id: None,
            track_id: None,
            unavailable,
        },
    }
}

fn safe_milliseconds(value: u64) -> f64 {
    value.min((1_u64 << 53) - 1) as f64
}

/// Where a manual retry should resume after the audio output disappeared.
/// Position zero resumes nothing, which keeps the ordinary "play from start"
/// path for failures that happen before any audio was confirmed.
fn output_recovery_point(current: Option<EntryId>, position: u64) -> Option<(EntryId, u64)> {
    current.zip((position > 0).then_some(position))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_loss_resumes_only_known_progress() {
        let entry = EntryId::parse("7").unwrap();
        assert_eq!(
            output_recovery_point(Some(entry), 4_200),
            Some((entry, 4_200))
        );
        assert_eq!(output_recovery_point(Some(entry), 0), None);
        assert_eq!(output_recovery_point(None, 4_200), None);
    }
}
