//! Desktop composition root for startup, storage and scoped local playback.
pub mod contract;
#[cfg(windows)]
mod diagnostics;
#[cfg(windows)]
mod geometry;
mod library;
#[cfg(windows)]
mod media_session;
#[cfg(windows)]
pub mod native;
pub mod playback;
#[cfg(windows)]
pub mod radio;
mod startup;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod window;

use contract::AppStatus;
#[cfg(windows)]
use library::desktop::{self as library_desktop, LibraryHost};
#[cfg(windows)]
use playback::desktop::{self, DesktopPlayer};
use startup::Startup;
use tauri::{Manager, RunEvent, State};

#[tauri::command]
fn app_get_status(startup: State<'_, Startup>) -> AppStatus {
    startup.status()
}

/// Runs the desktop event loop and releases startup resources on final exit.
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
        // Windows forwards launch arguments and the working directory. A
        // second launch only restores the first window in its current mode.
        tray::show(app);
    }));
    let builder = builder.setup(|app| {
        let directory = app.path().app_local_data_dir().ok();
        #[cfg(windows)]
        let (player, library_host, resource_dir, playback_data, system) = {
            let player = std::sync::Arc::new(DesktopPlayer::default());
            let library_host = std::sync::Arc::new(LibraryHost::default());
            app.manage(player.clone());
            app.manage(library_host.clone());
            app.manage(radio::desktop::RadioHost::default());
            let chrome = window::WindowChrome::default();
            app.manage(chrome.clone());
            let geometry = std::sync::Arc::new(geometry::Geometry::new(directory.as_deref()));
            app.manage(geometry.clone());
            if let Some(main) = app.get_webview_window("main") {
                geometry.restore(&main);
                chrome.attach(&main);
                geometry.remember(&main);
                let subject = main.clone();
                main.on_window_event(move |event| {
                    if matches!(
                        event,
                        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)
                    ) && !chrome.is_mini()
                    {
                        geometry.remember(&subject);
                    }
                });
            }
            let system = app
                .get_webview_window("main")
                .and_then(|main| {
                    media_session::start(
                        &main,
                        std::sync::Arc::downgrade(&player),
                        directory.clone(),
                    )
                })
                .map(|(sender, media)| {
                    app.manage(media);
                    sender
                });
            if tray::install(app.handle()).is_err() {
                eprintln!("ECHO tray unavailable");
            }
            (
                player,
                library_host,
                app.path().resource_dir().ok(),
                directory.clone(),
                system,
            )
        };
        app.manage(Startup::start(directory, move || {
            #[cfg(windows)]
            {
                // The library comes first so playback can report files that
                // turn out to be missing.
                let library = library_host.initialize(playback_data.as_deref());
                let mut hooks = library.map_or_else(Default::default, |library| {
                    let heard = library.clone();
                    playback::service::Hooks {
                        unavailable: Some(std::sync::Arc::new(move |track| {
                            library.report_unavailable(track)
                        })),
                        listened: Some(std::sync::Arc::new(move |event| {
                            heard.report_listened(event)
                        })),
                        system: None,
                    }
                });
                hooks.system = system;
                player.initialize(resource_dir.as_deref(), playback_data.as_deref(), hooks);
            }
        }));
        Ok(())
    });
    #[cfg(windows)]
    let builder = builder.invoke_handler(tauri::generate_handler![
        app_get_status,
        desktop::player_snapshot,
        desktop::player_subscribe,
        desktop::player_act,
        desktop::player_pick,
        library_desktop::library_summary,
        library_desktop::library_subscribe,
        library_desktop::library_add_root,
        library_desktop::library_remove_root,
        library_desktop::library_scan,
        library_desktop::library_cancel_scan,
        library_desktop::library_tracks,
        library_desktop::library_albums,
        library_desktop::library_artists,
        library_desktop::library_search,
        library_desktop::library_artwork,
        library_desktop::library_play,
        library_desktop::library_track,
        library_desktop::library_album,
        library_desktop::library_artist,
        library_desktop::library_home,
        library_desktop::library_palette,
        library_desktop::playlists_list,
        library_desktop::playlist_get,
        library_desktop::playlist_entries,
        library_desktop::playlist_act,
        library_desktop::favorite_tracks,
        library_desktop::favorite_albums,
        library_desktop::favorite_act,
        library_desktop::history_page,
        library_desktop::history_clear,
        radio::desktop::radio_search,
        radio::desktop::radio_cancel,
        radio::desktop::radio_collection,
        radio::desktop::radio_station,
        radio::desktop::radio_edit,
        radio::desktop::radio_collection_act,
        radio::desktop::radio_preferences,
        radio::desktop::radio_artwork,
        radio::desktop::radio_playback_available,
        radio::desktop::radio_play,
        window::window_subscribe,
        window::window_act,
        window::window_caption
    ]);
    #[cfg(not(windows))]
    let builder = builder.invoke_handler(tauri::generate_handler![app_get_status]);
    builder
        .build(tauri::generate_context!())
        .expect("build ECHO desktop runtime")
        .run(|app, event| {
            #[cfg(windows)]
            if let RunEvent::ExitRequested {
                code: None, api, ..
            } = &event
            {
                api.prevent_exit();
                begin_shutdown(app);
            }
            if matches!(event, RunEvent::Exit) {
                app.state::<Startup>().shutdown();
                #[cfg(windows)]
                {
                    app.state::<std::sync::Arc<DesktopPlayer>>().shutdown();
                    app.state::<std::sync::Arc<LibraryHost>>().shutdown();
                }
            }
        });
}

#[cfg(windows)]
fn begin_shutdown(app: &tauri::AppHandle) {
    let player = app.state::<std::sync::Arc<DesktopPlayer>>();
    if !player.begin_exit() {
        return;
    }
    app.remove_tray_by_id(tray::ID);
    app.state::<radio::desktop::RadioHost>().cancel();
    app.state::<std::sync::Arc<LibraryHost>>().begin_exit();
    // Preserve the existing bound for a decoder blocked inside a disk driver.
    let _ = std::thread::Builder::new()
        .name("echo-exit-watchdog".into())
        .spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(10));
            eprintln!("ECHO shutdown exceeded its bound; ending the process");
            std::process::exit(1);
        });
    let handle = app.clone();
    if std::thread::Builder::new()
        .name("echo-shutdown".into())
        .spawn(move || {
            handle.state::<Startup>().shutdown();
            handle.state::<std::sync::Arc<DesktopPlayer>>().shutdown();
            if let Some(media) = handle.try_state::<media_session::MediaSession>() {
                media.shutdown();
            }
            handle.state::<std::sync::Arc<LibraryHost>>().shutdown();
            if handle
                .state::<std::sync::Arc<geometry::Geometry>>()
                .save()
                .is_err()
            {
                eprintln!("ECHO could not save its window placement");
            }
            handle.exit(0);
        })
        .is_err()
    {
        app.exit(1);
    }
}
