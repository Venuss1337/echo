//! Single-source libmpv owner for the callback-only Windows decoder.
use crate::{
    native::NativeLibrary,
    radio::{RadioProtocol, RadioStream, StreamInfo},
};
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    fs::File,
    io::{Read, Seek, SeekFrom},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::{self, NonNull},
    slice,
    sync::{Arc, Mutex},
    time::Duration,
};

type Handle = *mut c_void;
type Open = unsafe extern "C" fn(*mut c_void, *mut c_char, *mut StreamInfo) -> c_int;

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

#[repr(C)]
struct Property {
    name: *const c_char,
    format: c_int,
    data: *const c_void,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecoderError {
    MissingExport,
    Create,
    Option,
    Initialize,
    Profile,
    Command,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecoderEvent {
    None,
    Loaded,
    Restarted,
    Buffering(bool),
    End { reason: i32, error: i32 },
    Shutdown,
    Other,
}

#[derive(Clone, Copy)]
pub enum Output {
    Wasapi,
    Null,
}

pub(super) struct StartOptions {
    pub output: Output,
    pub volume: u8,
    pub muted: bool,
    pub resume_at: u64,
}

struct Api {
    create: unsafe extern "C" fn() -> Handle,
    init: unsafe extern "C" fn(Handle) -> c_int,
    destroy: unsafe extern "C" fn(Handle),
    option: unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    command: unsafe extern "C" fn(Handle, *const *const c_char) -> c_int,
    get: unsafe extern "C" fn(Handle, *const c_char, c_int, *mut c_void) -> c_int,
    observe: unsafe extern "C" fn(Handle, u64, *const c_char, c_int) -> c_int,
    wait: unsafe extern "C" fn(Handle, f64) -> *const RawEvent,
    free: unsafe extern "C" fn(*mut c_void),
    wake: unsafe extern "C" fn(Handle, Option<unsafe extern "C" fn(*mut c_void)>, *mut c_void),
    stream: unsafe extern "C" fn(Handle, *const c_char, *mut c_void, Open) -> c_int,
}

impl Api {
    fn resolve(library: &NativeLibrary) -> Result<Self, DecoderError> {
        macro_rules! export {
            ($symbol:literal) => {{
                // SAFETY: NativeLibrary verified the trusted DLL and ABI.
                // Each destination field fixes the type from client.h/stream_cb.h.
                *unsafe { library.library().get($symbol) }
                    .map_err(|_| DecoderError::MissingExport)?
            }};
        }
        Ok(Self {
            create: export!(b"mpv_create"),
            init: export!(b"mpv_initialize"),
            destroy: export!(b"mpv_terminate_destroy"),
            option: export!(b"mpv_set_option_string"),
            command: export!(b"mpv_command"),
            get: export!(b"mpv_get_property"),
            observe: export!(b"mpv_observe_property"),
            wait: export!(b"mpv_wait_event"),
            free: export!(b"mpv_free"),
            wake: export!(b"mpv_set_wakeup_callback"),
            stream: export!(b"mpv_stream_cb_add_ro"),
        })
    }
}

/// Owns exactly one native source and one immutable generation. Create and drop
/// on the decoder worker. Even delayed events from this core keep its generation.
pub struct Decoder {
    handle: NonNull<c_void>,
    api: Api,
    generation: u64,
    restarted: bool,
    source: Source,
    wakeup: Option<Box<Wakeup>>,
    _library: Arc<NativeLibrary>,
}

enum Source {
    Local(Box<Mutex<Option<File>>>),
    Live(Box<RadioProtocol>),
}

impl Decoder {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn has_restarted(&self) -> bool {
        self.restarted
    }

    /// Starts an already-authorized file without passing any path to the decoder.
    pub fn start(
        library: Arc<NativeLibrary>,
        file: File,
        generation: u64,
        output: Output,
    ) -> Result<Self, DecoderError> {
        Self::start_configured(
            library,
            file,
            generation,
            StartOptions {
                output,
                volume: 75,
                muted: false,
                resume_at: 0,
            },
        )
    }

    pub(super) fn start_configured(
        library: Arc<NativeLibrary>,
        file: File,
        generation: u64,
        options: StartOptions,
    ) -> Result<Self, DecoderError> {
        Self::start_source(
            library,
            Source::Local(Box::new(Mutex::new(Some(file)))),
            c"echo-local://source",
            generation,
            options,
        )
    }

    pub(super) fn start_live(
        library: Arc<NativeLibrary>,
        stream: RadioStream,
        generation: u64,
        options: StartOptions,
    ) -> Result<Self, DecoderError> {
        let protocol = Box::<RadioProtocol>::default();
        let uri = protocol
            .replace(stream)
            .map_err(|_| DecoderError::Initialize)?;
        Self::start_source(library, Source::Live(protocol), &uri, generation, options)
    }

    fn start_source(
        library: Arc<NativeLibrary>,
        source: Source,
        uri: &CStr,
        generation: u64,
        options: StartOptions,
    ) -> Result<Self, DecoderError> {
        let api = Api::resolve(&library)?;
        // SAFETY: the factory comes from the verified module and yields a new owner.
        let handle = NonNull::new(unsafe { (api.create)() }).ok_or(DecoderError::Create)?;
        let mut decoder = Self {
            handle,
            api,
            generation,
            restarted: false,
            source,
            wakeup: None,
            _library: library,
        };
        decoder.initialize(options.output, options.resume_at)?;
        decoder.volume(options.volume)?;
        decoder.mute(options.muted)?;
        decoder.command(&[c"loadfile", uri])?;
        Ok(decoder)
    }

    fn initialize(&mut self, output: Output, resume_at: u64) -> Result<(), DecoderError> {
        let handle = self.handle.as_ptr();
        let output = match output {
            Output::Wasapi => c"wasapi",
            Output::Null => c"null",
        };
        // SAFETY: this thread owns the handle. Arguments are live C strings;
        // source is boxed and remains live through synchronous core termination.
        unsafe {
            for (name, value) in [
                (c"config", c"no"),
                (c"ao", output),
                (c"audio-fallback-to-null", c"no"),
                (c"vid", c"no"),
                (c"sid", c"no"),
                (c"autoload-files", c"no"),
                (c"access-references", c"no"),
                (c"load-unsafe-playlists", c"no"),
                (c"audio-display", c"no"),
                (c"demuxer", c"+lavf"),
            ] {
                if (self.api.option)(handle, name.as_ptr(), value.as_ptr()) < 0 {
                    return Err(DecoderError::Option);
                }
            }
            if matches!(self.source, Source::Live(_)) {
                for (name, value) in [
                    (c"cache", c"yes"),
                    (c"demuxer-max-bytes", c"512KiB"),
                    (c"demuxer-max-back-bytes", c"0"),
                    (c"demuxer-readahead-secs", c"1"),
                    (c"cache-secs", c"1"),
                ] {
                    if (self.api.option)(handle, name.as_ptr(), value.as_ptr()) < 0 {
                        return Err(DecoderError::Option);
                    }
                }
            }
            if resume_at > 0 {
                let start = seconds(resume_at)?;
                if (self.api.option)(handle, c"start".as_ptr(), start.as_ptr()) < 0 {
                    return Err(DecoderError::Option);
                }
            }
            if (self.api.init)(handle) < 0 {
                return Err(DecoderError::Initialize);
            }
            if (self.api.observe)(handle, 1, c"paused-for-cache".as_ptr(), 3) < 0 {
                return Err(DecoderError::Initialize);
            }
            // The approved stream registry advertises no built-in protocols.
            // The application callback has no protocol-list entry of its own.
            let mut value: *mut c_char = ptr::null_mut();
            if (self.api.get)(
                handle,
                c"protocol-list".as_ptr(),
                1,
                ptr::from_mut(&mut value).cast(),
            ) < 0
                || value.is_null()
            {
                return Err(DecoderError::Profile);
            }
            let empty = CStr::from_ptr(value).to_bytes().is_empty();
            (self.api.free)(value.cast());
            if !empty {
                return Err(DecoderError::Profile);
            }
            let registered = match &self.source {
                Source::Local(source) => (self.api.stream)(
                    handle,
                    c"echo-local".as_ptr(),
                    ptr::from_ref(source.as_ref()).cast_mut().cast(),
                    open,
                ),
                Source::Live(protocol) => protocol.register(handle, self.api.stream),
            };
            if registered < 0 {
                return Err(DecoderError::Initialize);
            }
        }
        Ok(())
    }

    pub fn pause(&mut self, paused: bool) -> Result<(), DecoderError> {
        self.command(&[c"set", c"pause", if paused { c"yes" } else { c"no" }])
    }

    /// Registers a worker notification. The callback may only signal nonblocking work.
    pub fn notify(&mut self, notify: impl Fn() + Send + Sync + 'static) {
        // SAFETY: core ownership remains here. Unregister before replacing the
        // allocation. libmpv synchronizes callback replacement with invocation.
        unsafe { (self.api.wake)(self.handle.as_ptr(), None, ptr::null_mut()) };
        let wakeup = Box::new(Wakeup(Box::new(notify)));
        let data = ptr::from_ref(wakeup.as_ref()).cast_mut().cast();
        self.wakeup = Some(wakeup);
        // SAFETY: data points to the new stable allocation through destroy.
        unsafe { (self.api.wake)(self.handle.as_ptr(), Some(wake_worker), data) };
    }

    pub fn stop(&mut self) -> Result<(), DecoderError> {
        if let Source::Live(protocol) = &self.source {
            protocol.cancel();
        }
        self.command(&[c"stop"])
    }

    pub fn seek(&mut self, milliseconds: u64) -> Result<(), DecoderError> {
        let seconds = seconds(milliseconds)?;
        self.command(&[c"seek", &seconds, c"absolute+exact"])
    }

    pub fn volume(&mut self, volume: u8) -> Result<(), DecoderError> {
        if volume > 100 {
            return Err(DecoderError::Command);
        }
        let volume = CString::new(volume.to_string()).map_err(|_| DecoderError::Command)?;
        self.command(&[c"set", c"volume", &volume])
    }

    pub fn mute(&mut self, mute: bool) -> Result<(), DecoderError> {
        self.command(&[c"set", c"mute", if mute { c"yes" } else { c"no" }])
    }

    pub fn position(&self) -> Option<f64> {
        self.number(c"time-pos")
    }
    pub fn duration(&self) -> Option<f64> {
        self.number(c"duration")
    }
    pub fn mixer_volume(&self) -> Option<f64> {
        self.number(c"volume")
    }
    pub fn is_paused(&self) -> Option<bool> {
        self.flag(c"pause")
    }
    pub fn is_muted(&self) -> Option<bool> {
        self.flag(c"mute")
    }
    pub fn is_seekable(&self) -> Option<bool> {
        self.flag(c"seekable")
    }

    pub fn audio_ready(&self) -> bool {
        self.number(c"audio-out-params/samplerate")
            .is_some_and(|rate| rate > 0.0)
    }

    pub(super) fn supported_live_format(&self) -> bool {
        let mut value: *mut c_char = ptr::null_mut();
        // SAFETY: string properties return a libmpv allocation; inspect it
        // before freeing it through the same core API.
        unsafe {
            if (self.api.get)(
                self.handle.as_ptr(),
                c"file-format".as_ptr(),
                1,
                ptr::from_mut(&mut value).cast(),
            ) < 0
                || value.is_null()
            {
                return false;
            }
            let supported = matches!(CStr::from_ptr(value).to_bytes(), b"mp3" | b"aac");
            (self.api.free)(value.cast());
            supported
        }
    }

    fn number(&self, property: &CStr) -> Option<f64> {
        let mut value = 0.0_f64;
        // SAFETY: format 5 requires a writable double and the core stays live.
        let status = unsafe {
            (self.api.get)(
                self.handle.as_ptr(),
                property.as_ptr(),
                5,
                ptr::from_mut(&mut value).cast(),
            )
        };
        (status >= 0 && value.is_finite()).then_some(value)
    }

    fn flag(&self, property: &CStr) -> Option<bool> {
        let mut value = 0_i32;
        // SAFETY: format 3 requires a writable C int and the core stays live.
        let status = unsafe {
            (self.api.get)(
                self.handle.as_ptr(),
                property.as_ptr(),
                3,
                ptr::from_mut(&mut value).cast(),
            )
        };
        (status >= 0).then_some(value != 0)
    }

    fn command(&mut self, args: &[&CStr]) -> Result<(), DecoderError> {
        let values: Vec<_> = args
            .iter()
            .map(|arg| arg.as_ptr())
            .chain([ptr::null()])
            .collect();
        // SAFETY: the argv array and its live strings are terminated for the synchronous call.
        let status = unsafe { (self.api.command)(self.handle.as_ptr(), values.as_ptr()) };
        if status < 0 {
            Err(DecoderError::Command)
        } else {
            Ok(())
        }
    }

    pub fn event(&mut self, deadline: Duration) -> (u64, DecoderEvent) {
        // SAFETY: only this owner calls wait; event and payload are copied before
        // the next wait. END_FILE is the only event interpreted as EndFile.
        let event = unsafe {
            let raw = (self.api.wait)(self.handle.as_ptr(), deadline.as_secs_f64()).as_ref();
            match raw.map(|raw| raw.id) {
                Some(0) => DecoderEvent::None,
                Some(1) => DecoderEvent::Shutdown,
                Some(8) => DecoderEvent::Loaded,
                Some(21) => DecoderEvent::Restarted,
                Some(22) => raw
                    .filter(|raw| raw.userdata == 1)
                    .and_then(|raw| raw.data.cast::<Property>().as_ref())
                    .filter(|property| property.format == 3)
                    .and_then(|property| property.data.cast::<c_int>().as_ref())
                    .map_or(DecoderEvent::Other, |&flag| {
                        DecoderEvent::Buffering(flag != 0)
                    }),
                Some(7) => raw
                    .and_then(|raw| raw.data.cast::<EndFile>().as_ref())
                    .map_or(DecoderEvent::Other, |end| DecoderEvent::End {
                        reason: end.reason,
                        error: end.error,
                    }),
                _ => DecoderEvent::Other,
            }
        };
        self.restarted |= event == DecoderEvent::Restarted;
        (self.generation, event)
    }
}

fn seconds(milliseconds: u64) -> Result<CString, DecoderError> {
    CString::new(format!(
        "{}.{:03}",
        milliseconds / 1000,
        milliseconds % 1000,
    ))
    .map_err(|_| DecoderError::Command)
}

struct Wakeup(Box<dyn Fn() + Send + Sync>);

unsafe extern "C" fn wake_worker(data: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Decoder keeps the boxed callback alive while registered.
        unsafe { (&*data.cast::<Wakeup>()).0() };
    }));
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if let Source::Live(protocol) = &self.source {
            protocol.cancel();
        }
        // SAFETY: this is the unique owner. Destroy waits for callback work and
        // releases the native cookies before the registration and DLL can drop.
        unsafe {
            (self.api.destroy)(self.handle.as_ptr());
        }
    }
}

unsafe extern "C" fn open(user: *mut c_void, url: *mut c_char, info: *mut StreamInfo) -> c_int {
    if user.is_null() || url.is_null() || info.is_null() {
        return -13;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: libmpv supplies the URI and writable callback table. This
        // core's owner keeps user pointing to its boxed registration.
        let source = unsafe { &*user.cast::<Mutex<Option<File>>>() };
        if unsafe { CStr::from_ptr(url) } != c"echo-local://source" {
            return -13;
        }
        let Some(file) = source.lock().ok().and_then(|mut source| source.take()) else {
            return -13;
        };
        let cookie = Box::new(Mutex::new(file));
        // SAFETY: ownership of the allocated cookie moves to the native core.
        unsafe {
            info.write(StreamInfo {
                cookie: Box::into_raw(cookie).cast(),
                read,
                seek: Some(seek),
                size: Some(size),
                close,
                cancel: no_op,
            });
        }
        0
    }))
    .unwrap_or(-13)
}

unsafe extern "C" fn read(cookie: *mut c_void, buf: *mut c_char, count: u64) -> i64 {
    if cookie.is_null() || buf.is_null() {
        return -1;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the native core retains the cookie until close and supplies
        // writable memory. A bounded short read fits any Rust slice size.
        let file = unsafe { &*cookie.cast::<Mutex<File>>() };
        let bytes = unsafe { slice::from_raw_parts_mut(buf.cast(), count.min(65536) as usize) };
        file.lock()
            .ok()
            .and_then(|mut file| file.read(bytes).ok())
            .map_or(-1, |length| length as i64)
    }))
    .unwrap_or(-1)
}

unsafe extern "C" fn seek(cookie: *mut c_void, offset: i64) -> i64 {
    if cookie.is_null() || offset < 0 {
        return -1;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the cookie lives through the callback; the mutex serializes cursor access.
        unsafe { &*cookie.cast::<Mutex<File>>() }
            .lock()
            .ok()
            .and_then(|mut file| file.seek(SeekFrom::Start(offset as u64)).ok())
            .and_then(|position| i64::try_from(position).ok())
            .unwrap_or(-1)
    }))
    .unwrap_or(-1)
}

unsafe extern "C" fn size(cookie: *mut c_void) -> i64 {
    if cookie.is_null() {
        return -1;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the native core keeps the cookie live until callbacks finish.
        unsafe { &*cookie.cast::<Mutex<File>>() }
            .lock()
            .ok()
            .and_then(|file| file.metadata().ok())
            .and_then(|metadata| i64::try_from(metadata.len()).ok())
            .unwrap_or(-1)
    }))
    .unwrap_or(-1)
}

unsafe extern "C" fn close(cookie: *mut c_void) {
    // SAFETY: libmpv invokes close exactly once after callbacks finish.
    drop(unsafe { Box::from_raw(cookie.cast::<Mutex<File>>()) });
}

unsafe extern "C" fn no_op(_: *mut c_void) {}
