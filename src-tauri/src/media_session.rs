//! Desktop SMTC adapter. WinRT ownership stays on one MTA thread and the
//! playback mailbox remains the sole place where native actions are applied.
use crate::{
    contract::{LiveState, PlayerState, PlayerView},
    playback::{desktop::DesktopPlayer, service::SystemAction},
};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::watch;
use windows::{
    Foundation::{TimeSpan, TypedEventHandler},
    Media::{
        MediaPlaybackStatus as Status, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
        SystemMediaTransportControls as Controls, SystemMediaTransportControlsButton as Button,
        SystemMediaTransportControlsButtonPressedEventArgs,
        SystemMediaTransportControlsTimelineProperties as Timeline,
    },
    Storage::Streams::{DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference},
    Win32::{
        Foundation::HWND,
        System::WinRT::{
            ISystemMediaTransportControlsInterop, RO_INIT_MULTITHREADED, RoGetActivationFactory,
            RoInitialize, RoUninitialize,
        },
    },
    core::HSTRING,
};

pub(crate) struct MediaSession(Mutex<Option<std::thread::JoinHandle<()>>>);

impl MediaSession {
    /// Called after the playback worker has closed its projection channel.
    pub(crate) fn shutdown(&self) {
        if let Ok(mut slot) = self.0.lock()
            && let Some(worker) = slot.take()
        {
            let _ = worker.join();
        }
    }
}

pub(crate) fn start(
    window: &tauri::WebviewWindow,
    player: Weak<DesktopPlayer>,
    directory: Option<PathBuf>,
) -> Option<(watch::Sender<Option<Projection>>, MediaSession)> {
    let hwnd = window.hwnd().ok()?.0 as usize;
    let (sender, receiver) = watch::channel(None);
    let worker = std::thread::Builder::new()
        .name("echo-system-media".into())
        .spawn(move || {
            if run(hwnd, player, receiver, directory.as_deref()).is_err() {
                eprintln!("ECHO Windows media controls unavailable");
            }
        })
        .ok()?;
    Some((sender, MediaSession(Mutex::new(Some(worker)))))
}

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: this guard is created only after successful initialization
        // and lives longer than every WinRT object on this thread.
        unsafe { RoUninitialize() };
    }
}

struct Session {
    controls: Controls,
    button: Option<i64>,
    seek: Option<i64>,
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(token) = self.seek {
            let _ = self.controls.RemovePlaybackPositionChangeRequested(token);
        }
        if let Some(token) = self.button {
            let _ = self.controls.RemoveButtonPressed(token);
        }
        if let Ok(display) = self.controls.DisplayUpdater() {
            let _ = display.ClearAll();
            let _ = display.Update();
        }
        let _ = self.controls.SetIsEnabled(false);
    }
}

fn run(
    hwnd: usize,
    player: Weak<DesktopPlayer>,
    mut updates: watch::Receiver<Option<Projection>>,
    directory: Option<&Path>,
) -> windows::core::Result<()> {
    // SAFETY: a fresh thread owns this apartment. The HWND belongs to the
    // app and GetForWindow validates it before creating the broker session.
    unsafe { RoInitialize(RO_INIT_MULTITHREADED)? };
    let _apartment = Apartment;
    let controls = unsafe {
        let factory: ISystemMediaTransportControlsInterop =
            RoGetActivationFactory(&HSTRING::from("Windows.Media.SystemMediaTransportControls"))?;
        factory.GetForWindow::<Controls>(HWND(hwnd as *mut _))?
    };
    let mut session = Session {
        controls,
        button: None,
        seek: None,
    };
    let buttons = player.clone();
    session.button = Some(session.controls.ButtonPressed(&TypedEventHandler::<
        Controls,
        SystemMediaTransportControlsButtonPressedEventArgs,
    >::new(move |_, args| {
        if let Some(player) = buttons.upgrade()
            && let Some(args) = args.as_ref()
            && let Some(action) = button_action(args.Button()?)
        {
            player.system_action(action);
        }
        Ok(())
    }))?);
    // Zero means Windows currently has no seekable presentation. Queued
    // seeks are checked against the generation again by the playback owner.
    let advertised = Arc::new(AtomicU64::new(0));
    let seek_generation = advertised.clone();
    session.seek = Some(
        session
            .controls
            .PlaybackPositionChangeRequested(&TypedEventHandler::<
                Controls,
                PlaybackPositionChangeRequestedEventArgs,
            >::new(move |_, args| {
                let generation = seek_generation.load(Ordering::Acquire);
                if generation != 0
                    && let Some(args) = args.as_ref()
                    && let Some(player) = player.upgrade()
                    && let Ok(ticks) = u64::try_from(args.RequestedPlaybackPosition()?.Duration)
                {
                    player.system_action(SystemAction::Seek {
                        generation,
                        milliseconds: ticks / 10_000,
                    });
                }
                Ok(())
            }))?,
    );

    let mut previous: Option<Projection> = None;
    let mut media = MediaDetails::default();
    while tauri::async_runtime::block_on(updates.changed()).is_ok() {
        let current = updates.borrow_and_update().clone();
        if let Some(current) = current
            && previous.as_ref() != Some(&current)
        {
            if previous
                .as_ref()
                .is_none_or(|old| !old.same_source(&current))
            {
                advertised.store(0, Ordering::Release);
                media = directory
                    .map(|path| MediaDetails::load(path, &current.source))
                    .unwrap_or_default();
                // The cover load can finish after a replacement was queued.
                // Only the current source and generation can publish its bytes.
                let latest = updates.borrow();
                if latest
                    .as_ref()
                    .is_none_or(|latest| !latest.same_source(&current))
                {
                    previous = None;
                    continue;
                }
            }
            let timeline = previous.as_ref().is_none_or(|old: &Projection| {
                old.generation != current.generation || old.timeline != current.timeline
            });
            current.apply(&session.controls, previous.as_ref(), timeline, &media)?;
            advertised.store(
                if current.timeline.is_some() {
                    current.generation
                } else {
                    0
                },
                Ordering::Release,
            );
            previous = Some(current);
        }
    }
    Ok(())
}

fn button_action(button: Button) -> Option<SystemAction> {
    Some(match button {
        Button::Play => SystemAction::Play,
        Button::Pause => SystemAction::Pause,
        Button::Stop => SystemAction::Stop,
        Button::Next => SystemAction::Next,
        Button::Previous => SystemAction::Previous,
        _ => return None,
    })
}

#[derive(Clone, PartialEq)]
enum MediaSource {
    None,
    Local {
        track: Option<u32>,
        cover: Option<u32>,
    },
    Radio(String),
}

#[derive(Default)]
struct MediaDetails {
    album: String,
    cover: Option<RandomAccessStreamReference>,
}

impl MediaDetails {
    fn load(directory: &Path, source: &MediaSource) -> Self {
        use crate::library::{artwork, open_reader, query};
        if matches!(source, MediaSource::None) {
            return Self::default();
        }
        let Ok(db) = open_reader(&directory.join("library.sqlite3")) else {
            return Self::default();
        };
        let _ = db.busy_timeout(std::time::Duration::from_millis(100));
        let (album, raster) = match source {
            MediaSource::Local { track, cover } => {
                let album = track
                    .and_then(|id| query::track(&db, id).ok().flatten())
                    .map(|row| row.album)
                    .unwrap_or_default();
                let raster = cover
                    .and_then(|id| query::artwork_hash(&db, id).ok().flatten())
                    .and_then(|hash| artwork::load(&directory.join("artwork"), &hash).ok());
                (album, raster)
            }
            MediaSource::Radio(id) => (
                String::new(),
                crate::radio::artwork::system_thumbnail(&db, id),
            ),
            MediaSource::None => return Self::default(),
        };
        Self {
            album,
            cover: raster.and_then(|bytes| thumbnail(&bytes).ok()),
        }
    }
}

fn thumbnail(bytes: &[u8]) -> windows::core::Result<RandomAccessStreamReference> {
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream.GetOutputStreamAt(0)?)?;
    writer.WriteBytes(bytes)?;
    writer.StoreAsync()?.join()?;
    writer.DetachStream()?;
    stream.Seek(0)?;
    RandomAccessStreamReference::CreateFromStream(&stream)
}

#[derive(Clone, PartialEq)]
pub(crate) struct Projection {
    generation: u64,
    status: Status,
    title: String,
    artist: String,
    source: MediaSource,
    can_play: bool,
    can_pause: bool,
    next: bool,
    previous: bool,
    timeline: Option<(u64, u64)>,
}

impl From<&PlayerView> for Projection {
    fn from(view: &PlayerView) -> Self {
        let track = view
            .current
            .as_deref()
            .and_then(|id| view.tracks.iter().find(|track| track.id == id));
        let state = &view.state;
        let status = match state {
            PlayerState::Playing => Status::Playing,
            PlayerState::Paused => Status::Paused,
            PlayerState::Loading | PlayerState::Buffering | PlayerState::Stopping => {
                Status::Changing
            }
            PlayerState::Idle | PlayerState::Ended | PlayerState::Error => Status::Stopped,
        };
        let can_play = match &view.live {
            Some(live) => matches!(
                live.status,
                LiveState::Paused | LiveState::Offline | LiveState::OutputUnavailable
            ),
            None => {
                !view.tracks.is_empty()
                    && matches!(
                        state,
                        PlayerState::Paused
                            | PlayerState::Idle
                            | PlayerState::Ended
                            | PlayerState::Error
                    )
            }
        };
        let (title, artist) = match &view.live {
            Some(live) => (
                live.title
                    .clone()
                    .unwrap_or_else(|| live.station.name.clone()),
                live.station.name.clone(),
            ),
            None => track
                .map(|track| {
                    (
                        track.title.clone(),
                        track.subtitle.clone().unwrap_or_default(),
                    )
                })
                .unwrap_or_default(),
        };
        Self {
            generation: view.generation.parse().unwrap_or(0),
            status,
            title,
            artist,
            source: match &view.live {
                Some(live) => MediaSource::Radio(live.station.id.clone()),
                None => track.map_or(MediaSource::None, |track| MediaSource::Local {
                    track: track.track_id,
                    cover: track.artwork_id,
                }),
            },
            can_play,
            can_pause: matches!(
                state,
                PlayerState::Playing | PlayerState::Loading | PlayerState::Buffering
            ),
            next: view.live.is_none() && view.has_next,
            previous: view.live.is_none() && view.has_previous,
            timeline: if view.live.is_none()
                && view.seekable
                && matches!(
                    state,
                    PlayerState::Playing | PlayerState::Paused | PlayerState::Buffering
                ) {
                view.duration_ms
                    .map(|end| (view.position_ms as u64, end as u64))
            } else {
                None
            },
        }
    }
}

impl Projection {
    fn same_source(&self, other: &Self) -> bool {
        self.generation == other.generation && self.source == other.source
    }

    fn apply(
        &self,
        controls: &Controls,
        previous: Option<&Self>,
        update_timeline: bool,
        media: &MediaDetails,
    ) -> windows::core::Result<()> {
        if previous.is_none_or(|old| {
            !self.same_source(old) || old.title != self.title || old.artist != self.artist
        }) {
            let display = controls.DisplayUpdater()?;
            display.ClearAll()?;
            if !self.title.is_empty() {
                display.SetType(MediaPlaybackType::Music)?;
                let music = display.MusicProperties()?;
                music.SetTitle(&HSTRING::from(self.title.as_str()))?;
                music.SetArtist(&HSTRING::from(self.artist.as_str()))?;
                music.SetAlbumTitle(&HSTRING::from(media.album.as_str()))?;
                if let Some(cover) = &media.cover {
                    // Artwork failure must not disable transport controls.
                    let _ = display.SetThumbnail(cover);
                }
            }
            display.Update()?;
        }
        if update_timeline {
            let (position, end) = self.timeline.unwrap_or_default();
            let end_time = span(end);
            let timeline = Timeline::new()?;
            timeline.SetPosition(span(position.min(end)))?;
            timeline.SetEndTime(end_time)?;
            timeline.SetMaxSeekTime(end_time)?;
            controls.UpdateTimelineProperties(&timeline)?;
        }
        if previous.is_none_or(|old| {
            old.status != self.status
                || old.can_play != self.can_play
                || old.can_pause != self.can_pause
                || old.next != self.next
                || old.previous != self.previous
                || old.title.is_empty() != self.title.is_empty()
        }) {
            controls.SetPlaybackStatus(self.status)?;
            controls.SetIsPlayEnabled(self.can_play)?;
            controls.SetIsPauseEnabled(self.can_pause)?;
            controls.SetIsStopEnabled(self.can_pause || self.status == Status::Paused)?;
            controls.SetIsNextEnabled(self.next)?;
            controls.SetIsPreviousEnabled(self.previous)?;
            controls.SetIsEnabled(!self.title.is_empty() || self.can_play)?;
        }
        Ok(())
    }
}

fn span(milliseconds: u64) -> TimeSpan {
    TimeSpan {
        Duration: milliseconds.min(i64::MAX as u64 / 10_000) as i64 * 10_000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Storage::Streams::DataReader;

    #[test]
    fn native_thumbnail_stream_remains_readable_after_writer_is_dropped() {
        std::thread::spawn(|| {
            // SAFETY: this test thread has no previous COM initialization.
            unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.unwrap();
            let _apartment = Apartment;
            let bytes = [0xff, 0xd8, 0xff, 0xd9];
            let reference = thumbnail(&bytes).unwrap();
            let stream = reference.OpenReadAsync().unwrap().join().unwrap();
            let input = stream.GetInputStreamAt(0).unwrap();
            let reader = DataReader::CreateDataReader(&input).unwrap();
            assert_eq!(reader.LoadAsync(4).unwrap().join().unwrap(), 4);
            let mut actual = [0; 4];
            reader.ReadBytes(&mut actual).unwrap();
            assert_eq!(actual, bytes);
        })
        .join()
        .unwrap();
    }
}
