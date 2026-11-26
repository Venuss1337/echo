//! Best-effort restoration of normal window bounds. A corrupt or obsolete
//! record falls back to Tauri's default placement.
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{LogicalSize, PhysicalPosition, WebviewWindow};
use windows::{
    Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    core::PCWSTR,
};

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    x: i32,
    y: i32,
    width: f64,
    height: f64,
}

impl Placement {
    fn valid(self) -> bool {
        (420.0..=8192.0).contains(&self.width) && (540.0..=8192.0).contains(&self.height)
    }
}

pub(crate) struct Geometry {
    path: Option<PathBuf>,
    last: Mutex<Option<Placement>>,
}

impl Geometry {
    pub(crate) fn new(directory: Option<&Path>) -> Self {
        Self {
            path: directory.map(|path| path.join("window-placement.json")),
            last: Mutex::new(None),
        }
    }

    pub(crate) fn restore(&self, window: &WebviewWindow) {
        let Some(placement) = self.path.as_ref().and_then(|path| read(path)) else {
            return;
        };
        // Geometry belongs to the user's current physical layout. If a saved
        // display is disconnected, let Windows place the normal window.
        let Ok(monitors) = window.available_monitors() else {
            return;
        };
        let Some(monitor) = monitors.iter().find(|monitor| {
            let a = monitor.work_area();
            i64::from(placement.x) >= i64::from(a.position.x)
                && i64::from(placement.x) + 64 < i64::from(a.position.x) + i64::from(a.size.width)
                && i64::from(placement.y) >= i64::from(a.position.y)
                && i64::from(placement.y) + 64 < i64::from(a.position.y) + i64::from(a.size.height)
        }) else {
            return;
        };
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let width = placement
            .width
            .min(f64::from(area.size.width) / scale - 24.0);
        let height = placement
            .height
            .min(f64::from(area.size.height) / scale - 24.0);
        if width >= 420.0 && height >= 540.0 {
            let x = fit(placement.x, area.position.x, area.size.width, width, scale);
            let y = fit(
                placement.y,
                area.position.y,
                area.size.height,
                height,
                scale,
            );
            let _ = window.set_position(PhysicalPosition::new(x, y));
            let _ = window.set_size(LogicalSize::new(width, height));
        }
    }

    pub(crate) fn remember(&self, window: &WebviewWindow) {
        if !matches!(window.is_minimized(), Ok(false))
            || !matches!(window.is_maximized(), Ok(false))
        {
            return;
        }
        let (Ok(origin), Ok(size), Ok(scale)) = (
            window.outer_position(),
            window.inner_size(),
            window.scale_factor(),
        ) else {
            return;
        };
        let logical = size.to_logical::<f64>(scale);
        let placement = Placement {
            x: origin.x,
            y: origin.y,
            width: logical.width,
            height: logical.height,
        };
        if placement.valid()
            && let Ok(mut last) = self.last.lock()
        {
            *last = Some(placement);
        }
    }

    pub(crate) fn save(&self) -> std::io::Result<()> {
        let (Some(path), Ok(last)) = (&self.path, self.last.lock()) else {
            return Ok(());
        };
        let Some(placement) = *last else {
            return Ok(());
        };
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        let result = (|| {
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            output.write_all(&serde_json::to_vec(&placement).map_err(std::io::Error::other)?)?;
            output.sync_all()?;
            drop(output);
            use std::os::windows::ffi::OsStrExt;
            let wide = |path: &Path| {
                path.as_os_str()
                    .encode_wide()
                    .chain([0])
                    .collect::<Vec<_>>()
            };
            let from = wide(&temp);
            let to = wide(path);
            // SAFETY: both UTF-16 buffers are NUL-terminated app-owned paths
            // and remain live until Windows finishes the same-volume rename.
            unsafe {
                MoveFileExW(
                    PCWSTR(from.as_ptr()),
                    PCWSTR(to.as_ptr()),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            }
            .map_err(std::io::Error::other)
        })();
        let _ = std::fs::remove_file(temp);
        result
    }
}

fn fit(saved: i32, start: i32, extent: u32, logical: f64, scale: f64) -> i32 {
    let frame_and_client = ((logical + 24.0) * scale).ceil() as u32;
    let last = i64::from(start) + i64::from(extent.saturating_sub(frame_and_client));
    i64::from(saved).clamp(i64::from(start), last.min(i64::from(i32::MAX))) as i32
}

fn read(path: &Path) -> Option<Placement> {
    let mut data = Vec::new();
    File::open(path)
        .ok()?
        .take(513)
        .read_to_end(&mut data)
        .ok()?;
    if data.len() > 512 {
        return None;
    }
    let placement: Placement = serde_json::from_slice(&data).ok()?;
    placement.valid().then_some(placement)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_or_invalid_geometry_and_atomically_replaces_it() {
        let temp = tempfile::tempdir().unwrap();
        let store = Geometry::new(Some(temp.path()));
        let path = store.path.as_ref().unwrap();
        std::fs::write(path, [b' '; 513]).unwrap();
        assert!(read(path).is_none());
        let mut placement = Placement {
            x: -50,
            y: 60,
            width: 419.0,
            height: 700.0,
        };
        assert!(!placement.valid());
        placement.width = 800.0;
        *store.last.lock().unwrap() = Some(placement);
        store.save().unwrap();
        assert_eq!(read(path).unwrap().x, -50);
        assert_eq!(temp.path().read_dir().unwrap().count(), 1);
    }
}
