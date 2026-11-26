//! Library coordinator: roots, scan scheduling, watching and progress.
//!
//! One coordinator thread owns every mutation that is not a scan batch: adding
//! and removing roots, queueing and cancelling scans, root health checks and
//! watcher lifecycle. Scans run one at a time on their own thread, which keeps
//! disk and CPU use bounded and makes cancellation unambiguous. Queries do not
//! pass through the coordinator: callers read through a small pool of
//! read-only SQLite connections, so a long scan never blocks the UI's pages.

use super::{
    artwork, index, meta, palette,
    query::{self, PlayPlan, QueryError},
    scan::{self, ScanEnv, ScanOutcome},
    user::{self, UserError},
    watch::RootWatcher,
};
use crate::{
    contract::{
        AlbumPage, AlbumQuery, ArtistPage, ArtistQuery, ArtworkSize, LibraryFault, LibrarySummary,
        Palette, PlayTarget, ScanPhase, ScanView, SearchResults, TrackPage, TrackQuery,
    },
    library,
    playback::{
        history::ListenEvent,
        source::{self, SourceError},
    },
};
use rusqlite::Connection;
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError, Sender, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Intervals are injectable so tests exercise the real scheduler quickly.
#[derive(Clone, Copy)]
pub(crate) struct Timing {
    /// Quiet period after the last watcher hint before rescanning.
    pub debounce: Duration,
    /// Minimum gap between a root's scans started by hints.
    pub min_interval: Duration,
    pub health: Duration,
    /// Safety-net rescan of every online root.
    pub reconcile: Duration,
    pub tick: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            debounce: Duration::from_secs(3),
            min_interval: Duration::from_secs(10),
            health: Duration::from_secs(5),
            reconcile: Duration::from_secs(15 * 60),
            tick: Duration::from_millis(500),
        }
    }
}

/// Receives each summary. Returns false once the subscriber is gone.
pub(crate) type Publisher = Box<dyn Fn(&LibrarySummary) -> bool + Send + Sync>;
type Reply = SyncSender<Result<(), LibraryFault>>;

enum Cmd {
    Add(PathBuf, Reply),
    Remove(i64, Reply),
    Scan(Option<i64>, Reply),
    Cancel(Reply),
}

enum Msg {
    Cmd(Cmd),
    Hint {
        root_id: i64,
        failed: bool,
        pending: Arc<AtomicBool>,
    },
    Unavailable(u32),
    Listened(ListenEvent),
    ScanFinished {
        root_id: i64,
        result: rusqlite::Result<ScanOutcome>,
    },
    Wake,
}

struct Inner {
    database: PathBuf,
    artwork_dir: PathBuf,
    sender: Sender<Msg>,
    readers: Mutex<Vec<Connection>>,
    scan: Mutex<Option<ScanView>>,
    revision: AtomicU64,
    user_revision: AtomicU64,
    /// Serializes user mutations on their own connection, so they never wait
    /// behind the coordinator. Scans commit in short batches.
    user: Mutex<Connection>,
    subscriber: Mutex<Option<Publisher>>,
    last_publish: Mutex<Instant>,
    stopping: AtomicBool,
}

pub(crate) struct LibraryService {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

fn fault(error: QueryError) -> LibraryFault {
    match error {
        QueryError::Invalid => LibraryFault::Invalid,
        QueryError::Storage => LibraryFault::StorageFailed,
    }
}

impl Inner {
    fn with_reader<T>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T, QueryError>,
    ) -> Result<T, LibraryFault> {
        let pooled = self.readers.lock().ok().and_then(|mut pool| pool.pop());
        let connection = match pooled {
            Some(connection) => connection,
            None => {
                library::open_reader(&self.database).map_err(|_| LibraryFault::StorageFailed)?
            }
        };
        let result = work(&connection).map_err(fault);
        if let Ok(mut pool) = self.readers.lock()
            && pool.len() < 4
        {
            pool.push(connection);
        }
        result
    }

    fn summary(&self) -> Result<LibrarySummary, LibraryFault> {
        let (roots, counts) = self.with_reader(|db| Ok((query::roots(db)?, query::counts(db)?)))?;
        Ok(LibrarySummary {
            revision: self.revision.load(Ordering::Acquire).to_string(),
            user_revision: self.user_revision.load(Ordering::Acquire).to_string(),
            roots,
            tracks: counts.tracks,
            albums: counts.albums,
            artists: counts.artists,
            unavailable_tracks: counts.unavailable,
            scan: self.scan.lock().ok().and_then(|scan| scan.clone()),
        })
    }

    fn user_changed(&self) {
        self.user_revision.fetch_add(1, Ordering::AcqRel);
        self.publish(true);
    }

    /// Bumps the revision and pushes a summary. Progress ticks are throttled;
    /// state changes pass `force`.
    fn publish(&self, force: bool) {
        if let Ok(mut last) = self.last_publish.lock() {
            if !force && last.elapsed() < Duration::from_millis(150) {
                return;
            }
            *last = Instant::now();
        }
        self.revision.fetch_add(1, Ordering::AcqRel);
        let Ok(summary) = self.summary() else { return };
        if let Ok(mut subscriber) = self.subscriber.lock()
            && subscriber.as_ref().is_some_and(|send| !send(&summary))
        {
            *subscriber = None;
        }
    }
}

impl LibraryService {
    pub(crate) fn start(directory: &Path) -> Result<Self, LibraryFault> {
        Self::start_with(directory, Timing::default())
    }

    pub(crate) fn start_with(directory: &Path, timing: Timing) -> Result<Self, LibraryFault> {
        let database = directory.join("library.sqlite3");
        let mut writer = library::open(&database).map_err(|_| LibraryFault::StorageFailed)?;
        // A scan that was running when the process ended left only a status
        // row behind; no track depends on it.
        let _ = index::repair_interrupted(&writer, unix_now());
        if !index::search_is_consistent(&writer) {
            index::rebuild_search(&mut writer).map_err(|_| LibraryFault::StorageFailed)?;
        }
        // Unbounded on purpose: scan completion must never block behind a full
        // mailbox while the coordinator waits to join that scan. Watcher hints are
        // deduplicated per root, so the queue stays small.
        let (sender, receiver) = mpsc::channel();
        let user = library::open(&database).map_err(|_| LibraryFault::StorageFailed)?;
        let inner = Arc::new(Inner {
            database,
            artwork_dir: directory.join("artwork"),
            sender,
            readers: Mutex::new(Vec::new()),
            scan: Mutex::new(None),
            revision: AtomicU64::new(1),
            user_revision: AtomicU64::new(1),
            user: Mutex::new(user),
            subscriber: Mutex::new(None),
            last_publish: Mutex::new(Instant::now() - Duration::from_secs(1)),
            stopping: AtomicBool::new(false),
        });
        let mut coordinator = Coordinator::new(inner.clone(), writer, timing);
        let thread = thread::Builder::new()
            .name("echo-library".into())
            .spawn(move || coordinator.run(&receiver))
            .map_err(|_| LibraryFault::Unavailable)?;
        Ok(Self {
            inner,
            thread: Mutex::new(Some(thread)),
        })
    }

    pub(crate) fn subscribe(&self, publisher: Publisher) -> Result<LibrarySummary, LibraryFault> {
        if let Ok(mut slot) = self.inner.subscriber.lock() {
            *slot = Some(publisher);
        }
        self.inner.summary()
    }

    pub(crate) fn summary(&self) -> Result<LibrarySummary, LibraryFault> {
        self.inner.summary()
    }

    pub(crate) fn tracks(&self, request: &TrackQuery) -> Result<TrackPage, LibraryFault> {
        self.inner.with_reader(|db| query::tracks_page(db, request))
    }

    pub(crate) fn albums(&self, request: &AlbumQuery) -> Result<AlbumPage, LibraryFault> {
        self.inner.with_reader(|db| query::albums_page(db, request))
    }

    pub(crate) fn artists(&self, request: &ArtistQuery) -> Result<ArtistPage, LibraryFault> {
        self.inner
            .with_reader(|db| query::artists_page(db, request))
    }

    pub(crate) fn search(&self, text: &str, limit: u32) -> Result<SearchResults, LibraryFault> {
        self.inner.with_reader(|db| query::search(db, text, limit))
    }

    pub(crate) fn play_plan(&self, target: &PlayTarget) -> Result<PlayPlan, LibraryFault> {
        self.inner.with_reader(|db| query::play_plan(db, target))
    }

    /// Cached artwork bytes. The large variant is made on first use from the
    /// original embedded picture, and only if that picture still has the hash
    /// the index recorded. Otherwise the thumbnail is returned.
    pub(crate) fn artwork(&self, id: u32, size: ArtworkSize) -> Result<Vec<u8>, LibraryFault> {
        let dir = &self.inner.artwork_dir;
        let hash = self
            .inner
            .with_reader(|db| query::artwork_hash(db, id))?
            .ok_or(LibraryFault::NotFound)?;
        if size == ArtworkSize::Large {
            if let Ok(bytes) = artwork::load_large(dir, &hash) {
                return Ok(bytes);
            }
            for path in self.inner.with_reader(|db| query::artwork_files(db, id))? {
                let Some(picture) = source::open_selected(&path).ok().and_then(|file| {
                    meta::configure_thread();
                    meta::read(std::io::BufReader::new(file), "").ok()?.picture
                }) else {
                    continue;
                };
                if artwork::content_hash(&picture) != hash {
                    continue;
                }
                if let Ok(image) = artwork::make_variant(&picture, artwork::LARGE_EDGE)
                    && image.jpeg.len() as u64 <= artwork::MAX_LARGE_BYTES
                {
                    let _ = artwork::store_large(dir, &hash, &image);
                    return Ok(image.jpeg);
                }
            }
        }
        artwork::load(dir, &hash).map_err(|_| LibraryFault::NotFound)
    }

    /// Contrast-checked colors for an artwork, computed once and cached.
    pub(crate) fn palette(&self, id: u32) -> Result<Palette, LibraryFault> {
        if let Some(cached) = self
            .inner
            .with_reader(|db| query::artwork_palette(db, id))?
            && let Ok(palette) = serde_json::from_str(&cached)
        {
            return Ok(palette);
        }
        let bytes = self.artwork(id, ArtworkSize::Thumb)?;
        let palette = palette::extract(&bytes).ok_or(LibraryFault::NotFound)?;
        if let Ok(json) = serde_json::to_string(&palette)
            && let Ok(db) = self.inner.user.lock()
        {
            let _ = db.execute(
                "UPDATE artwork SET palette = ?2 WHERE id = ?1",
                rusqlite::params![i64::from(id), json],
            );
        }
        Ok(palette)
    }

    pub(crate) fn read<T>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T, QueryError>,
    ) -> Result<T, LibraryFault> {
        self.inner.with_reader(work)
    }

    /// Runs one user-data mutation and announces it once committed.
    pub(crate) fn mutate<T>(
        &self,
        work: impl FnOnce(&mut Connection, i64) -> Result<T, UserError>,
    ) -> Result<T, LibraryFault> {
        let result = {
            let mut db = self
                .inner
                .user
                .lock()
                .map_err(|_| LibraryFault::StorageFailed)?;
            work(&mut db, unix_now()).map_err(|error| match error {
                UserError::Invalid => LibraryFault::Invalid,
                UserError::NotFound => LibraryFault::NotFound,
                UserError::Storage => LibraryFault::StorageFailed,
            })?
        };
        self.inner.user_changed();
        Ok(result)
    }

    /// Accepts confirmed listening from the playback worker without blocking it.
    pub(crate) fn report_listened(&self, event: ListenEvent) {
        let _ = self.inner.sender.send(Msg::Listened(event));
    }

    fn command(&self, build: impl FnOnce(Reply) -> Cmd) -> Result<(), LibraryFault> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.inner
            .sender
            .send(Msg::Cmd(build(reply)))
            .map_err(|_| LibraryFault::Unavailable)?;
        answer.recv().map_err(|_| LibraryFault::Unavailable)?
    }

    pub(crate) fn add_root(&self, path: PathBuf) -> Result<(), LibraryFault> {
        self.command(|reply| Cmd::Add(path, reply))
    }

    pub(crate) fn remove_root(&self, id: u32) -> Result<(), LibraryFault> {
        self.command(|reply| Cmd::Remove(i64::from(id), reply))
    }

    pub(crate) fn scan(&self, root: Option<u32>) -> Result<(), LibraryFault> {
        self.command(|reply| Cmd::Scan(root.map(i64::from), reply))
    }

    pub(crate) fn cancel_scan(&self) -> Result<(), LibraryFault> {
        self.command(Cmd::Cancel)
    }

    /// Called when playback found a library file unopenable. Never blocks the
    /// caller; the coordinator checks the file and flags it if it is gone.
    pub(crate) fn report_unavailable(&self, track_id: u32) {
        let _ = self.inner.sender.send(Msg::Unavailable(track_id));
    }

    pub(crate) fn shutdown(&self) {
        self.inner.stopping.store(true, Ordering::Release);
        let _ = self.inner.sender.send(Msg::Wake);
        let thread = self.thread.lock().ok().and_then(|mut slot| slot.take());
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

impl Drop for LibraryService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct RootState {
    path: PathBuf,
    /// Set while a hint for this root sits in the mailbox.
    hint_pending: Arc<AtomicBool>,
    online: bool,
    watcher: Option<RootWatcher>,
    dirty_since: Option<Instant>,
    last_scan_end: Option<Instant>,
}

struct Running {
    root_id: i64,
    cancel: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

struct Coordinator {
    inner: Arc<Inner>,
    writer: Connection,
    timing: Timing,
    roots: HashMap<i64, RootState>,
    queue: VecDeque<i64>,
    running: Option<Running>,
    play: Option<(u64, i64)>,
    next_health: Instant,
    next_reconcile: Instant,
}

impl Coordinator {
    fn new(inner: Arc<Inner>, writer: Connection, timing: Timing) -> Self {
        Self {
            inner,
            writer,
            roots: HashMap::new(),
            queue: VecDeque::new(),
            running: None,
            play: None,
            next_health: Instant::now() + timing.health,
            next_reconcile: Instant::now() + timing.reconcile,
            timing,
        }
    }

    fn run(&mut self, receiver: &mpsc::Receiver<Msg>) {
        self.load_roots();
        // Reconcile at startup: files may have changed while ECHO was closed.
        let online: Vec<i64> = self
            .roots
            .iter()
            .filter(|(_, r)| r.online)
            .map(|(id, _)| *id)
            .collect();
        for id in online {
            self.enqueue(id);
        }
        self.start_next();
        self.inner.publish(true);
        while !self.inner.stopping.load(Ordering::Acquire) {
            match receiver.recv_timeout(self.timing.tick) {
                Ok(message) => self.handle(message),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.tick();
        }
        // Playback shuts down first and its final listening update may be queued.
        while let Ok(message) = receiver.try_recv() {
            if let Msg::Listened(event) = message {
                self.record(event);
            }
        }
        if let Some(running) = self.running.take() {
            running.cancel.store(true, Ordering::Release);
            let _ = running.thread.join();
        }
    }

    fn load_roots(&mut self) {
        let Ok(mut statement) = self
            .writer
            .prepare("SELECT id, path, online FROM library_roots")
        else {
            return;
        };
        let rows: Vec<(i64, String, bool)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default();
        drop(statement);
        for (id, path, online) in rows {
            let mut state = RootState {
                path: PathBuf::from(path),
                hint_pending: Arc::new(AtomicBool::new(false)),
                online,
                watcher: None,
                dirty_since: None,
                last_scan_end: None,
            };
            if state.online {
                state.watcher = self.watch(id, &state.path, &state.hint_pending);
            }
            self.roots.insert(id, state);
        }
    }

    fn watch(&self, root_id: i64, path: &Path, pending: &Arc<AtomicBool>) -> Option<RootWatcher> {
        let sender = self.inner.sender.clone();
        let pending = pending.clone();
        RootWatcher::start(path, move |failed| {
            // One queued hint per root is enough: the rescan reads the disk,
            // not the event list. A failure always goes through.
            if failed || !pending.swap(true, Ordering::AcqRel) {
                let _ = sender.send(Msg::Hint {
                    root_id,
                    failed,
                    pending: pending.clone(),
                });
            }
        })
        .ok()
    }

    fn handle(&mut self, message: Msg) {
        match message {
            Msg::Wake => {}
            Msg::Cmd(command) => self.command(command),
            Msg::Hint {
                root_id,
                failed,
                pending,
            } => {
                pending.store(false, Ordering::Release);
                if let Some(root) = self.roots.get_mut(&root_id) {
                    root.dirty_since = Some(Instant::now());
                }
                if failed {
                    self.next_health = Instant::now();
                }
            }
            Msg::Unavailable(track_id) => self.check_track(track_id),
            Msg::Listened(event) => self.record(event),
            Msg::ScanFinished { root_id, result } => {
                if self
                    .running
                    .as_ref()
                    .is_some_and(|running| running.root_id == root_id)
                    && let Some(running) = self.running.take()
                {
                    let _ = running.thread.join();
                }
                if let Some(root) = self.roots.get_mut(&root_id) {
                    root.last_scan_end = Some(Instant::now());
                }
                if let Err(error) = result {
                    eprintln!("ECHO library scan failed: {error}");
                }
                if let Ok(mut scan) = self.inner.scan.lock() {
                    *scan = None;
                }
                self.sync_online_state();
                self.inner.publish(true);
                self.start_next();
            }
        }
    }

    fn command(&mut self, command: Cmd) {
        match command {
            Cmd::Add(path, reply) => {
                let _ = reply.send(self.add_root(&path));
            }
            Cmd::Remove(id, reply) => {
                let _ = reply.send(self.remove_root(id));
            }
            Cmd::Scan(root, reply) => {
                let targets: Vec<i64> = match root {
                    Some(id) if self.roots.contains_key(&id) => vec![id],
                    Some(_) => {
                        let _ = reply.send(Err(LibraryFault::NotFound));
                        return;
                    }
                    None => self.roots.keys().copied().collect(),
                };
                for id in targets {
                    self.enqueue(id);
                }
                self.start_next();
                self.inner.publish(true);
                let _ = reply.send(Ok(()));
            }
            Cmd::Cancel(reply) => {
                self.queue.clear();
                if let Some(running) = &self.running {
                    running.cancel.store(true, Ordering::Release);
                }
                let _ = reply.send(Ok(()));
            }
        }
    }

    fn add_root(&mut self, path: &Path) -> Result<(), LibraryFault> {
        let normalized = source::open_directory(path).map_err(|error| match error {
            SourceError::Path | SourceError::Volume | SourceError::ReparseOrOffline => {
                LibraryFault::UnsupportedLocation
            }
            SourceError::Unavailable | SourceError::NotAFile => LibraryFault::Invalid,
        })?;
        let text = normalized
            .to_str()
            .ok_or(LibraryFault::UnsupportedLocation)?;
        let id = index::add_root(&self.writer, text, unix_now()).map_err(|error| match error {
            index::RootError::Duplicate => LibraryFault::AlreadyAdded,
            index::RootError::Overlapping => LibraryFault::Overlapping,
            index::RootError::Storage => LibraryFault::StorageFailed,
        })?;
        let hint_pending = Arc::new(AtomicBool::new(false));
        let watcher = self.watch(id, &normalized, &hint_pending);
        self.roots.insert(
            id,
            RootState {
                path: normalized,
                hint_pending,
                online: true,
                watcher,
                dirty_since: None,
                last_scan_end: None,
            },
        );
        self.enqueue(id);
        self.start_next();
        self.inner.publish(true);
        Ok(())
    }

    /// Stops any scan of the root first so no writer is mid-batch when its
    /// rows are removed. Music files are never touched.
    fn remove_root(&mut self, id: i64) -> Result<(), LibraryFault> {
        if !self.roots.contains_key(&id) {
            return Err(LibraryFault::NotFound);
        }
        self.queue.retain(|queued| *queued != id);
        if self
            .running
            .as_ref()
            .is_some_and(|running| running.root_id == id)
            && let Some(running) = self.running.take()
        {
            running.cancel.store(true, Ordering::Release);
            let _ = running.thread.join();
            if let Ok(mut scan) = self.inner.scan.lock() {
                *scan = None;
            }
        }
        self.roots.remove(&id);
        let hashes =
            index::remove_root(&mut self.writer, id).map_err(|_| LibraryFault::StorageFailed)?;
        for hash in hashes {
            artwork::remove(&self.inner.artwork_dir, &hash);
        }
        self.start_next();
        self.inner.publish(true);
        Ok(())
    }

    fn enqueue(&mut self, id: i64) {
        let running = self
            .running
            .as_ref()
            .is_some_and(|running| running.root_id == id);
        if !running && !self.queue.contains(&id) {
            self.queue.push_back(id);
        }
    }

    fn start_next(&mut self) {
        if self.running.is_some() || self.inner.stopping.load(Ordering::Acquire) {
            return;
        }
        while let Some(root_id) = self.queue.pop_front() {
            let Some(path) = self.roots.get(&root_id).map(|root| root.path.clone()) else {
                continue;
            };
            let cancel = Arc::new(AtomicBool::new(false));
            let inner = self.inner.clone();
            let env = ScanEnv {
                database: self.inner.database.clone(),
                artwork_dir: self.inner.artwork_dir.clone(),
                cancel: cancel.clone(),
                workers: thread::available_parallelism()
                    .map_or(2, |n| n.get().saturating_sub(1).clamp(1, 4)),
                progress: Arc::new({
                    let inner = inner.clone();
                    move |view| {
                        if let Ok(mut scan) = inner.scan.lock() {
                            *scan = Some(view);
                        }
                        inner.publish(false);
                    }
                }),
                queued: u32::try_from(self.queue.len()).unwrap_or(u32::MAX),
                on_batch: None,
                max_pages: None,
            };
            if let Ok(mut scan) = self.inner.scan.lock() {
                *scan = Some(ScanView {
                    root_id: query::id32(root_id),
                    phase: ScanPhase::Discovering,
                    discovered: 0,
                    processed: 0,
                    indexed: 0,
                    unchanged: 0,
                    skipped: 0,
                    failed: 0,
                    total_known: false,
                    queued: env.queued,
                });
            }
            let sender = self.inner.sender.clone();
            let spawned = thread::Builder::new()
                .name("echo-scan".into())
                .spawn(move || {
                    let result = scan::run(&env, root_id, &path);
                    // The coordinator joins this thread, so a full mailbox must not
                    // lose the completion message: block until it is accepted.
                    let _ = sender.send(Msg::ScanFinished { root_id, result });
                });
            match spawned {
                Ok(thread) => {
                    self.running = Some(Running {
                        root_id,
                        cancel,
                        thread,
                    });
                    self.inner.publish(true);
                    return;
                }
                Err(_) => {
                    if let Ok(mut scan) = self.inner.scan.lock() {
                        *scan = None;
                    }
                }
            }
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if now >= self.next_health {
            self.next_health = now + self.timing.health;
            self.check_health();
        }
        if now >= self.next_reconcile {
            self.next_reconcile = now + self.timing.reconcile;
            let online: Vec<i64> = self
                .roots
                .iter()
                .filter(|(_, r)| r.online)
                .map(|(id, _)| *id)
                .collect();
            for id in online {
                self.enqueue(id);
            }
        }
        let due: Vec<i64> = self
            .roots
            .iter()
            .filter(|(_, root)| {
                root.online
                    && root
                        .dirty_since
                        .is_some_and(|since| now.duration_since(since) >= self.timing.debounce)
                    && root
                        .last_scan_end
                        .is_none_or(|end| now.duration_since(end) >= self.timing.min_interval)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            if let Some(root) = self.roots.get_mut(&id) {
                root.dirty_since = None;
            }
            self.enqueue(id);
        }
        self.start_next();
    }

    /// Detects drives that were unplugged or returned. Offline roots keep
    /// their tracks; the query layer reports those tracks as unavailable.
    fn check_health(&mut self) {
        let mut changed = false;
        let ids: Vec<i64> = self.roots.keys().copied().collect();
        for id in ids {
            let scanning = self
                .running
                .as_ref()
                .is_some_and(|running| running.root_id == id);
            let Some(path) = self.roots.get(&id).map(|root| root.path.clone()) else {
                continue;
            };
            let reachable = source::open_directory(&path).is_ok();
            let Some(was) = self.roots.get(&id).map(|root| root.online) else {
                continue;
            };
            if reachable == was || scanning {
                continue;
            }
            let _ = index::set_root_online(&self.writer, id, reachable);
            let pending = self
                .roots
                .get(&id)
                .map(|root| root.hint_pending.clone())
                .unwrap_or_default();
            let watcher = if reachable {
                self.watch(id, &path, &pending)
            } else {
                None
            };
            if let Some(root) = self.roots.get_mut(&id) {
                root.online = reachable;
                root.watcher = watcher;
            }
            if reachable {
                self.enqueue(id);
            }
            changed = true;
        }
        if changed {
            self.inner.publish(true);
        }
    }

    /// Mirrors the online flag a scan wrote into the coordinator's view.
    fn sync_online_state(&mut self) {
        let Ok(mut statement) = self.writer.prepare("SELECT id, online FROM library_roots") else {
            return;
        };
        let rows: Vec<(i64, bool)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default();
        drop(statement);
        for (id, online) in rows {
            let path = self.roots.get(&id).map(|root| root.path.clone());
            let was = self.roots.get(&id).map(|root| root.online);
            if let (Some(path), Some(was)) = (path, was)
                && was != online
            {
                let pending = self
                    .roots
                    .get(&id)
                    .map(|root| root.hint_pending.clone())
                    .unwrap_or_default();
                let watcher = if online {
                    self.watch(id, &path, &pending)
                } else {
                    None
                };
                if let Some(root) = self.roots.get_mut(&id) {
                    root.online = online;
                    root.watcher = watcher;
                }
            }
        }
    }

    /// One playback generation owns at most one history row.
    fn record(&mut self, event: ListenEvent) {
        if let Some((generation, play)) = self.play
            && generation == event.generation
        {
            // Totals are not worth refreshing every open view for.
            let _ = user::update_play(&self.writer, play, event.listened_ms);
            return;
        } else if let Ok(Some(play)) = user::record_play(
            &mut self.writer,
            event.track_id,
            event.started_at,
            event.listened_ms,
        ) {
            self.play = Some((event.generation, play));
        } else {
            return;
        }
        self.inner.user_changed();
    }

    fn check_track(&mut self, track_id: u32) {
        let found: Option<(String, String, bool)> = self
            .writer
            .query_row(
                "SELECT r.path, t.rel_path, r.online FROM tracks t JOIN library_roots r ON r.id = t.root_id WHERE t.id = ?1",
                [i64::from(track_id)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();
        let Some((root, rel, online)) = found else {
            return;
        };
        // An offline root already reports its tracks unavailable; only flag a
        // file that is verifiably gone from a reachable root.
        let gone = online
            && matches!(
                std::fs::symlink_metadata(Path::new(&root).join(rel)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound
            );
        if gone && index::mark_track_missing(&self.writer, i64::from(track_id)).unwrap_or(false) {
            self.inner.publish(true);
        }
    }
}

#[cfg(test)]
mod tests;
