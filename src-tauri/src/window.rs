//! The integrated title bar's native half.
//!
//! Tauri removes the caption but keeps the native frame, so resizing, shadows,
//! Aero Snap, the taskbar and Win+arrow behave as usual. WebView2 turns
//! `app-region: drag` into real caption hits, which brings dragging,
//! double-click maximize and the system menu without any IPC permission.
//!
//! What neither can do is tell Windows where the maximize button is: the page
//! covers the whole client area, so the frame never sees that hit test and
//! Windows 11 never offers Snap Layouts. A small child window that only
//! answers `HTMAXBUTTON` sits over the page's button, reports hover and press
//! back to it, and paints nothing. This is the arrangement Windows Terminal
//! uses for its own drawn caption buttons.

use crate::contract::{CaptionBounds, WindowFault, WindowIntent, WindowView};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tauri::{State, WebviewWindow, WindowEvent, ipc::Channel};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowPlacement, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOSENDCHANGING,
    SetWindowPlacement, WINDOWPLACEMENT, WS_EX_TOPMOST,
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::ScreenToClient,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::WM_MOUSELEAVE,
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent},
            WindowsAndMessaging::{
                CREATESTRUCTW, CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GetClientRect,
                GetParent, GetWindowLongPtrW, HTMAXBUTTON, HTTRANSPARENT, HWND_TOP, IDC_ARROW,
                IsIconic, IsZoomed, LoadCursorW, PostMessageW, RegisterClassW, SC_MAXIMIZE,
                SC_RESTORE, SM_CXPADDEDBORDER, SM_CYFRAME, SWP_HIDEWINDOW, SWP_NOACTIVATE,
                SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SetWindowLongPtrW,
                SetWindowPos, WINDOW_EX_STYLE, WM_CREATE, WM_ERASEBKGND, WM_NCDESTROY,
                WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
                WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_SYSCOMMAND, WNDCLASSW, WS_CHILD,
                WS_CLIPSIBLINGS,
            },
        },
    },
    core::w,
};

/// What the caption knows, and where it draws its maximize button.
#[derive(Default)]
struct Caption {
    view: WindowView,
    bounds: Option<CaptionBounds>,
    /// The hit-test window, as an address because handles are not `Send`.
    target: isize,
    channel: Option<Channel<WindowView>>,
    restore: Option<WINDOWPLACEMENT>,
    /// What WebView2 was last told about the page's visibility.
    presented: Option<bool>,
}

/// Owns the hit-test window's state. All native calls run on the UI thread.
#[derive(Clone, Default)]
pub(crate) struct WindowChrome(Arc<Mutex<Caption>>);

impl WindowChrome {
    pub(crate) fn is_mini(&self) -> bool {
        self.lock().restore.is_some()
    }
    fn lock(&self) -> MutexGuard<'_, Caption> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Creates the hit-test window and follows the main window's size.
    /// Must run on the UI thread, which owns both windows.
    pub(crate) fn attach(&self, window: &WebviewWindow) {
        let Ok(parent) = window.hwnd() else { return };
        // Without the target the HTML button still works; only the flyout is lost.
        if let Some(target) = create_target(parent, self.clone()) {
            self.lock().target = target.0 as isize;
        }
        let chrome = self.clone();
        let address = parent.0 as isize;
        let webview = window.clone();
        window.on_window_event(move |event| match event {
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                chrome.place(HWND(address as *mut _));
                chrome.present(&webview);
            }
            _ => {}
        });
        self.place(parent);
        self.present(window);
    }

    /// Tells WebView2 whether anyone can see the page.
    ///
    /// Minimizing, including Hide to tray (which minimizes before hiding),
    /// leaves playback, the tray and the system media controls to Rust, so
    /// nothing needs the page drawn. Without this, WebView2 keeps the hidden
    /// page composited: its GPU process holds tiles and textures for a window
    /// nobody sees. `IsVisible = false` stops rendering and lets Chromium drop
    /// that graphics memory, and the low memory target asks every WebView2
    /// process to trim caches, as Microsoft recommends for inactive apps. The
    /// page itself keeps running, so player events and IPC are unaffected,
    /// and `document.hidden` already pauses the renderer's own interpolation.
    /// Restoring reverses both before the window is shown again.
    fn present(&self, window: &WebviewWindow) {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL, ICoreWebView2_19,
        };
        use windows::core::Interface;
        let Ok(hwnd) = window.hwnd() else { return };
        // SAFETY: a read-only query of this application's live main window.
        let visible = !unsafe { IsIconic(hwnd).as_bool() };
        {
            let mut caption = self.lock();
            if caption.presented == Some(visible) {
                return;
            }
            caption.presented = Some(visible);
        }
        let _ = window.with_webview(move |webview| {
            let controller = webview.controller();
            // SAFETY: with_webview runs this on the UI thread that owns the
            // controller, which stays alive for the duration of the call.
            unsafe {
                if controller.SetIsVisible(visible).is_err() {
                    eprintln!("ECHO could not change the page's visibility");
                }
                // Older WebView2 runtimes lack interface 19; visibility alone
                // still stops rendering there.
                if let Ok(core) = controller
                    .CoreWebView2()
                    .and_then(|core| core.cast::<ICoreWebView2_19>())
                {
                    let _ = core.SetMemoryUsageTargetLevel(if visible {
                        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
                    } else {
                        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
                    });
                }
            }
        });
    }

    /// Moves the target over the drawn button and publishes the window state.
    fn place(&self, parent: HWND) {
        let (target, bounds) = {
            let caption = self.lock();
            (
                HWND(caption.target as *mut _),
                if caption.restore.is_none() {
                    caption.bounds
                } else {
                    None
                },
            )
        };
        // SAFETY: both handles belong to this thread and outlive the call, or
        // are already invalid, which these calls report without effect.
        let (maximized, minimized) =
            unsafe { (IsZoomed(parent).as_bool(), IsIconic(parent).as_bool()) };
        if !target.is_invalid() {
            unsafe {
                let mut client = RECT::default();
                let scale = GetDpiForWindow(parent).max(96);
                let result = match bounds {
                    Some(bounds) if GetClientRect(parent, &mut client).is_ok() => {
                        let place = bounds.physical(client.right - client.left, scale);
                        SetWindowPos(
                            target,
                            Some(HWND_TOP),
                            place.left,
                            place.top,
                            place.right - place.left,
                            place.bottom - place.top,
                            SWP_NOACTIVATE | SWP_SHOWWINDOW,
                        )
                    }
                    _ => SetWindowPos(
                        target,
                        None,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_HIDEWINDOW,
                    ),
                };
                if result.is_err() {
                    eprintln!("ECHO could not place its maximize hit target");
                }
            }
        }
        self.update(|view| {
            view.minimized = minimized;
            // Minimizing a maximized window leaves it maximized underneath.
            if !minimized {
                view.maximized = maximized;
            }
        });
    }

    /// Presents the existing renderer at compact size and remembers the exact
    /// Windows placement (including restore bounds for a maximized window).
    pub(crate) fn enter_mini(&self, window: &WebviewWindow) -> Result<(), WindowFault> {
        if self.lock().restore.is_some() {
            return Ok(());
        }
        let hwnd = window.hwnd().map_err(|_| WindowFault::Unavailable)?;
        let monitor = window.current_monitor().ok().flatten().ok_or_else(|| {
            eprintln!("ECHO mini player: no current monitor");
            WindowFault::Unavailable
        })?;
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let height = (f64::from(area.size.height) / scale - 24.0).min(640.0);
        if f64::from(area.size.width) / scale < 444.0 || height < 540.0 {
            eprintln!("ECHO mini player: monitor work area is too small");
            return Err(WindowFault::Unavailable);
        }
        let size = tauri::LogicalSize::new(420.0, height).to_physical::<u32>(scale);
        let mut placement = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        // SAFETY: the window is live and the size field matches its ABI.
        unsafe { GetWindowPlacement(hwnd, &mut placement) }
            .map_err(|_| WindowFault::Unavailable)?;
        self.lock().restore = Some(placement);
        let result = (|| -> Result<(), WindowFault> {
            if unsafe { IsZoomed(hwnd).as_bool() } {
                window.unmaximize().map_err(|_| WindowFault::Unavailable)?;
            }
            window
                .set_size(size)
                .map_err(|_| WindowFault::Unavailable)?;
            let outer = window.outer_size().map_err(|_| WindowFault::Unavailable)?;
            let x = area.position.x + area.size.width.saturating_sub(outer.width + 16) as i32;
            let y = area.position.y + area.size.height.saturating_sub(outer.height + 16) as i32;
            // Keep the window runtime's style cache in sync so later frame
            // updates, such as restoring from minimize, preserve topmost.
            window
                .set_always_on_top(true)
                .map_err(|_| WindowFault::Unavailable)?;
            // Bypass the window's pre-change negotiation for this explicit
            // transition. WM_WINDOWPOSCHANGED still reports the applied move.
            // SAFETY: hwnd is the live main window owned by this application.
            unsafe {
                SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOSENDCHANGING,
                )
            }
            .map_err(|_| WindowFault::Unavailable)?;
            self.restore_mini_z_order(window)
        })();
        if result.is_err() {
            let _ = self.leave_mini(window);
            return Err(WindowFault::Unavailable);
        }
        self.update(|view| view.mini = Some(true));
        self.place(hwnd);
        Ok(())
    }

    /// Restoring the visible mini player may require an explicit activation
    /// before Windows honors the requested z-order on a non-foreground HWND.
    pub(crate) fn restore_mini_z_order(&self, window: &WebviewWindow) -> Result<(), WindowFault> {
        if !self.is_mini() {
            return Ok(());
        }
        let hwnd = window.hwnd().map_err(|_| WindowFault::Unavailable)?;
        // SAFETY: this reads or updates the live main window on its UI thread.
        unsafe {
            let topmost = || GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST.0 as isize != 0;
            if !topmost() {
                SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOSENDCHANGING,
                )
                .map_err(|_| WindowFault::Unavailable)?;
                if !topmost() {
                    eprintln!("ECHO mini player: Windows did not apply topmost z-order");
                    return Err(WindowFault::Unavailable);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn leave_mini(&self, window: &WebviewWindow) -> Result<(), WindowFault> {
        let Some(placement) = self.lock().restore.take() else {
            return Ok(());
        };
        let hwnd = window.hwnd().map_err(|_| WindowFault::Unavailable)?;
        let runtime = window.set_always_on_top(false);
        // SAFETY: placement was captured from the same live top-level window.
        let restored = unsafe { SetWindowPlacement(hwnd, &placement) };
        let top = unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOSENDCHANGING,
            )
        };
        self.update(|view| view.mini = None);
        self.place(hwnd);
        runtime.map_err(|_| WindowFault::Unavailable)?;
        top.map_err(|_| WindowFault::Unavailable)?;
        restored.map_err(|_| WindowFault::Unavailable)
    }

    /// Publishes only real changes: moving over the button is not one.
    fn update(&self, change: impl FnOnce(&mut WindowView)) {
        let changed = {
            let mut caption = self.lock();
            let before = caption.view;
            change(&mut caption.view);
            caption.view != before && caption.channel.is_some()
        };
        if changed {
            // The window procedure calls this, so never re-enter the runtime
            // from inside a message that Windows is still delivering. Tasks may
            // run in any order; each sends the state as it is, under the lock,
            // so the last one delivered is never older than the truth.
            let chrome = self.clone();
            tauri::async_runtime::spawn(async move {
                let caption = chrome.lock();
                if let Some(channel) = &caption.channel {
                    let _ = channel.send(caption.view);
                }
            });
        }
    }
}

impl CaptionBounds {
    /// A caption button near the top-right corner, and nothing else: the page
    /// cannot use this to cover other parts of the window.
    fn plausible(&self) -> bool {
        let within = |value: f64, low: f64, high: f64| (low..=high).contains(&value);
        within(self.right, 0.0, 240.0)
            && within(self.top, 0.0, 32.0)
            && within(self.width, 16.0, 96.0)
            && within(self.height, 16.0, 64.0)
    }

    /// CSS pixels from the top-right corner to physical client pixels.
    fn physical(&self, client_width: i32, dpi: u32) -> RECT {
        let scaled = |value: f64| (value * f64::from(dpi) / 96.0).round() as i32;
        // Edges are scaled, not sizes, so neighbouring buttons never overlap.
        RECT {
            left: client_width - scaled(self.right + self.width),
            top: scaled(self.top),
            right: client_width - scaled(self.right),
            bottom: scaled(self.top + self.height),
        }
    }
}

fn create_target(parent: HWND, chrome: WindowChrome) -> Option<HWND> {
    let state = Box::into_raw(Box::new(chrome));
    // SAFETY: the class is registered once per process with a static procedure.
    // The box is owned by the window from WM_CREATE until WM_NCDESTROY. A failed
    // creation leaks it, because Windows does not say whether either one ran.
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(target_proc),
            hInstance: module.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("ECHO_CAPTION_TARGET"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let created = CreateWindowExW(
            // Neither layered nor transparent: both lose the hit test.
            WINDOW_EX_STYLE::default(),
            w!("ECHO_CAPTION_TARGET"),
            w!("Maximize or restore"),
            WS_CHILD | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            Some(parent),
            None,
            Some(module.into()),
            Some(state.cast::<std::ffi::c_void>().cast_const()),
        );
        created.ok()
    }
}

/// The resize strip above the button still belongs to the frame.
unsafe fn in_top_resize_strip(target: HWND, position: LPARAM) -> bool {
    unsafe {
        let Ok(parent) = GetParent(target) else {
            return false;
        };
        if IsZoomed(parent).as_bool() {
            return false;
        }
        let mut point = POINT {
            x: i32::from(position.0 as i16),
            y: i32::from((position.0 >> 16) as i16),
        };
        let dpi = GetDpiForWindow(parent).max(96);
        let strip = GetSystemMetricsForDpi(SM_CYFRAME, dpi)
            + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
        // Half the frame: enough to grab, without eating the button.
        ScreenToClient(parent, &mut point).as_bool() && point.y < strip / 2
    }
}

unsafe extern "system" fn target_proc(
    target: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: GWLP_USERDATA holds the box given to CreateWindowExW between
    // WM_CREATE and WM_NCDESTROY. Every message arrives on the owning thread.
    unsafe {
        if message == WM_CREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(target, GWLP_USERDATA, create.lpCreateParams as isize);
            return LRESULT(0);
        }
        let state = GetWindowLongPtrW(target, GWLP_USERDATA) as *mut WindowChrome;
        if state.is_null() {
            return DefWindowProcW(target, message, wparam, lparam);
        }
        let chrome = &*state;
        match message {
            WM_NCHITTEST => {
                return LRESULT(if in_top_resize_strip(target, lparam) {
                    HTTRANSPARENT as isize
                } else {
                    HTMAXBUTTON as isize
                });
            }
            WM_NCMOUSEMOVE => {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: target,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut track);
                chrome.update(|view| view.maximize_hovered = true);
            }
            WM_NCMOUSELEAVE | WM_MOUSELEAVE => chrome.update(|view| {
                view.maximize_hovered = false;
                view.maximize_pressed = false;
            }),
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK => {
                // Not forwarded: the default procedure draws a classic button.
                chrome.update(|view| view.maximize_pressed = true);
                return LRESULT(0);
            }
            WM_NCLBUTTONUP => {
                let pressed = chrome.lock().view.maximize_pressed;
                chrome.update(|view| view.maximize_pressed = false);
                if pressed && let Ok(parent) = GetParent(target) {
                    let command = if IsZoomed(parent).as_bool() {
                        SC_RESTORE
                    } else {
                        SC_MAXIMIZE
                    };
                    let _ = PostMessageW(
                        Some(parent),
                        WM_SYSCOMMAND,
                        WPARAM(command as usize),
                        LPARAM(0),
                    );
                }
                return LRESULT(0);
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_NCDESTROY => {
                SetWindowLongPtrW(target, GWLP_USERDATA, 0);
                chrome.lock().target = 0;
                drop(Box::from_raw(state));
            }
            _ => {}
        }
        DefWindowProcW(target, message, wparam, lparam)
    }
}

fn main_only(window: &WebviewWindow) -> Result<(), WindowFault> {
    (window.label() == "main")
        .then_some(())
        .ok_or(WindowFault::Unavailable)
}

/// Streams maximize state and the native button's hover to the title bar.
#[tauri::command]
pub(crate) fn window_subscribe(
    window: WebviewWindow,
    chrome: State<'_, WindowChrome>,
    channel: Channel<WindowView>,
) -> Result<WindowView, WindowFault> {
    main_only(&window)?;
    let mut caption = chrome.lock();
    caption.channel = Some(channel);
    Ok(caption.view)
}

/// Fixed caption actions. Closing takes the ordinary close path, so the
/// existing checkpoint and bounded shutdown still run.
#[tauri::command]
pub(crate) fn window_act(
    window: WebviewWindow,
    chrome: State<'_, WindowChrome>,
    intent: WindowIntent,
) -> Result<(), WindowFault> {
    main_only(&window)?;
    match intent {
        WindowIntent::Minimize => window.minimize(),
        WindowIntent::EnterMini => return chrome.enter_mini(&window),
        WindowIntent::ReturnFromMini => return chrome.leave_mini(&window),
        WindowIntent::ToggleMaximize if chrome.lock().restore.is_some() => {
            return chrome.leave_mini(&window);
        }
        WindowIntent::ToggleMaximize => match window.is_maximized() {
            Ok(true) => window.unmaximize(),
            Ok(false) => window.maximize(),
            Err(error) => Err(error),
        },
        WindowIntent::Close => window.close(),
    }
    .map_err(|_| WindowFault::Unavailable)
}

/// Where the page draws its maximize button, or nothing while it is hidden.
#[tauri::command]
pub(crate) fn window_caption(
    window: WebviewWindow,
    chrome: State<'_, WindowChrome>,
    bounds: Option<CaptionBounds>,
) -> Result<(), WindowFault> {
    main_only(&window)?;
    if bounds.is_some_and(|bounds| !bounds.plausible()) {
        return Err(WindowFault::Invalid);
    }
    chrome.lock().bounds = bounds;
    // Synchronous commands run on the UI thread, which owns the target.
    chrome.place(window.hwnd().map_err(|_| WindowFault::Unavailable)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUTTON: CaptionBounds = CaptionBounds {
        right: 46.0,
        top: 0.0,
        width: 46.0,
        height: 34.0,
    };

    #[test]
    fn hit_target_follows_the_right_edge_at_every_scale() {
        let at = |width, dpi| {
            let r = BUTTON.physical(width, dpi);
            (r.left, r.top, r.right, r.bottom)
        };
        assert_eq!(at(1240, 96), (1148, 0, 1194, 34));
        assert_eq!(at(1550, 120), (1435, 0, 1492, 43));
        assert_eq!(at(1860, 144), (1722, 0, 1791, 51));
        assert_eq!(at(2480, 192), (2296, 0, 2388, 68));
    }

    #[test]
    fn the_page_cannot_place_the_target_outside_the_caption() {
        assert!(BUTTON.plausible());
        for bounds in [
            CaptionBounds {
                width: 900.0,
                ..BUTTON
            },
            CaptionBounds {
                height: 400.0,
                ..BUTTON
            },
            CaptionBounds {
                top: 300.0,
                ..BUTTON
            },
            CaptionBounds {
                right: 4000.0,
                ..BUTTON
            },
            CaptionBounds {
                right: -1.0,
                ..BUTTON
            },
            CaptionBounds {
                width: f64::NAN,
                ..BUTTON
            },
            CaptionBounds {
                top: f64::INFINITY,
                ..BUTTON
            },
        ] {
            assert!(!bounds.plausible(), "{bounds:?}");
        }
    }

    #[test]
    fn only_real_changes_count() {
        let chrome = WindowChrome::default();
        chrome.update(|view| view.maximize_hovered = true);
        assert!(chrome.lock().view.maximize_hovered);
        chrome.update(|view| {
            view.maximize_hovered = false;
            view.maximize_pressed = false;
        });
        assert_eq!(chrome.lock().view, WindowView::default());
    }
}
