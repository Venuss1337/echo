//! Verified Windows resource loading. The playback owner will be introduced in M2.

use libloading::os::windows::{
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, Library,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::c_ulong,
    fs::{self, File, OpenOptions},
    io::Read,
    os::windows::fs::OpenOptionsExt,
    path::Path,
};

const PRIMARY: &str = "libmpv-2.dll";

/// Identifies a missing or invalid native-resource set without exposing local paths.
#[derive(Debug, PartialEq, Eq)]
pub enum NativeError {
    Location,
    Manifest,
    Missing(String),
    UnexpectedFile,
    Unreadable(String),
    Digest(String),
    AlreadyLoaded(String),
    Loader,
    MissingExport(&'static str),
    IncompatibleAbi(u32),
}

/// Keeps validated resource file handles alive until the DLL is unloaded.
pub struct NativeLibrary {
    library: Library,
    _files: Vec<File>,
}

// These exports are the minimum contract exercised by the native lifecycle test.
const EXPORTS: [&str; 8] = [
    "mpv_create",
    "mpv_initialize",
    "mpv_terminate_destroy",
    "mpv_command",
    "mpv_set_option_string",
    "mpv_get_property",
    "mpv_wait_event",
    "mpv_free",
];

impl NativeLibrary {
    /// Loads a reviewed manifest's resource set from the fixed `native` directory.
    ///
    /// # Safety
    /// The manifest must come from the audited build, and the root and its
    /// ancestors must be controlled by the installer or isolated test process.
    /// Call this before loading third-party modules concurrently.
    pub unsafe fn open(root: &Path, manifest: &str) -> Result<Self, NativeError> {
        let expected: BTreeMap<String, String> =
            serde_json::from_str(manifest).map_err(|_| NativeError::Manifest)?;
        if !expected.contains_key(PRIMARY)
            || expected.iter().any(|(name, hash)| {
                !name.ends_with(".dll")
                    || name.len() <= 4
                    || !name.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b)
                    })
                    || hash.len() != 64
                    || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            return Err(NativeError::Manifest);
        }
        if !root.is_absolute() {
            return Err(NativeError::Location);
        }
        let root = root.canonicalize().map_err(|_| NativeError::Location)?;
        let dir = root.join("native");
        let actual = dir
            .canonicalize()
            .map_err(|_| NativeError::Missing(PRIMARY.into()))?;
        if actual != dir {
            return Err(NativeError::Location);
        }
        for entry in fs::read_dir(&dir).map_err(|_| NativeError::Location)? {
            let entry = entry.map_err(|_| NativeError::Location)?;
            let file_name = entry.file_name();
            if !entry
                .file_type()
                .map_err(|_| NativeError::Location)?
                .is_file()
                || !file_name
                    .to_str()
                    .is_some_and(|name| expected.contains_key(name))
            {
                return Err(NativeError::UnexpectedFile);
            }
        }

        let mut files = Vec::with_capacity(expected.len());
        for (name, hash) in &expected {
            if Library::open_already_loaded(name.as_str()).is_ok() {
                return Err(NativeError::AlreadyLoaded(name.clone()));
            }
            let path = dir.join(name);
            let resolved = path
                .canonicalize()
                .map_err(|_| NativeError::Missing(name.clone()))?;
            if resolved != path {
                return Err(NativeError::Location);
            }
            let mut file = OpenOptions::new()
                .read(true)
                .share_mode(1) // FILE_SHARE_READ; deny writes and replacement while mapped.
                .open(path)
                .map_err(|_| NativeError::Unreadable(name.clone()))?;
            let mut hasher = Sha256::new();
            let mut buffer = [0; 32 * 1024];
            loop {
                let count = file
                    .read(&mut buffer)
                    .map_err(|_| NativeError::Unreadable(name.clone()))?;
                if count == 0 {
                    break;
                }
                hasher.update(&buffer[..count]);
            }
            let actual = hasher.finalize();
            if actual
                .iter()
                .zip(hash.as_bytes().as_chunks::<2>().0)
                .any(|(&byte, pair)| {
                    std::str::from_utf8(pair)
                        .ok()
                        .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                        != Some(byte)
                })
            {
                return Err(NativeError::Digest(name.clone()));
            }
            files.push(file);
        }

        // SAFETY: the caller establishes directory trust and the expected binary set.
        // We hold read-only Windows share locks and exclude CWD, PATH and app dirs.
        let library = unsafe {
            Library::load_with_flags(
                dir.join(PRIMARY),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        }
        .map_err(|_| NativeError::Loader)?;
        // SAFETY: the verified image exports the signature from mpv's client.h.
        let abi = unsafe {
            let version = library
                .get::<unsafe extern "C" fn() -> c_ulong>(b"mpv_client_api_version")
                .map_err(|_| NativeError::MissingExport("mpv_client_api_version"))?;
            version()
        };
        if abi >> 16 != 2 {
            return Err(NativeError::IncompatibleAbi(abi));
        }
        for symbol in EXPORTS {
            // SAFETY: this checks only the address, without dereferencing or calling it.
            unsafe { library.get::<*const ()>(symbol.as_bytes()) }
                .map_err(|_| NativeError::MissingExport(symbol))?;
        }
        Ok(Self {
            library,
            _files: files,
        })
    }

    /// Borrows the loaded library for the isolated playback probe or future owner.
    pub fn library(&self) -> &Library {
        &self.library
    }
}

#[cfg(feature = "native-resources")]
pub(crate) fn open_packaged(root: &Path) -> Result<NativeLibrary, NativeError> {
    // SAFETY: the manifest is frozen into the executable at packaging time, and
    // the resource directory comes from Tauri's installed-resource resolver.
    unsafe { NativeLibrary::open(root, include_str!(env!("ECHO_NATIVE_DLL_MANIFEST"))) }
}

#[cfg(feature = "native-resources")]
pub(crate) fn open_packaged_local(root: &Path) -> Result<NativeLibrary, NativeError> {
    let decode = serde_json::from_str::<BTreeMap<String, String>>;
    let selected = decode(include_str!(env!("ECHO_NATIVE_DLL_MANIFEST")))
        .map_err(|_| NativeError::Manifest)?;
    let reviewed = decode(include_str!(
        "../../native/windows-x64/evidence/local/dlls.json"
    ))
    .map_err(|_| NativeError::Manifest)?;
    if selected != reviewed {
        return Err(NativeError::Manifest);
    }
    open_packaged(root)
}

/// Smoke checks the embedded manifest from the installed Windows executable
/// without starting the desktop window or loading any media.
#[cfg(feature = "native-resources")]
pub fn verify_installed_executable() -> Result<(), NativeError> {
    let executable = std::env::current_exe().map_err(|_| NativeError::Location)?;
    let root = executable.parent().ok_or(NativeError::Location)?;
    open_packaged(root).map(drop)
}
