//! Single-owner, test-only bindings to the pinned libmpv client header.
use echo_lib::native::NativeLibrary;
use echo_lib::radio::{RadioProtocol, StreamInfo};
use libloading::os::windows::{LOAD_LIBRARY_SEARCH_SYSTEM32, Library};
use std::{
    ffi::{CStr, c_char, c_int, c_ulong, c_void},
    path::Path,
    ptr::{self, NonNull},
    time::{Duration, Instant},
};

type Handle = *mut c_void;
type Init = unsafe extern "C" fn(Handle) -> c_int;
type Destroy = unsafe extern "C" fn(Handle);
type OptionString = unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int;
type Command = unsafe extern "C" fn(Handle, *const *const c_char) -> c_int;
type Get = unsafe extern "C" fn(Handle, *const c_char, c_int, *mut c_void) -> c_int;
type Wait = unsafe extern "C" fn(Handle, f64) -> *const RawEvent;
type AddStream = unsafe extern "C" fn(
    Handle,
    *const c_char,
    *mut c_void,
    unsafe extern "C" fn(*mut c_void, *mut c_char, *mut StreamInfo) -> c_int,
) -> c_int;

const FORMAT_FLAG: c_int = 3;
const FORMAT_DOUBLE: c_int = 5;
const FORMAT_STRING: c_int = 1;
const OPTION_NOT_FOUND: c_int = -5;

#[repr(C)]
struct RawEvent {
    id: c_int,
    error: c_int,
    userdata: u64,
    data: *const c_void,
}

#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
}

#[derive(Debug, PartialEq)]
pub enum Event {
    Loaded,
    Restarted,
    Seek,
    End { reason: i32, error: i32 },
    Other,
}

pub const EOF: i32 = 0;
pub const STOP: i32 = 2;
pub const ERROR: i32 = 4;

#[derive(Debug, PartialEq)]
pub enum LoadError {
    RelativePath,
    Dll,
    Symbol(&'static str),
    Abi(u32),
    Create,
    Initialize,
    Option(&'static str),
}

enum LibraryOwner {
    Probe(Library),
    Installed(NativeLibrary),
}

impl LibraryOwner {
    fn library(&self) -> &Library {
        match self {
            Self::Probe(library) => library,
            Self::Installed(installed) => installed.library(),
        }
    }
}

pub struct Mpv {
    handle: NonNull<c_void>,
    destroy: Destroy,
    command: Command,
    get: Get,
    wait: Wait,
    library: LibraryOwner,
    radio: Box<RadioProtocol>,
}

impl Mpv {
    /// Opens a trusted developer DLL with only System32 in dependency search.
    ///
    /// # Safety
    /// The absolute path must designate a reviewed binary in a directory the caller
    /// controls. Windows executes DLL initialization before version checks can run.
    pub unsafe fn open(path: &Path) -> Result<Self, LoadError> {
        if !path.is_absolute() {
            return Err(LoadError::RelativePath);
        }
        let path = path.canonicalize().map_err(|_| LoadError::Dll)?;
        // SAFETY: caller attests provenance. The candidate imports system DLLs only;
        // omitting DLL_LOAD_DIR also excludes arbitrary DLLs alongside the candidate.
        let library = unsafe { Library::load_with_flags(path, LOAD_LIBRARY_SEARCH_SYSTEM32) }
            .map_err(|_| LoadError::Dll)?;
        Self::initialize(LibraryOwner::Probe(library), c"null")
    }

    pub fn from_installed(library: NativeLibrary, audio_output: &CStr) -> Result<Self, LoadError> {
        Self::initialize(LibraryOwner::Installed(library), audio_output)
    }

    fn initialize(owner: LibraryOwner, audio_output: &CStr) -> Result<Self, LoadError> {
        let installed = matches!(owner, LibraryOwner::Installed(_));
        let library = owner.library();
        // SAFETY: symbols are copied with client.h signatures from the trusted DLL.
        // The library remains owned by Self until its only handle is terminated.
        unsafe {
            let version =
                symbol::<unsafe extern "C" fn() -> c_ulong>(library, "mpv_client_api_version")?;
            check_abi(version())?;
            let create = symbol::<unsafe extern "C" fn() -> Handle>(library, "mpv_create")?;
            let option = symbol::<OptionString>(library, "mpv_set_option_string")?;
            let init = symbol::<Init>(library, "mpv_initialize")?;
            let destroy = symbol::<Destroy>(library, "mpv_terminate_destroy")?;
            let command = symbol::<Command>(library, "mpv_command")?;
            let get = symbol::<Get>(library, "mpv_get_property")?;
            let wait = symbol::<Wait>(library, "mpv_wait_event")?;
            let handle = NonNull::new(create()).ok_or(LoadError::Create)?;
            let mpv = Self {
                handle,
                destroy,
                command,
                get,
                wait,
                library: owner,
                radio: Box::default(),
            };
            for name in [c"load-scripts", c"ytdl"] {
                let result = option(handle.as_ptr(), name.as_ptr(), c"no".as_ptr());
                // The source-built profile compiles scripting out; the full
                // developer DLL has those options and must accept disabling them.
                if (installed && result != OPTION_NOT_FOUND) || (!installed && result < 0) {
                    return Err(LoadError::Option(name.to_str().expect("ASCII option")));
                }
            }
            for (name, value) in [
                (c"config", c"no"),
                (c"ao", audio_output),
                (c"audio-fallback-to-null", c"no"),
                (c"vid", c"no"),
                (c"network-timeout", c"3"),
            ] {
                if option(handle.as_ptr(), name.as_ptr(), value.as_ptr()) < 0 {
                    return Err(LoadError::Option(name.to_str().expect("ASCII option")));
                }
            }
            if init(handle.as_ptr()) < 0 {
                return Err(LoadError::Initialize);
            }
            Ok(mpv)
        }
    }

    pub fn radio(&mut self) -> &RadioProtocol {
        assert_eq!(self.command(&[c"set", c"access-references", c"no"]), 0);
        // SAFETY: the boxed registration stays at a fixed address through
        // Drop, which cancels the source and terminates the owned core.
        unsafe {
            let add = symbol::<AddStream>(self.library.library(), "mpv_stream_cb_add_ro").unwrap();
            assert_eq!(self.radio.register(self.handle.as_ptr(), add), 0);
        }
        &self.radio
    }

    /// Tests are single-threaded per handle. The return value is command execution
    /// status only. Tests separately obtain playback events and property readings.
    pub fn command(&mut self, args: &[&CStr]) -> i32 {
        let args: Vec<_> = args
            .iter()
            .map(|arg| arg.as_ptr())
            .chain([ptr::null()])
            .collect();
        // SAFETY: arguments are NUL-terminated and live through this synchronous call.
        unsafe { (self.command)(self.handle.as_ptr(), args.as_ptr()) }
    }

    pub fn flag(&mut self, name: &CStr) -> Option<bool> {
        self.property::<c_int>(name, FORMAT_FLAG)
            .map(|flag| flag != 0)
    }

    pub fn number(&mut self, name: &CStr) -> Option<f64> {
        self.property(name, FORMAT_DOUBLE)
    }

    pub fn text(&mut self, name: &CStr) -> Option<String> {
        let value = self
            .property::<*mut c_char>(name, FORMAT_STRING)
            .and_then(NonNull::new)?;
        // SAFETY: mpv allocated and NUL-terminated the string; free via the same DLL.
        unsafe {
            let free =
                symbol::<unsafe extern "C" fn(*mut c_void)>(self.library.library(), "mpv_free")
                    .unwrap();
            let version = CStr::from_ptr(value.as_ptr())
                .to_string_lossy()
                .into_owned();
            free(value.as_ptr().cast());
            Some(version)
        }
    }

    fn property<T: Default>(&mut self, name: &CStr, format: c_int) -> Option<T> {
        let mut value = T::default();
        // SAFETY: callers pair each MPV_FORMAT with its exact C storage type.
        let result = unsafe {
            (self.get)(
                self.handle.as_ptr(),
                name.as_ptr(),
                format,
                (&mut value as *mut T).cast(),
            )
        };
        (result >= 0).then_some(value)
    }

    pub fn event(&mut self, timeout: Duration) -> Event {
        // SAFETY: copy the handle-owned event/payload while it is valid, before
        // another wait or destruction. Only END_FILE carries this payload layout.
        unsafe {
            let raw = (self.wait)(self.handle.as_ptr(), timeout.as_secs_f64())
                .as_ref()
                .expect("libmpv event");
            match raw.id {
                8 => Event::Loaded,
                20 => Event::Seek,
                21 => Event::Restarted,
                7 => {
                    let end = raw.data.cast::<EndFile>().as_ref().expect("end-file data");
                    Event::End {
                        reason: end.reason,
                        error: end.error,
                    }
                }
                _ => Event::Other,
            }
        }
    }

    pub fn until<T>(&mut self, mut check: impl FnMut(&mut Self, Event) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(6);
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            let event = self.event(remaining.min(Duration::from_millis(50)));
            if let Some(result) = check(self, event) {
                return result;
            }
        }
        panic!("timed out waiting for libmpv state");
    }

    pub fn expect(&mut self, expected: Event) {
        self.until(|_, event| {
            if event == expected {
                return Some(());
            }
            assert!(
                !matches!(event, Event::End { .. }),
                "unexpected {event:?}, wanted {expected:?}"
            );
            None
        });
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        self.radio.cancel();
        // SAFETY: this is the sole owning handle; the loaded module outlives Drop.
        // The worker watchdog bounds the documented blocking termination call.
        unsafe { (self.destroy)(self.handle.as_ptr()) };
    }
}

/// The API encodes the major version in the high word. All APIs used here
/// exist at 2.0, and mpv preserves ABI across minor revisions.
pub fn check_abi(version: u32) -> Result<(), LoadError> {
    if version >> 16 == 2 {
        Ok(())
    } else {
        Err(LoadError::Abi(version))
    }
}

unsafe fn symbol<T: Copy>(library: &Library, name: &'static str) -> Result<T, LoadError> {
    // SAFETY: caller established the symbol signature and keeps the library live.
    unsafe { library.get::<T>(name.as_bytes()) }
        .map(|symbol| *symbol)
        .map_err(|_| LoadError::Symbol(name))
}
