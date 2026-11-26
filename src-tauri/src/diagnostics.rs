//! Explicit local diagnostics export. The schema contains no free-form logs,
//! station metadata, user paths, account identifiers or native error text.
use crate::{playback::desktop::DesktopPlayer, startup::Startup};
use serde::Serialize;
use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};
use windows::{
    Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW},
    core::{HSTRING, PCWSTR, w},
};

static EXPORTING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    application_version: &'static str,
    operating_system: &'static str,
    architecture: &'static str,
    library: crate::contract::LibraryState,
    schema_version: Option<i64>,
    native_playback_available: bool,
}

pub(crate) fn export(app: &AppHandle) {
    if EXPORTING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = app
            .path()
            .app_local_data_dir()
            .map_err(io::Error::other)
            .and_then(|directory| write_report(&directory, &app));
        let ui = app.clone();
        let queued = app.run_on_main_thread(move || {
            let (message, flags) = match result {
                Ok(path) => (
                    format!("Diagnostics saved to\n{}", path.display()),
                    MB_ICONINFORMATION,
                ),
                Err(_) => ("ECHO could not save its diagnostics.".into(), MB_ICONERROR),
            };
            if let Some(window) = ui.get_webview_window("main")
                && let Ok(hwnd) = window.hwnd()
            {
                let text = HSTRING::from(message);
                // SAFETY: this callback runs on the HWND's UI thread and
                // both message strings remain live throughout the call.
                unsafe {
                    MessageBoxW(
                        Some(hwnd),
                        PCWSTR(text.as_ptr()),
                        w!("ECHO diagnostics"),
                        MB_OK | flags,
                    )
                };
            }
            EXPORTING.store(false, Ordering::Release);
        });
        if queued.is_err() {
            EXPORTING.store(false, Ordering::Release);
        }
    });
}

fn write_report(directory: &Path, app: &AppHandle) -> io::Result<PathBuf> {
    let status = app.state::<Startup>().status();
    let schema_version = crate::library::open_reader(&directory.join("library.sqlite3"))
        .ok()
        .and_then(|db| {
            db.pragma_query_value(None, "user_version", |row| row.get(0))
                .ok()
        });
    let player = app.state::<Arc<DesktopPlayer>>();
    let report = Report {
        application_version: env!("CARGO_PKG_VERSION"),
        operating_system: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        library: status.library,
        schema_version,
        native_playback_available: app
            .get_webview_window("main")
            .is_some_and(|main| player.service(&main).is_ok()),
    };
    let json = serde_json::to_vec_pretty(&report).map_err(io::Error::other)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_millis();
    let path = directory.join(format!("diagnostics-{timestamp}.json"));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?
        .write_all(&json)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_schema_is_an_explicit_allowlist() {
        let value = serde_json::to_value(Report {
            application_version: "test",
            operating_system: "windows",
            architecture: "x86_64",
            library: crate::contract::LibraryState::Empty,
            schema_version: Some(5),
            native_playback_available: true,
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "applicationVersion": "test", "operatingSystem": "windows",
                "architecture": "x86_64", "library": "empty", "schemaVersion": 5,
                "nativePlaybackAvailable": true,
            })
        );
    }
}
