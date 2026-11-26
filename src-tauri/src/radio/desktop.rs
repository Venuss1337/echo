//! Scope directory and storage requests to the bundled main window.
use super::{artwork, directory::Directory, store};
use crate::contract::{
    PlayerView, RadioCollectionAct, RadioFault, RadioSort, Station, StationEdit, StationPage,
    StationQuery, StationShelf,
};
use crate::playback::{desktop::DesktopPlayer, service::Command};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{Manager, State, WebviewWindow, ipc::Response};
use tokio::{sync::Semaphore, task::AbortHandle};

// Only builds that load the reviewed callback-only decoder expose playback.
// Tests for the M5 gate and its exact profile are recorded in docs/RADIO.md.
const STATION_PLAYBACK: bool = cfg!(feature = "native-resources");

#[derive(Default)]
struct ActiveSearch {
    sequence: u64,
    active: Option<(u32, AbortHandle)>,
}

pub(crate) struct RadioHost {
    directory: Arc<Directory>,
    search: Mutex<ActiveSearch>,
    artwork: Semaphore,
    storage: Semaphore,
}

impl Default for RadioHost {
    fn default() -> Self {
        Self {
            directory: Arc::default(),
            search: Mutex::default(),
            artwork: Semaphore::new(2),
            storage: Semaphore::new(4),
        }
    }
}

impl RadioHost {
    pub(crate) fn cancel(&self) {
        if let Ok(mut search) = self.search.lock()
            && let Some((_, active)) = search.active.take()
        {
            active.abort();
        }
    }
}

fn db_path(window: &WebviewWindow) -> Result<PathBuf, RadioFault> {
    if window.label() != "main" {
        return Err(RadioFault::Unavailable);
    }
    let path = window
        .path()
        .app_local_data_dir()
        .map_err(|_| RadioFault::Unavailable)?
        .join("library.sqlite3");
    // The startup worker owns initial database creation and migration.
    if !path.is_file() {
        return Err(RadioFault::Unavailable);
    }
    Ok(path)
}

async fn blocking<T: Send + 'static>(
    host: &RadioHost,
    window: &WebviewWindow,
    work: impl FnOnce(&mut rusqlite::Connection) -> Result<T, RadioFault> + Send + 'static,
) -> Result<T, RadioFault> {
    let path = db_path(window)?;
    let _permit = host.storage.try_acquire().map_err(|_| RadioFault::Busy)?;
    tauri::async_runtime::spawn_blocking(move || work(&mut store::database(&path)?))
        .await
        .map_err(|_| RadioFault::Unavailable)?
}

#[tauri::command]
pub(crate) async fn radio_search(
    window: WebviewWindow,
    host: State<'_, RadioHost>,
    request: u32,
    query: StationQuery,
) -> Result<StationPage, RadioFault> {
    let path = db_path(&window)?;
    let directory = host.directory.clone();
    let (sequence, task) = {
        let mut state = host.search.lock().map_err(|_| RadioFault::Unavailable)?;
        if let Some((_, active)) = state.active.take() {
            active.abort();
        }
        let task = tokio::spawn(async move { directory.search(path, query).await });
        state.sequence += 1;
        state.active = Some((request, task.abort_handle()));
        let sequence = state.sequence;
        (sequence, task)
    };
    let result = task.await.map_err(|error| {
        if error.is_cancelled() {
            RadioFault::Cancelled
        } else {
            RadioFault::Unavailable
        }
    });
    let mut state = host.search.lock().map_err(|_| RadioFault::Unavailable)?;
    if state.sequence == sequence {
        state.active = None;
    }
    result?
}

#[tauri::command]
pub(crate) fn radio_cancel(window: WebviewWindow, host: State<'_, RadioHost>, request: u32) {
    if window.label() != "main" {
        return;
    }
    if let Ok(mut state) = host.search.lock()
        && state
            .active
            .as_ref()
            .is_some_and(|(token, _)| *token == request)
        && let Some((_, active)) = state.active.take()
    {
        active.abort();
    }
}

#[tauri::command]
pub(crate) async fn radio_collection(
    window: WebviewWindow,
    host: State<'_, RadioHost>,
    shelf: StationShelf,
    offset: u32,
) -> Result<StationPage, RadioFault> {
    blocking(&host, &window, move |db| {
        let (stations, next) = store::shelf(db, shelf, offset)?;
        Ok(StationPage {
            stations,
            next,
            stale: false,
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn radio_station(
    window: WebviewWindow,
    host: State<'_, RadioHost>,
    id: String,
) -> Result<Station, RadioFault> {
    blocking(&host, &window, move |db| {
        store::station(db, &id).map(|entry| entry.station)
    })
    .await
}

#[tauri::command]
pub(crate) async fn radio_edit(
    window: WebviewWindow,
    host: State<'_, RadioHost>,
    id: String,
) -> Result<StationEdit, RadioFault> {
    blocking(&host, &window, move |db| {
        let entry = store::station(db, &id)?;
        let station = entry.station;
        if !station.manual {
            return Err(RadioFault::NotFound);
        }
        Ok(StationEdit {
            id: Some(station.id),
            url: entry.stream,
            name: station.name,
            country: station.country,
            language: station.language,
            tags: station.tags,
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn radio_collection_act(
    window: WebviewWindow,
    host: State<'_, RadioHost>,
    action: RadioCollectionAct,
) -> Result<Option<Station>, RadioFault> {
    blocking(&host, &window, move |db| match action {
        RadioCollectionAct::Save { station } => store::save(db, station).map(Some),
        RadioCollectionAct::Remove { id } => store::remove(db, &id).map(|()| None),
        RadioCollectionAct::Favorite { id, enabled } => store::favorite(db, &id, enabled).map(Some),
    })
    .await
}

#[tauri::command]
pub(crate) async fn radio_preferences(
    window: WebviewWindow,
    host: State<'_, RadioHost>,
    sort: Option<RadioSort>,
) -> Result<RadioSort, RadioFault> {
    blocking(&host, &window, move |db| store::preference(db, sort)).await
}

#[tauri::command]
pub(crate) async fn radio_artwork(
    window: WebviewWindow,
    host: State<'_, RadioHost>,
    id: String,
) -> Result<Response, RadioFault> {
    let path = db_path(&window)?;
    let _permit = host.artwork.try_acquire().map_err(|_| RadioFault::Busy)?;
    artwork::image(path, id).await.map(Response::new)
}

#[tauri::command]
pub(crate) fn radio_playback_available(window: WebviewWindow) -> bool {
    window.label() == "main" && STATION_PLAYBACK
}

#[tauri::command]
pub(crate) async fn radio_play(
    window: WebviewWindow,
    host: State<'_, RadioHost>,
    player: State<'_, Arc<DesktopPlayer>>,
    id: String,
) -> Result<PlayerView, RadioFault> {
    if !radio_playback_available(window.clone()) {
        return Err(RadioFault::PlaybackDisabled);
    }
    let service = player
        .service(&window)
        .map_err(|_| RadioFault::Unavailable)?;
    let entry = blocking(&host, &window, move |db| store::station(db, &id)).await?;
    let click = (!entry.station.manual).then(|| entry.station.id.clone());
    let reply = service
        .request(Command::PlayStation {
            station: entry.station,
            source: entry.stream,
        })
        .await
        .map_err(|_| RadioFault::Busy)?;
    if let Some(id) = click {
        let directory = host.directory.clone();
        tauri::async_runtime::spawn(async move { directory.count_click(id).await });
    }
    Ok(reply)
}
