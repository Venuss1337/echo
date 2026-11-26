//! Opens approved local bytes once and hands the decoder a read-only handle.
use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf, Prefix},
};

const SHARE_READ: u32 = 1;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const BACKUP_SEMANTICS: u32 = 0x0200_0000;
const REPARSE_POINT: u32 = 0x400;
const OFFLINE: u32 = 0x1000;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetVolumeNameForVolumeMountPointW(root: *const u16, volume: *mut u16, size: u32) -> i32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceError {
    Path,
    Volume,
    Unavailable,
    ReparseOrOffline,
    NotAFile,
}

fn drive_and_parts(path: &Path) -> Result<(u8, Vec<&OsStr>), SourceError> {
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err(SourceError::Path);
    };
    let Prefix::Disk(drive) = prefix.kind() else {
        return Err(SourceError::Path);
    };
    if components.next() != Some(Component::RootDir) {
        return Err(SourceError::Path);
    }
    let names = components
        .map(|part| {
            let Component::Normal(name) = part else {
                return Err(SourceError::Path);
            };
            if name
                .encode_wide()
                .any(|unit| unit == 0 || unit == b':' as u16)
            {
                return Err(SourceError::Path);
            }
            Ok(name)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((drive, names))
}

fn volume_root(drive: u8) -> Result<PathBuf, SourceError> {
    let root = [drive.into(), b':' as u16, b'\\' as u16, 0];
    let mut volume = [0_u16; 64];
    // SAFETY: both buffers live for the calls, the input is terminated and
    // the output capacity is passed in UTF-16 units.
    unsafe {
        if !matches!(GetDriveTypeW(root.as_ptr()), 2 | 3 | 5 | 6)
            || GetVolumeNameForVolumeMountPointW(
                root.as_ptr(),
                volume.as_mut_ptr(),
                volume.len() as u32,
            ) == 0
            || !matches!(GetDriveTypeW(volume.as_ptr()), 2 | 3 | 5 | 6)
        {
            return Err(SourceError::Volume);
        }
    }
    let end = volume
        .iter()
        .position(|unit| *unit == 0)
        .ok_or(SourceError::Volume)?;
    Ok(PathBuf::from(OsString::from_wide(&volume[..end])))
}

fn checked_open(path: &Path, directory: bool) -> Result<File, SourceError> {
    let flags = OPEN_REPARSE_POINT | if directory { BACKUP_SEMANTICS } else { 0 };
    let file = OpenOptions::new()
        .read(true)
        .share_mode(SHARE_READ)
        .custom_flags(flags)
        .open(path)
        .map_err(|_| SourceError::Unavailable)?;
    let metadata = file.metadata().map_err(|_| SourceError::Unavailable)?;
    if metadata.file_attributes() & (REPARSE_POINT | OFFLINE) != 0 {
        return Err(SourceError::ReparseOrOffline);
    }
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err(SourceError::NotAFile);
    }
    Ok(file)
}

/// Opens a path already authorized by the application. This is not an IPC API.
/// Rejects relative, UNC, device, ADS, network-volume and reparse paths.
pub fn open_selected(path: &Path) -> Result<File, SourceError> {
    let (drive, names) = drive_and_parts(path)?;
    // A bare drive root names a directory, never a file.
    let (last, ancestors) = names.split_last().ok_or(SourceError::Path)?;
    let mut path = volume_root(drive)?;
    // Use a volume GUID to avoid driving later opens through a remapped drive
    // letter. Keep each checked directory open with deny-write/delete sharing
    // until the last component has been opened with no reparse processing.
    let mut anchors = Vec::with_capacity(ancestors.len());
    for name in ancestors {
        path.push(name);
        anchors.push(checked_open(&path, true)?);
    }
    path.push(last);
    checked_open(&path, false)
}

/// Validates a folder the user chose as a library root and returns its
/// normalized path. The same rules as [`open_selected`] apply: a local drive,
/// no UNC or device names, and no reparse point (junction, symlink, cloud
/// placeholder) anywhere on the path. A drive root such as `D:\` is allowed.
pub fn open_directory(path: &Path) -> Result<PathBuf, SourceError> {
    let (drive, names) = drive_and_parts(path)?;
    let mut checked = volume_root(drive)?;
    // The volume root itself needs no check: it is the mount point of a
    // local fixed or removable volume that GetVolumeNameForVolumeMountPointW resolved.
    let mut anchors = Vec::with_capacity(names.len());
    for name in &names {
        checked.push(name);
        anchors.push(checked_open(&checked, true)?);
    }
    let mut normalized = PathBuf::from(format!("{}:\\", (drive as char).to_ascii_uppercase()));
    for name in names {
        normalized.push(name);
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_real_directories_but_not_files_junctions_or_unc() {
        let dir = tempfile::tempdir().unwrap();
        let music = dir.path().join("Müsik 音");
        std::fs::create_dir(&music).unwrap();
        let normalized = open_directory(&music).unwrap();
        assert!(normalized.is_dir());
        assert!(open_directory(Path::new(r"C:\")).is_ok());
        let file = music.join("a.wav");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(open_directory(&file).err(), Some(SourceError::NotAFile));
        assert_eq!(
            open_directory(&music.join("missing")).err(),
            Some(SourceError::Unavailable)
        );
        assert_eq!(
            open_directory(Path::new(r"\server\share")).err(),
            Some(SourceError::Path)
        );
        let junction = dir.path().join("link");
        let made = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/j"])
            .arg(&junction)
            .arg(&music)
            .output()
            .unwrap();
        assert!(made.status.success());
        let result = open_directory(&junction);
        std::fs::remove_dir(&junction).unwrap();
        assert_eq!(result.err(), Some(SourceError::ReparseOrOffline));
    }

    #[test]
    fn rejects_nonlocal_names_before_open() {
        for path in [
            r"\\127.0.0.1\share\song.wav",
            r"\\?\UNC\host\share\a.wav",
            r"\\.\COM1",
            r"\\?\GLOBALROOT\Device\a",
            r"C:\dir\song.wav:hidden",
            r"C:\dir\..\song.wav",
            r"C:song.wav",
            r"dir\song.wav",
        ] {
            assert!(
                matches!(open_selected(Path::new(path)), Err(SourceError::Path)),
                "{path}"
            );
        }
    }

    #[test]
    fn opens_unicode_and_space_from_the_fixed_volume() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known bytes 音.wav");
        std::fs::write(&path, b"audio fixture").unwrap();
        let mut file = open_selected(&path).unwrap();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut bytes).unwrap();
        assert_eq!(bytes, b"audio fixture");
    }

    #[test]
    fn refuses_an_actual_directory_junction() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real");
        let junction = dir.path().join("junction");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("track.wav"), b"audio fixture").unwrap();
        let status = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/j"])
            .arg(&junction)
            .arg(&target)
            .output()
            .unwrap();
        assert!(status.status.success(), "could not create test junction");
        let result = open_selected(&junction.join("track.wav"));
        // Remove the link itself before the temporary directory is dropped.
        std::fs::remove_dir(&junction).unwrap();
        assert!(matches!(result, Err(SourceError::ReparseOrOffline)));
    }
}
