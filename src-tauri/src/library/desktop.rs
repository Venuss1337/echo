//! Main-window command boundary for the library. The renderer names rows by
//! id and never supplies, receives for opening, or influences a file path:
//! folders come from the native picker and playable paths from Rust's index.

use super::{
    query::{self, QueryError},
    service::LibraryService,
    user,
};
use crate::{
    contract::{
        AlbumPage, AlbumQuery, AlbumRow, ArtistPage, ArtistQuery, ArtistRow, ArtworkSize,
        FavoriteAlbum, FavoriteTarget, HistoryPage, HistoryScope, HomeView, LibraryFault,
        LibrarySummary, MediaPage, Palette, PlayTarget, PlayerFault, PlayerView, PlaylistAct,
        PlaylistCard, PlaylistOutcome, PlaylistPage, QueueMode, SearchResults, TrackPage,
        TrackQuery, TrackRow,
    },
    playback::{
        desktop::{DesktopPlayer, pick_on_ui_thread},
        queue::{Item, TrackRef},
        service::{Command, PickMode},
    },
};
use std::{
    collections::hash_map::RandomState,
    hash::BuildHasher,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{State, WebviewWindow, ipc::Channel};

#[derive(Default)]
pub(crate) struct LibraryHost {
    ready: AtomicBool,
    exiting: AtomicBool,
    service: Mutex<Option<Arc<LibraryService>>>,
}

impl LibraryHost {
    /// Starts the service after the database has been bootstrapped. Failure
    /// leaves every command reporting `unavailable`.
    pub(crate) fn initialize(&self, directory: Option<&Path>) -> Option<Arc<LibraryService>> {
        let service = directory
            .and_then(|directory| LibraryService::start(directory).ok())
            .map(Arc::new);
        if let Ok(mut slot) = self.service.lock() {
            slot.clone_from(&service);
        }
        self.ready.store(true, Ordering::Release);
        service
    }

    pub(crate) fn begin_exit(&self) {
        self.exiting.store(true, Ordering::Release);
    }

    pub(crate) fn shutdown(&self) {
        let service = self.service.lock().ok().and_then(|mut slot| slot.take());
        if let Some(service) = service {
            service.shutdown();
        }
    }

    fn service(&self, window: &WebviewWindow) -> Result<Arc<LibraryService>, LibraryFault> {
        if window.label() != "main" || self.exiting.load(Ordering::Acquire) {
            return Err(LibraryFault::Unavailable);
        }
        self.service
            .lock()
            .map_err(|_| LibraryFault::Unavailable)?
            .clone()
            .ok_or_else(|| {
                if self.ready.load(Ordering::Acquire) {
                    LibraryFault::Unavailable
                } else {
                    LibraryFault::Opening
                }
            })
    }
}

/// Runs blocking SQLite or filesystem work off the async runtime's threads.
async fn blocking<T: Send + 'static>(
    host: &LibraryHost,
    window: &WebviewWindow,
    work: impl FnOnce(Arc<LibraryService>) -> Result<T, LibraryFault> + Send + 'static,
) -> Result<T, LibraryFault> {
    let service = host.service(window)?;
    tauri::async_runtime::spawn_blocking(move || work(service))
        .await
        .map_err(|_| LibraryFault::Unavailable)?
}

#[tauri::command]
pub(crate) async fn library_summary(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
) -> Result<LibrarySummary, LibraryFault> {
    blocking(&host, &window, |service| service.summary()).await
}

#[tauri::command]
pub(crate) async fn library_subscribe(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    channel: Channel<LibrarySummary>,
) -> Result<LibrarySummary, LibraryFault> {
    blocking(&host, &window, move |service| {
        service.subscribe(Box::new(move |summary| {
            channel.send(summary.clone()).is_ok()
        }))
    })
    .await
}

/// Opens the native folder picker and adds each chosen folder as a root.
#[tauri::command]
pub(crate) async fn library_add_root(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
) -> Result<LibrarySummary, LibraryFault> {
    let service = host.service(&window)?;
    let folders = pick_on_ui_thread(&window, true)
        .await
        .map_err(|fault| match fault {
            PlayerFault::Busy => LibraryFault::Busy,
            _ => LibraryFault::PickerUnavailable,
        })?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut first_error = None;
        let mut added = false;
        for folder in folders {
            match service.add_root(folder) {
                Ok(()) => added = true,
                Err(error) => first_error = first_error.or(Some(error)),
            }
        }
        match first_error {
            Some(error) if !added => Err(error),
            _ => service.summary(),
        }
    })
    .await
    .map_err(|_| LibraryFault::Unavailable)?
}

#[tauri::command]
pub(crate) async fn library_remove_root(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    id: u32,
) -> Result<LibrarySummary, LibraryFault> {
    blocking(&host, &window, move |service| {
        service.remove_root(id)?;
        service.summary()
    })
    .await
}

#[tauri::command]
pub(crate) async fn library_scan(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    root: Option<u32>,
) -> Result<(), LibraryFault> {
    blocking(&host, &window, move |service| service.scan(root)).await
}

#[tauri::command]
pub(crate) async fn library_cancel_scan(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
) -> Result<(), LibraryFault> {
    blocking(&host, &window, |service| service.cancel_scan()).await
}

#[tauri::command]
pub(crate) async fn library_tracks(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    query: TrackQuery,
) -> Result<TrackPage, LibraryFault> {
    blocking(&host, &window, move |service| service.tracks(&query)).await
}

#[tauri::command]
pub(crate) async fn library_albums(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    query: AlbumQuery,
) -> Result<AlbumPage, LibraryFault> {
    blocking(&host, &window, move |service| service.albums(&query)).await
}

#[tauri::command]
pub(crate) async fn library_artists(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    query: ArtistQuery,
) -> Result<ArtistPage, LibraryFault> {
    blocking(&host, &window, move |service| service.artists(&query)).await
}

#[tauri::command]
pub(crate) async fn library_search(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    text: String,
    limit: u32,
) -> Result<SearchResults, LibraryFault> {
    blocking(&host, &window, move |service| service.search(&text, limit)).await
}

/// Raw JPEG bytes for an artwork id: at most 512 KiB, or 2 MiB when large.
#[tauri::command]
pub(crate) async fn library_artwork(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    id: u32,
    size: ArtworkSize,
) -> Result<tauri::ipc::Response, LibraryFault> {
    blocking(&host, &window, move |service| service.artwork(id, size))
        .await
        .map(tauri::ipc::Response::new)
}

/// Resolves a track or album to local files through the index and hands them
/// to the one playback coordinator. Files are reauthorized when opened.
#[tauri::command]
pub(crate) async fn library_play(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    player: State<'_, Arc<DesktopPlayer>>,
    target: PlayTarget,
    mode: QueueMode,
    shuffle: bool,
) -> Result<PlayerView, LibraryFault> {
    let playback = player.service(&window).map_err(|fault| match fault {
        PlayerFault::Opening => LibraryFault::Opening,
        _ => LibraryFault::Unavailable,
    })?;
    let plan = blocking(&host, &window, move |service| service.play_plan(&target)).await?;
    if plan.items.is_empty() {
        return Err(LibraryFault::NothingPlayable);
    }
    let items: Vec<Item> = plan
        .items
        .into_iter()
        .map(|item| {
            (
                item.path,
                Some(TrackRef {
                    track_id: item.track_id,
                    title: item.title,
                    artist: item.artist,
                    artwork_id: item.artwork_id,
                }),
            )
        })
        .collect();
    let (mode, start) = match mode {
        QueueMode::Replace => (PickMode::Listen, plan.start),
        QueueMode::Append => (PickMode::Append, 0),
        QueueMode::Next => (PickMode::Next, 0),
    };
    playback
        .request(Command::AddLibrary {
            items,
            mode,
            start,
            shuffle: shuffle.then(|| RandomState::new().hash_one("ECHO")),
        })
        .await
        .map_err(|error| match PlayerFault::from(error) {
            PlayerFault::Busy => LibraryFault::Busy,
            PlayerFault::FileUnavailable => LibraryFault::NothingPlayable,
            _ => LibraryFault::Unavailable,
        })
}

fn found<T>(value: Result<Option<T>, LibraryFault>) -> Result<T, LibraryFault> {
    value?.ok_or(LibraryFault::NotFound)
}

fn reading<T: Send + 'static>(
    service: &LibraryService,
    work: impl FnOnce(&rusqlite::Connection) -> Result<T, QueryError>,
) -> Result<T, LibraryFault> {
    service.read(work)
}

#[tauri::command]
pub(crate) async fn library_track(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    id: u32,
) -> Result<TrackRow, LibraryFault> {
    blocking(&host, &window, move |service| {
        found(reading(&service, |db| query::track(db, id)))
    })
    .await
}

#[tauri::command]
pub(crate) async fn library_album(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    id: u32,
) -> Result<AlbumRow, LibraryFault> {
    blocking(&host, &window, move |service| {
        found(reading(&service, |db| query::album(db, id)))
    })
    .await
}

#[tauri::command]
pub(crate) async fn library_artist(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    id: u32,
) -> Result<ArtistRow, LibraryFault> {
    blocking(&host, &window, move |service| {
        found(reading(&service, |db| query::artist(db, id)))
    })
    .await
}

#[tauri::command]
pub(crate) async fn library_home(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
) -> Result<HomeView, LibraryFault> {
    blocking(&host, &window, move |service| reading(&service, user::home)).await
}

#[tauri::command]
pub(crate) async fn library_palette(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    id: u32,
) -> Result<Palette, LibraryFault> {
    blocking(&host, &window, move |service| service.palette(id)).await
}

#[tauri::command]
pub(crate) async fn playlists_list(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
) -> Result<Vec<PlaylistCard>, LibraryFault> {
    blocking(&host, &window, move |service| {
        reading(&service, user::playlists)
    })
    .await
}

#[tauri::command]
pub(crate) async fn playlist_get(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    id: u32,
) -> Result<PlaylistCard, LibraryFault> {
    blocking(&host, &window, move |service| {
        found(reading(&service, |db| user::playlist(db, id)))
    })
    .await
}

#[tauri::command]
pub(crate) async fn playlist_entries(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    id: u32,
    offset: u32,
    limit: u32,
) -> Result<PlaylistPage, LibraryFault> {
    blocking(&host, &window, move |service| {
        reading(&service, |db| user::playlist_entries(db, id, offset, limit))
    })
    .await
}

#[tauri::command]
pub(crate) async fn playlist_act(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    action: PlaylistAct,
) -> Result<PlaylistOutcome, LibraryFault> {
    blocking(&host, &window, move |service| {
        service.mutate(|db, now| user::act(db, &action, now))
    })
    .await
}

#[tauri::command]
pub(crate) async fn favorite_tracks(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    offset: u32,
    limit: u32,
) -> Result<MediaPage, LibraryFault> {
    blocking(&host, &window, move |service| {
        reading(&service, |db| user::favorite_tracks(db, offset, limit))
    })
    .await
}

#[tauri::command]
pub(crate) async fn favorite_albums(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    offset: u32,
    limit: u32,
) -> Result<Vec<FavoriteAlbum>, LibraryFault> {
    blocking(&host, &window, move |service| {
        reading(&service, |db| user::favorite_albums(db, offset, limit))
    })
    .await
}

#[tauri::command]
pub(crate) async fn favorite_act(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    target: FavoriteTarget,
    on: bool,
) -> Result<(), LibraryFault> {
    blocking(&host, &window, move |service| {
        service.mutate(|db, now| user::set_favorite(db, target, on, now))
    })
    .await
}

#[tauri::command]
pub(crate) async fn history_page(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    offset: u32,
    limit: u32,
) -> Result<HistoryPage, LibraryFault> {
    blocking(&host, &window, move |service| {
        reading(&service, |db| user::history(db, offset, limit))
    })
    .await
}

#[tauri::command]
pub(crate) async fn history_clear(
    window: WebviewWindow,
    host: State<'_, Arc<LibraryHost>>,
    scope: HistoryScope,
) -> Result<u32, LibraryFault> {
    blocking(&host, &window, move |service| {
        service.mutate(|db, now| user::clear_history(db, scope, now))
    })
    .await
}
