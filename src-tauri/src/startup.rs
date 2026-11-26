//! Owns startup work and joins it during final application exit.
use crate::{
    contract::{AppStatus, LibraryState},
    library,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    thread::{self, JoinHandle},
};

pub(super) struct Startup {
    library: Arc<OnceLock<LibraryState>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Startup {
    pub(super) fn start(
        directory: Option<PathBuf>,
        verify_native: impl FnOnce() + Send + 'static,
    ) -> Self {
        let library = Arc::new(OnceLock::new());
        let result = library.clone();
        let worker = thread::Builder::new()
            .name("echo-startup".into())
            .spawn(move || {
                let state = match directory.as_deref().map(library::initialize) {
                    Some(Ok(())) => LibraryState::Empty,
                    Some(Err(error)) => {
                        eprintln!("ECHO library initialization: {error:?}");
                        LibraryState::Unavailable
                    }
                    None => LibraryState::Unavailable,
                };
                let _ = result.set(state);
                verify_native();
            })
            .ok();
        if worker.is_none() {
            let _ = library.set(LibraryState::Unavailable);
        }
        Self {
            library,
            worker: Mutex::new(worker),
        }
    }

    pub(super) fn status(&self) -> AppStatus {
        AppStatus {
            library: self.library.get().copied().unwrap_or(LibraryState::Opening),
        }
    }

    pub(super) fn shutdown(&self) {
        // Release the mutex before joining. The worker owns its connection,
        // never calls the event loop, and closes SQLite before publishing status.
        let worker = self.worker.lock().ok().and_then(|mut slot| slot.take());
        if let Some(worker) = worker
            && worker.join().is_err()
        {
            let _ = self.library.set(LibraryState::Unavailable);
        }
    }
}

impl Drop for Startup {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_joins_startup_and_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        let startup = Startup::start(Some(directory.path().to_owned()), move || {
            sent.send(()).unwrap()
        });
        startup.shutdown();
        startup.shutdown();
        assert!(received.try_recv().is_ok());
        assert!(matches!(startup.status().library, LibraryState::Empty));
    }

    #[test]
    fn invalid_data_path_stays_unavailable() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let startup = Startup::start(Some(file.path().to_owned()), || {});
        startup.shutdown();
        assert!(matches!(
            startup.status().library,
            LibraryState::Unavailable
        ));
        assert!(file.path().is_file());
    }
}
