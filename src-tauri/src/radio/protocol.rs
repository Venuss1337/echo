//! Binding to mpv 0.42.0 include/mpv/stream_cb.h. The core owns each open cookie.
use super::RadioStream;
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    io::{self, Read},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr, slice,
    sync::Mutex,
};

type Open = unsafe extern "C" fn(*mut c_void, *mut c_char, *mut StreamInfo) -> c_int;

#[repr(C)]
pub struct StreamInfo {
    pub(crate) cookie: *mut c_void,
    pub(crate) read: unsafe extern "C" fn(*mut c_void, *mut c_char, u64) -> i64,
    pub(crate) seek: Option<unsafe extern "C" fn(*mut c_void, i64) -> i64>,
    pub(crate) size: Option<unsafe extern "C" fn(*mut c_void) -> i64>,
    pub(crate) close: unsafe extern "C" fn(*mut c_void),
    pub(crate) cancel: unsafe extern "C" fn(*mut c_void),
}

#[derive(Default)]
struct Pending {
    sequence: u64,
    stream: Option<(CString, RadioStream)>,
    cancel: Option<Box<dyn Fn() + Send + Sync>>,
}

/// Owns one registration and assigns a single-use URI to each source.
#[derive(Default)]
pub struct RadioProtocol {
    pending: Mutex<Pending>,
}

impl RadioProtocol {
    /// Registers the callback table on an owned libmpv core.
    ///
    /// # Safety
    /// The caller supplies the real export and a live core, and keeps this
    /// allocation at a stable address until mpv_terminate_destroy returns.
    pub unsafe fn register(
        &self,
        core: *mut c_void,
        add: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void, Open) -> c_int,
    ) -> c_int {
        // SAFETY: the caller owns core and guarantees the lifetime of user_data.
        unsafe {
            add(
                core,
                c"echo-radio".as_ptr(),
                ptr::from_ref(self).cast_mut().cast(),
                open,
            )
        }
    }

    /// Cancels the previous stream before publishing a new one-shot decoder URI.
    pub fn replace(&self, stream: RadioStream) -> io::Result<CString> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| io::Error::other("radio state unavailable"))?;
        let sequence = pending
            .sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("radio identifiers exhausted"))?;
        let uri = CString::new(format!("echo-radio://{sequence}")).map_err(io::Error::other)?;
        if let Some(cancel) = pending.cancel.take() {
            cancel();
        }
        pending.sequence = sequence;
        pending.cancel = Some(Box::new(stream.cancellation()));
        pending.stream = Some((uri.clone(), stream));
        Ok(uri)
    }

    /// Wakes a blocked native read. Call before replacing, stopping or destroying the core.
    pub fn cancel(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            if let Some(cancel) = pending.cancel.take() {
                cancel();
            }
            pending.stream = None;
        }
    }
}

struct Cookie {
    stream: Mutex<RadioStream>,
    cancel: Box<dyn Fn() + Send + Sync>,
}

unsafe extern "C" fn open(user: *mut c_void, uri: *mut c_char, info: *mut StreamInfo) -> c_int {
    const LOADING_FAILED: c_int = -13;
    if user.is_null() || uri.is_null() || info.is_null() {
        return LOADING_FAILED;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the registration owner outlives the core and mpv provides a
        // C string and a writable stream info struct for this invocation.
        let (protocol, uri) = unsafe { (&*user.cast::<RadioProtocol>(), CStr::from_ptr(uri)) };
        let Ok(mut pending) = protocol.pending.lock() else {
            return LOADING_FAILED;
        };
        if !pending
            .stream
            .as_ref()
            .is_some_and(|(id, _)| id.as_c_str() == uri)
        {
            return LOADING_FAILED;
        }
        let Some((_, stream)) = pending.stream.take() else {
            return LOADING_FAILED;
        };
        let cookie = Box::new(Cookie {
            cancel: Box::new(stream.cancellation()),
            stream: Mutex::new(stream),
        });
        // SAFETY: mpv receives ownership of the cookie and releases it via close.
        unsafe {
            info.write(StreamInfo {
                cookie: Box::into_raw(cookie).cast(),
                read,
                seek: None,
                size: None,
                close,
                cancel,
            });
        }
        0
    }))
    .unwrap_or(LOADING_FAILED)
}

unsafe extern "C" fn read(cookie: *mut c_void, buf: *mut c_char, count: u64) -> i64 {
    if cookie.is_null() || buf.is_null() || count == 0 {
        return -1;
    }
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: mpv owns the cookie until close and provides a writable buffer.
        // The short-read cap avoids constructing an oversized Rust slice.
        let stream = unsafe { &*cookie.cast::<Cookie>() };
        let bytes =
            unsafe { slice::from_raw_parts_mut(buf.cast::<u8>(), count.min(16384) as usize) };
        stream
            .stream
            .lock()
            .ok()
            .and_then(|mut stream| stream.read(bytes).ok())
            .map_or(-1, |n| n as i64)
    }))
    .unwrap_or(-1)
}

unsafe extern "C" fn cancel(cookie: *mut c_void) {
    if !cookie.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: mpv keeps the cookie live during callbacks. Cancellation
            // never locks the reader, which may be waiting for the network.
            unsafe {
                ((*cookie.cast::<Cookie>()).cancel)();
            }
        }));
    }
}

unsafe extern "C" fn close(cookie: *mut c_void) {
    if !cookie.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: mpv calls close exactly once, after other callbacks finish.
            drop(unsafe { Box::from_raw(cookie.cast::<Cookie>()) });
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::MaybeUninit;

    #[test]
    fn replacement_invalidates_old_cookie_and_disallows_reopening() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let protocol = RadioProtocol::default();
        let old = protocol
            .replace(RadioStream::spawn(runtime.handle(), std::future::pending()))
            .unwrap();
        let mut info = MaybeUninit::<StreamInfo>::uninit();
        let user = ptr::from_ref(&protocol).cast_mut().cast();
        // SAFETY: this test supplies a live registration, C strings, and an
        // output struct. It simulates mpv's single close after the last read.
        unsafe {
            assert_eq!(open(user, old.as_ptr().cast_mut(), info.as_mut_ptr()), 0);
            let info = info.assume_init();
            let new = protocol
                .replace(RadioStream::spawn(runtime.handle(), std::future::pending()))
                .unwrap();
            let mut stale = [0];
            assert_eq!((info.read)(info.cookie, stale.as_mut_ptr(), 1), -1);
            (info.close)(info.cookie);
            let mut reopened = MaybeUninit::<StreamInfo>::uninit();
            assert_eq!(
                open(user, old.as_ptr().cast_mut(), reopened.as_mut_ptr()),
                -13
            );
            protocol.cancel();
            assert_eq!(
                open(user, new.as_ptr().cast_mut(), reopened.as_mut_ptr()),
                -13
            );
        }
    }
}
