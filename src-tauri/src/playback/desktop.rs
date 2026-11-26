//! Main-window command boundary. The renderer never supplies filesystem paths.
use super::{
    decoder::Output,
    queue::{EntryId, Repeat},
    service::{Command, PickMode, Service},
    source::open_selected,
};
use crate::contract::{PickAction, PlayerFault, PlayerIntent, PlayerView, RepeatMode};
use std::{
    collections::hash_map::RandomState,
    ffi::OsString,
    hash::BuildHasher,
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{State, WebviewWindow, ipc::Channel};
use windows::{
    Win32::{
        Foundation::HWND,
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
        UI::Shell::{
            Common::COMDLG_FILTERSPEC, FOS_ALLOWMULTISELECT, FOS_DONTADDTORECENT,
            FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_NODEREFERENCELINKS,
            FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH,
        },
    },
    core::{HRESULT, w},
};

/// One native modal at a time across file and folder pickers.
static PICKER_BUSY: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
pub(crate) struct DesktopPlayer {
    ready: AtomicBool,
    exiting: AtomicBool,
    service: Mutex<Option<Arc<Service>>>,
}

impl DesktopPlayer {
    pub(crate) fn system_action(&self, action: super::service::SystemAction) {
        if !self.exiting.load(Ordering::Acquire)
            && let Ok(service) = self.service.lock()
            && let Some(service) = service.as_ref()
        {
            service.system_action(action);
        }
    }
    pub(crate) fn begin_exit(&self) -> bool {
        !self.exiting.swap(true, Ordering::AcqRel)
    }

    /// `hooks` report unopenable files and confirmed listening to the library.
    pub(crate) fn initialize(
        &self,
        root: Option<&Path>,
        data: Option<&Path>,
        hooks: super::service::Hooks,
    ) {
        #[cfg(feature = "native-resources")]
        if let Some((root, data)) = root.zip(data)
            && let Ok(library) = crate::native::open_packaged_local(root)
            && let Ok(service) = Service::start_with_hooks(
                Arc::new(library),
                data.join("library.sqlite3"),
                Output::Wasapi,
                hooks,
            )
            && let Ok(mut slot) = self.service.lock()
        {
            *slot = Some(Arc::new(service));
        }
        #[cfg(not(feature = "native-resources"))]
        let _ = (root, data, hooks, Output::Wasapi);
        self.ready.store(true, Ordering::Release);
    }

    pub(crate) fn shutdown(&self) {
        let service = self.service.lock().ok().and_then(|mut slot| slot.take());
        if let Some(service) = service
            && service.shutdown().is_err()
        {
            eprintln!("ECHO playback did not checkpoint cleanly");
        }
    }

    pub(crate) fn service(&self, window: &WebviewWindow) -> Result<Arc<Service>, PlayerFault> {
        if window.label() != "main" || self.exiting.load(Ordering::Acquire) {
            return Err(PlayerFault::Unavailable);
        }
        self.service
            .lock()
            .map_err(|_| PlayerFault::Unavailable)?
            .clone()
            .ok_or_else(|| {
                if self.ready.load(Ordering::Acquire) {
                    PlayerFault::Unavailable
                } else {
                    PlayerFault::Opening
                }
            })
    }
}

#[tauri::command]
pub(crate) async fn player_snapshot(
    window: WebviewWindow,
    player: State<'_, Arc<DesktopPlayer>>,
) -> Result<PlayerView, PlayerFault> {
    Ok(player.service(&window)?.request(Command::Snapshot).await?)
}

#[tauri::command]
pub(crate) async fn player_subscribe(
    window: WebviewWindow,
    player: State<'_, Arc<DesktopPlayer>>,
    channel: Channel<PlayerView>,
) -> Result<PlayerView, PlayerFault> {
    Ok(player
        .service(&window)?
        .request(Command::Subscribe(channel))
        .await?)
}

#[tauri::command]
pub(crate) async fn player_act(
    window: WebviewWindow,
    player: State<'_, Arc<DesktopPlayer>>,
    intent: PlayerIntent,
) -> Result<PlayerView, PlayerFault> {
    let id = |value: &str| EntryId::parse(value).ok_or(PlayerFault::Invalid);
    let command = match intent {
        PlayerIntent::Play { id: value } => Command::Play(id(&value)?),
        PlayerIntent::Pause => Command::Pause(true),
        PlayerIntent::Resume => Command::Pause(false),
        PlayerIntent::Stop => Command::Stop,
        PlayerIntent::Next => Command::Next,
        PlayerIntent::Previous => Command::Previous,
        PlayerIntent::Seek {
            generation,
            milliseconds,
        } if milliseconds.is_finite()
            && milliseconds.fract() == 0.0
            && (0.0..=9_007_199_254_740_991.0).contains(&milliseconds) =>
        {
            Command::Seek {
                generation: id(&generation)?.value(),
                milliseconds: milliseconds as u64,
            }
        }
        PlayerIntent::Seek { .. } => return Err(PlayerFault::Invalid),
        PlayerIntent::Volume { value } => Command::Volume(value),
        PlayerIntent::Mute { enabled } => Command::Mute(enabled),
        PlayerIntent::Repeat { mode } => Command::Repeat(match mode {
            RepeatMode::Off => Repeat::Off,
            RepeatMode::One => Repeat::One,
            RepeatMode::All => Repeat::All,
        }),
        PlayerIntent::Shuffle { enabled } => Command::Shuffle(enabled.then(|| {
            // RandomState seeds playback order from the platform. Tests supply
            // their own deterministic seed directly to the queue.
            RandomState::new().hash_one("ECHO")
        })),
        PlayerIntent::Remove { id: value } => Command::Remove(id(&value)?),
        PlayerIntent::MoveBefore { id: value, before } => {
            Command::MoveBefore(id(&value)?, before.as_deref().map(id).transpose()?)
        }
        PlayerIntent::ClearUpcoming => Command::ClearUpcoming,
    };
    Ok(player.service(&window)?.request(command).await?)
}

/// Shows the native picker on Tauri's UI thread and returns the chosen paths.
/// The renderer never sees or supplies these paths.
pub(crate) async fn pick_on_ui_thread(
    window: &WebviewWindow,
    folders: bool,
) -> Result<Vec<PathBuf>, PlayerFault> {
    let owner = window.hwnd().map_err(|_| PlayerFault::PickerUnavailable)?.0 as usize;
    if PICKER_BUSY.swap(true, Ordering::AcqRel) {
        return Err(PlayerFault::Busy);
    }
    struct Unlock;
    impl Drop for Unlock {
        fn drop(&mut self) {
            PICKER_BUSY.store(false, Ordering::Release);
        }
    }
    let unlock = Unlock;
    let (send, receive) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let _unlock = unlock;
            let _ = send.send(pick(owner, folders));
        })
        .map_err(|_| PlayerFault::PickerUnavailable)?;
    receive.await.map_err(|_| PlayerFault::PickerUnavailable)?
}

#[tauri::command]
pub(crate) async fn player_pick(
    window: WebviewWindow,
    player: State<'_, Arc<DesktopPlayer>>,
    action: PickAction,
) -> Result<PlayerView, PlayerFault> {
    let service = player.service(&window)?;
    let paths = pick_on_ui_thread(&window, false).await?;
    if paths.is_empty() {
        return Ok(service.request(Command::Snapshot).await?);
    }
    let paths = tauri::async_runtime::spawn_blocking(move || -> Result<_, PlayerFault> {
        for path in &paths {
            open_selected(path).map_err(|_| PlayerFault::FileUnavailable)?;
        }
        Ok(paths)
    })
    .await
    .map_err(|_| PlayerFault::PickerUnavailable)??;
    let mode = match action {
        PickAction::Listen => PickMode::Listen,
        PickAction::Append => PickMode::Append,
        PickAction::Next => PickMode::Next,
    };
    Ok(service.request(Command::AddPicked(paths, mode)).await?)
}

fn pick(owner: usize, folders: bool) -> Result<Vec<PathBuf>, PlayerFault> {
    let failure = |_| PlayerFault::PickerUnavailable;
    // SAFETY: Tauri's Windows UI thread owns the live STA and window. Keep
    // every Shell interface there. A short-lived apartment can unload the
    // Shell module while its modal dialog is still finishing asynchronous work.
    // The system modal loop pumps UI messages while showing the picker.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(failure)?;
        let options = dialog.GetOptions().map_err(failure)?;
        let common = FOS_FORCEFILESYSTEM
            | FOS_ALLOWMULTISELECT
            | FOS_PATHMUSTEXIST
            | FOS_NOCHANGEDIR
            | FOS_NODEREFERENCELINKS
            | FOS_DONTADDTORECENT;
        if folders {
            dialog
                .SetOptions(options | common | FOS_PICKFOLDERS)
                .map_err(failure)?;
        } else {
            dialog
                .SetOptions(options | common | FOS_FILEMUSTEXIST)
                .map_err(failure)?;
            dialog
                .SetFileTypes(&[COMDLG_FILTERSPEC {
                    pszName: w!("Audio files"),
                    pszSpec: w!("*.mp3;*.flac;*.wav;*.ogg;*.oga;*.m4a;*.aac"),
                }])
                .map_err(failure)?;
        }
        match dialog.Show(Some(HWND(owner as *mut _))) {
            Err(error) if error.code() == HRESULT::from_win32(1223) => return Ok(Vec::new()),
            result => result.map_err(failure)?,
        }
        let results = dialog.GetResults().map_err(failure)?;
        let count = results.GetCount().map_err(failure)?;
        if count > 1024 {
            return Err(PlayerFault::Invalid);
        }
        (0..count)
            .map(|index| {
                let item = results.GetItemAt(index).map_err(failure)?;
                let wide = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(failure)?;
                if wide.is_null() {
                    return Err(PlayerFault::PickerUnavailable);
                }
                // The Shell returns a NUL-terminated COM allocation. Preserve
                // Windows path code units and release with the matching allocator.
                let path = PathBuf::from(OsString::from_wide(wide.as_wide()));
                CoTaskMemFree(Some(wide.0.cast()));
                Ok(path)
            })
            .collect()
    }
}
