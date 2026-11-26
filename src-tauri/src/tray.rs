//! Explicit background playback and native controls for the existing window.
use crate::playback::{desktop::DesktopPlayer, service::SystemAction};
use std::sync::Arc;
use tauri::{
    AppHandle, Manager,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

pub(crate) const ID: &str = "echo-tray";

pub(crate) fn install(app: &AppHandle) -> tauri::Result<()> {
    let mut menu = MenuBuilder::new(app);
    for (id, label) in [
        ("echo-show", "Show ECHO"),
        ("echo-hide", "Hide to tray"),
        ("echo-mini", "Always-on-top mini player"),
        ("echo-play", "Play"),
        ("echo-pause", "Pause"),
        ("echo-back", "Previous track"),
        ("echo-next", "Next track"),
        ("echo-stop", "Stop"),
        ("echo-diagnostics", "Export diagnostics"),
        ("echo-exit", "Exit ECHO"),
    ] {
        menu = menu.item(&MenuItemBuilder::with_id(id, label).build(app)?);
    }
    let Some(icon) = app.default_window_icon() else {
        return Ok(());
    };
    TrayIconBuilder::with_id(ID)
        .icon(icon.clone())
        .tooltip("ECHO")
        .menu(&menu.build()?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let action = match event.id().as_ref() {
                "echo-show" => {
                    show(app);
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = app
                            .state::<crate::window::WindowChrome>()
                            .leave_mini(&window);
                    }
                    return;
                }
                "echo-mini" => {
                    show(app);
                    if let Some(window) = app.get_webview_window("main")
                        && app
                            .state::<crate::window::WindowChrome>()
                            .enter_mini(&window)
                            .is_err()
                    {
                        eprintln!("ECHO mini player unavailable on this display");
                    }
                    return;
                }
                "echo-hide" => {
                    if let Some(window) = app.get_webview_window("main")
                        && window.minimize().is_ok()
                    {
                        let _ = window.hide();
                    }
                    return;
                }
                "echo-exit" => return crate::begin_shutdown(app),
                "echo-diagnostics" => {
                    show(app);
                    return crate::diagnostics::export(app);
                }
                "echo-play" => SystemAction::Play,
                "echo-pause" => SystemAction::Pause,
                "echo-back" => SystemAction::Previous,
                "echo-next" => SystemAction::Next,
                "echo-stop" => SystemAction::Stop,
                _ => return,
            };
            app.state::<Arc<DesktopPlayer>>().system_action(action);
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub(crate) fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        if let Ok(hwnd) = window.hwnd() {
            use windows::Win32::UI::WindowsAndMessaging::{IsWindowVisible, SW_SHOW, ShowWindow};
            // SAFETY: the HWND belongs to this app's live main window. A
            // shell visibility change can leave the runtime's cache unchanged.
            unsafe {
                if !IsWindowVisible(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                }
            }
        }
        let _ = window.set_focus();
        if let Some(chrome) = app.try_state::<crate::window::WindowChrome>()
            && chrome.restore_mini_z_order(&window).is_err()
        {
            eprintln!("ECHO could not restore the mini-player z-order");
        }
    }
}
