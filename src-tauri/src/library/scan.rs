//! Filesystem scan pipeline: one walker, a few metadata workers, one writer.
//!
//! ```text
//! walker --Read--> [bounded] --> workers --Done/Failed--> [bounded] --> writer (SQLite)
//!    \------------------------Seen(id)---------------------------------^
//! ```
//!
//! * Memory is bounded by the two channel capacities, the batch size and the
//!   artwork decode gate, not by library size.
//! * Unchanged files (same size, timestamp and extractor version) are only
//!   stamped as seen; their tags are never read again.
//! * Every file is opened through [`open_selected`], which refuses reparse
//!   points on any ancestor, so a junction swapped in mid-scan cannot redirect
//!   a read. Directories that are reparse points are skipped, which also
//!   breaks junction and symlink loops.
//! * The scan never deletes a track. It may flag tracks missing, and only
//!   after the walker visited every directory it could list.

use super::{
    artwork,
    index::{self, ArtworkRef, Finalized, ScanTotals, Stamp, Upsert},
    meta::{self, METADATA_VERSION, ReadError, TrackMeta},
    text::clean,
};
use crate::{
    contract::{ScanPhase, ScanStatus, ScanView},
    library,
    playback::source::{self, open_selected},
};
use rusqlite::{Connection, TransactionBehavior, params};
use std::{
    collections::{HashMap, HashSet},
    io::BufReader,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Extensions the player build can decode. The picker uses the same list.
pub(crate) const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "wav", "ogg", "oga", "m4a", "aac"];
const MAX_DEPTH: usize = 64;
const MAX_PATH_UNITS: usize = 32_000;
const CHANNEL: usize = 256;
const BATCH: usize = 200;
const FLUSH: Duration = Duration::from_millis(300);
const PROGRESS_EVERY: Duration = Duration::from_millis(200);
const MAX_UNREADABLE: usize = 256;

const REPARSE_POINT: u32 = 0x400;
const OFFLINE: u32 = 0x1000;
const SYSTEM: u32 = 0x4;
const RECALL_ON_OPEN: u32 = 0x4_0000;
const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

/// Counters shared between pipeline stages and the progress reporter.
#[derive(Default)]
pub(crate) struct Counters {
    pub discovered: AtomicU32,
    pub processed: AtomicU32,
    pub indexed: AtomicU32,
    pub unchanged: AtomicU32,
    pub skipped: AtomicU32,
    pub failed: AtomicU32,
    pub walk_done: AtomicBool,
}

impl Counters {
    fn view(&self, root_id: i64, phase: ScanPhase, queued: u32) -> ScanView {
        let get = |value: &AtomicU32| value.load(Ordering::Relaxed);
        let walk_done = self.walk_done.load(Ordering::Acquire);
        ScanView {
            root_id: library::query::id32(root_id),
            phase: match phase {
                ScanPhase::Discovering if walk_done => ScanPhase::Reading,
                other => other,
            },
            discovered: get(&self.discovered),
            processed: get(&self.processed),
            indexed: get(&self.indexed),
            unchanged: get(&self.unchanged),
            skipped: get(&self.skipped),
            failed: get(&self.failed),
            total_known: walk_done,
            queued,
        }
    }

    fn totals(&self, missing: usize) -> ScanTotals {
        let get = |value: &AtomicU32| value.load(Ordering::Relaxed);
        ScanTotals {
            discovered: get(&self.discovered),
            indexed: get(&self.indexed),
            unchanged: get(&self.unchanged),
            skipped: get(&self.skipped),
            failed: get(&self.failed),
            missing: u32::try_from(missing).unwrap_or(u32::MAX),
        }
    }
}

pub(crate) struct ScanEnv {
    pub database: PathBuf,
    pub artwork_dir: PathBuf,
    pub cancel: Arc<AtomicBool>,
    pub workers: usize,
    /// Called from the writer thread, at most every 200 ms and at each phase change.
    pub progress: Arc<dyn Fn(ScanView) + Send + Sync>,
    pub queued: u32,
    /// Called after each committed batch with the number of files processed so
    /// far. Tests use it to cancel at a deterministic point.
    pub on_batch: Option<Arc<dyn Fn(u32) + Send + Sync>>,
    /// Page ceiling for the scan's own connection. Tests use it to make SQLite
    /// report a full disk at a real write.
    pub max_pages: Option<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ScanOutcome {
    pub status: ScanStatus,
    pub discovered: u32,
    pub indexed: u32,
    pub unchanged: u32,
    pub failed: u32,
    pub skipped: u32,
    pub missing: u32,
    /// Orphaned thumbnail hashes already deleted from the index.
    pub removed_artwork: usize,
}

struct ReadJob {
    rel: String,
    abs: PathBuf,
    stamp: Stamp,
    folder_key: String,
    moved_from: Option<i64>,
}

struct Done {
    job: ReadJob,
    meta: TrackMeta,
    artwork: Option<ArtworkRef>,
}

enum Message {
    Seen(i64),
    Done(Box<Done>),
    Failed,
}

#[derive(Default)]
struct WalkSummary {
    /// False when cancelled or when the channel closed early.
    finished: bool,
    unreadable_dirs: Vec<String>,
    /// More unreadable directories than we track: treat the walk as untrusted.
    overflow: bool,
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

fn mtime_ns(time: std::io::Result<SystemTime>) -> i64 {
    match time {
        Ok(time) => match time.duration_since(UNIX_EPOCH) {
            Ok(after) => i64::try_from(after.as_nanos()).unwrap_or(i64::MAX),
            Err(before) => -i64::try_from(before.duration().as_nanos()).unwrap_or(i64::MAX),
        },
        Err(_) => 0,
    }
}

fn is_audio(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, extension)| {
        AUDIO_EXTENSIONS
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
    })
}

fn file_stem(name: &str) -> String {
    clean(name.rsplit_once('.').map_or(name, |(stem, _)| stem))
}

/// Walks the root. Sends one message per audio file and records directories
/// that could not be listed so the caller never flags their tracks missing.
fn walk(
    root: &Path,
    root_id: i64,
    reader: &Connection,
    work: &SyncSender<ReadJob>,
    messages: &SyncSender<Message>,
    counters: &Counters,
    cancel: &AtomicBool,
) -> WalkSummary {
    let mut summary = WalkSummary::default();
    let mut stack = vec![(root.to_path_buf(), String::new(), 0_usize)];
    let mut claimed: HashSet<i64> = HashSet::new();
    let Ok(mut by_path) = reader
        .prepare("SELECT id, size, mtime_ns, metadata_version FROM tracks WHERE root_id = ?1 AND path_key = ?2")
    else {
        return summary;
    };
    let Ok(mut by_stamp) = reader.prepare(
        "SELECT id, rel_path FROM tracks WHERE root_id = ?1 AND size = ?2 AND mtime_ns = ?3 AND path_key <> ?4",
    ) else {
        return summary;
    };
    let unreadable = |summary: &mut WalkSummary, key: String| {
        if summary.unreadable_dirs.len() < MAX_UNREADABLE {
            summary.unreadable_dirs.push(key);
        } else {
            summary.overflow = true;
        }
    };

    while let Some((directory, rel_dir, depth)) = stack.pop() {
        if cancel.load(Ordering::Acquire) {
            return summary;
        }
        let Ok(entries) = std::fs::read_dir(&directory) else {
            unreadable(&mut summary, rel_dir.to_lowercase());
            continue;
        };
        for entry in entries {
            if cancel.load(Ordering::Acquire) {
                return summary;
            }
            let Ok(entry) = entry else {
                // The listing broke part way, so some children were never seen.
                unreadable(&mut summary, rel_dir.to_lowercase());
                break;
            };
            let Ok(metadata) = entry.metadata() else {
                counters.skipped.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            let attributes = metadata.file_attributes();
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                // A name that is not valid Unicode cannot be stored faithfully.
                counters.skipped.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            if attributes & SYSTEM != 0 {
                continue;
            }
            if attributes & (REPARSE_POINT | OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0
            {
                // Junctions, symlinks and cloud placeholders are neither
                // followed nor read, so loops and implicit downloads are impossible.
                counters.skipped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            if metadata.is_dir() {
                if depth + 1 > MAX_DEPTH {
                    counters.skipped.fetch_add(1, Ordering::Relaxed);
                    unreadable(
                        &mut summary,
                        format!("{}{}\\", rel_dir.to_lowercase(), name.to_lowercase()),
                    );
                } else {
                    stack.push((entry.path(), format!("{rel_dir}{name}\\"), depth + 1));
                }
                continue;
            }
            if !metadata.is_file() || !is_audio(name) {
                continue;
            }
            let rel = format!("{rel_dir}{name}");
            if rel.encode_utf16().count() + root.as_os_str().len() > MAX_PATH_UNITS {
                counters.skipped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            counters.discovered.fetch_add(1, Ordering::Relaxed);
            let stamp = Stamp {
                size: metadata.len(),
                mtime_ns: mtime_ns(metadata.modified()),
            };
            let key = index::path_key(&rel);
            let known: Option<(i64, i64, i64, i64)> = by_path
                .query_row(params![root_id, key], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .ok();
            let mut moved_from = None;
            match known {
                Some((id, size, modified, version))
                    if size == stamp.size as i64
                        && modified == stamp.mtime_ns
                        && version == METADATA_VERSION =>
                {
                    counters.unchanged.fetch_add(1, Ordering::Relaxed);
                    if messages.send(Message::Seen(id)).is_err() {
                        return summary;
                    }
                    continue;
                }
                Some(_) => {}
                None => {
                    // A new path with the fingerprint of a row whose old file is
                    // gone is a rename or move: keep the track identity.
                    let candidates: Vec<(i64, String)> = by_stamp
                        .query_map(
                            params![root_id, stamp.size as i64, stamp.mtime_ns, key],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        )
                        .map(|rows| rows.filter_map(Result::ok).take(8).collect())
                        .unwrap_or_default();
                    moved_from = candidates.into_iter().find_map(|(id, old)| {
                        let gone = matches!(
                            std::fs::symlink_metadata(root.join(&old)),
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound
                        );
                        (gone && claimed.insert(id)).then_some(id)
                    });
                }
            }
            let job = ReadJob {
                folder_key: format!("{}\\{rel_dir}", root.display()).to_lowercase(),
                abs: entry.path(),
                rel,
                stamp,
                moved_from,
            };
            if work.send(job).is_err() {
                return summary;
            }
        }
    }
    summary.finished = true;
    summary
}

fn read_worker(
    jobs: &Mutex<Receiver<ReadJob>>,
    messages: &SyncSender<Message>,
    known_art: &Mutex<HashMap<String, ArtworkRef>>,
    env: &ScanEnv,
) {
    meta::configure_thread();
    loop {
        if env.cancel.load(Ordering::Acquire) {
            return;
        }
        // The lock is held only for the dequeue, never while reading a file.
        let job = match jobs.lock().map(|rx| rx.recv()) {
            Ok(Ok(job)) => job,
            _ => return,
        };
        let outcome = process(job, known_art, env);
        let message = match outcome {
            Some(done) => Message::Done(Box::new(done)),
            None => Message::Failed,
        };
        if messages.send(message).is_err() {
            return;
        }
    }
}

fn process(
    job: ReadJob,
    known_art: &Mutex<HashMap<String, ArtworkRef>>,
    env: &ScanEnv,
) -> Option<Done> {
    let file = open_selected(&job.abs).ok()?;
    let name = job.rel.rsplit('\\').next().unwrap_or(&job.rel);
    let mut meta = match meta::read(BufReader::with_capacity(64 * 1024, file), &file_stem(name)) {
        Ok(meta) => meta,
        Err(ReadError::Unrecognized | ReadError::Io) => return None,
    };
    // Covers are keyed by content hash. A cover already cached, in this scan or
    // an earlier one, reuses its recorded dimensions; the writer links it to the
    // row. Malformed, unsupported or oversized covers simply leave the track
    // without artwork rather than failing the file.
    let artwork = meta.picture.take().and_then(|bytes| {
        let hash = artwork::content_hash(&bytes);
        if let Some(known) = known_art
            .lock()
            .ok()
            .and_then(|map| map.get(&hash).cloned())
        {
            return Some(known);
        }
        let thumbnail = artwork::make_thumbnail(&bytes).ok()?;
        artwork::store(&env.artwork_dir, &hash, &thumbnail).ok()?;
        let reference = ArtworkRef {
            hash: hash.clone(),
            width: thumbnail.width,
            height: thumbnail.height,
            bytes: thumbnail.jpeg.len() as u64,
        };
        if let Ok(mut map) = known_art.lock() {
            map.insert(hash, reference.clone());
        }
        Some(reference)
    });
    Some(Done { job, meta, artwork })
}

fn write_batch(
    db: &mut Connection,
    root_id: i64,
    scan_id: i64,
    seen: &mut Vec<i64>,
    done: &mut Vec<Done>,
    counters: &Counters,
) -> rusqlite::Result<()> {
    if seen.is_empty() && done.is_empty() {
        return Ok(());
    }
    // Batch upserts can read before their first write. Reserve the writer
    // slot up front so a playback checkpoint cannot invalidate that snapshot.
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    index::mark_seen(&tx, seen, scan_id)?;
    let now = now_seconds();
    for item in done.iter() {
        index::upsert_track(
            &tx,
            &Upsert {
                root_id,
                rel_path: &item.job.rel,
                stamp: item.job.stamp,
                meta: &item.meta,
                artwork: item.artwork.as_ref(),
                folder_key: &item.job.folder_key,
                scan_id,
                moved_from: item.job.moved_from,
                now,
            },
        )?;
    }
    tx.commit()?;
    counters
        .indexed
        .fetch_add(done.len() as u32, Ordering::Relaxed);
    counters
        .processed
        .fetch_add((seen.len() + done.len()) as u32, Ordering::Relaxed);
    seen.clear();
    done.clear();
    Ok(())
}

/// Records a scan that could not start because the root is unreachable. No
/// track row changes, so a disconnected drive keeps its whole index.
fn record_offline(db: &Connection, root_id: i64) -> rusqlite::Result<ScanOutcome> {
    let now = now_seconds();
    let scan = index::begin_scan(db, root_id, now)?;
    index::set_root_online(db, root_id, false)?;
    index::finish_scan(
        db,
        scan,
        "offline",
        &ScanTotals {
            discovered: 0,
            indexed: 0,
            unchanged: 0,
            skipped: 0,
            failed: 0,
            missing: 0,
        },
        now,
    )?;
    Ok(outcome(ScanStatus::Offline, &Counters::default(), 0, 0))
}

fn outcome(status: ScanStatus, counters: &Counters, missing: usize, removed: usize) -> ScanOutcome {
    let get = |value: &AtomicU32| value.load(Ordering::Relaxed);
    ScanOutcome {
        status,
        discovered: get(&counters.discovered),
        indexed: get(&counters.indexed),
        unchanged: get(&counters.unchanged),
        failed: get(&counters.failed),
        skipped: get(&counters.skipped),
        missing: u32::try_from(missing).unwrap_or(u32::MAX),
        removed_artwork: removed,
    }
}

/// Scans one root to completion, cancellation or failure. Blocks the calling
/// thread; the service runs it on a dedicated scan thread.
pub(crate) fn run(env: &ScanEnv, root_id: i64, root: &Path) -> rusqlite::Result<ScanOutcome> {
    let mut db = library::open(&env.database).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let root = match source::open_directory(root) {
        Ok(path) => path,
        Err(_) => return record_offline(&db, root_id),
    };
    let now = now_seconds();
    let scan_id = index::begin_scan(&db, root_id, now)?;
    index::set_root_online(&db, root_id, true)?;

    if let Some(pages) = env.max_pages {
        db.pragma_update_and_check(None, "max_page_count", pages, |_| Ok(()))?;
    }

    let counters = Arc::new(Counters::default());
    let known_art: Arc<Mutex<HashMap<String, ArtworkRef>>> = Arc::new(Mutex::new(
        db.prepare("SELECT hash, width, height, bytes FROM artwork")?
            .query_map([], |row| {
                let hash: String = row.get(0)?;
                Ok((
                    hash.clone(),
                    ArtworkRef {
                        hash,
                        width: row.get(1)?,
                        height: row.get(2)?,
                        bytes: row.get::<_, i64>(3)? as u64,
                    },
                ))
            })?
            .filter_map(Result::ok)
            .collect(),
    ));
    let (work_tx, work_rx) = mpsc::sync_channel::<ReadJob>(CHANNEL);
    let (message_tx, message_rx) = mpsc::sync_channel::<Message>(CHANNEL);
    let work_rx = Arc::new(Mutex::new(work_rx));
    let env_arc = Arc::new(ScanEnv {
        database: env.database.clone(),
        artwork_dir: env.artwork_dir.clone(),
        cancel: env.cancel.clone(),
        workers: env.workers,
        progress: env.progress.clone(),
        queued: env.queued,
        on_batch: env.on_batch.clone(),
        max_pages: env.max_pages,
    });

    let walker = {
        let (counters, message_tx, root, cancel, database) = (
            counters.clone(),
            message_tx.clone(),
            root.clone(),
            env.cancel.clone(),
            env.database.clone(),
        );
        thread::Builder::new()
            .name("echo-scan-walk".into())
            .spawn(move || {
                let summary = match library::open_reader(&database) {
                    Ok(reader) => walk(
                        &root,
                        root_id,
                        &reader,
                        &work_tx,
                        &message_tx,
                        &counters,
                        &cancel,
                    ),
                    Err(_) => WalkSummary::default(),
                };
                counters
                    .walk_done
                    .store(summary.finished, Ordering::Release);
                summary
            })
    };
    let Ok(walker) = walker else {
        index::finish_scan(&db, scan_id, "failed", &counters.totals(0), now_seconds())?;
        return Ok(outcome(ScanStatus::Failed, &counters, 0, 0));
    };
    let workers: Vec<_> = (0..env.workers.max(1))
        .filter_map(|index| {
            let (jobs, messages, known, env) = (
                work_rx.clone(),
                message_tx.clone(),
                known_art.clone(),
                env_arc.clone(),
            );
            thread::Builder::new()
                .name(format!("echo-scan-read-{index}"))
                .spawn(move || read_worker(&jobs, &messages, &known, &env))
                .ok()
        })
        .collect();
    // The writer must see the channel close when every sender has finished.
    drop(message_tx);
    drop(work_rx);

    let progress = |phase: ScanPhase| (env.progress)(counters.view(root_id, phase, env.queued));
    progress(ScanPhase::Discovering);
    let (mut seen, mut done): (Vec<i64>, Vec<Done>) = (Vec::new(), Vec::new());
    let (mut last_flush, mut last_progress) = (Instant::now(), Instant::now());
    let mut storage_error = None;
    loop {
        match message_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Message::Seen(id)) => seen.push(id),
            Ok(Message::Done(item)) => done.push(*item),
            Ok(Message::Failed) => {
                counters.failed.fetch_add(1, Ordering::Relaxed);
                counters.processed.fetch_add(1, Ordering::Relaxed);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if seen.len() + done.len() >= BATCH || last_flush.elapsed() >= FLUSH {
            if let Err(error) =
                write_batch(&mut db, root_id, scan_id, &mut seen, &mut done, &counters)
            {
                storage_error = Some(error);
                env.cancel.store(true, Ordering::Release);
                break;
            }
            last_flush = Instant::now();
            if let Some(hook) = &env.on_batch {
                hook(counters.processed.load(Ordering::Relaxed));
            }
        }
        if last_progress.elapsed() >= PROGRESS_EVERY {
            progress(ScanPhase::Discovering);
            last_progress = Instant::now();
        }
        if env.cancel.load(Ordering::Acquire) {
            break;
        }
    }
    // Commit what arrived before cancellation: each item is a complete,
    // consistent row, and keeping it means the next scan skips that work.
    if storage_error.is_none() {
        storage_error =
            write_batch(&mut db, root_id, scan_id, &mut seen, &mut done, &counters).err();
    }
    drop(message_rx);
    let summary = walker.join().unwrap_or_default();
    for worker in workers {
        let _ = worker.join();
    }
    progress(ScanPhase::Finalizing);

    let cancelled = env.cancel.load(Ordering::Acquire);
    let finished_at = now_seconds();
    if let Some(error) = storage_error {
        index::finish_scan(&db, scan_id, "failed", &counters.totals(0), finished_at)?;
        return Err(error);
    }
    if cancelled || !summary.finished {
        index::finish_scan(&db, scan_id, "cancelled", &counters.totals(0), finished_at)?;
        return Ok(outcome(ScanStatus::Cancelled, &counters, 0, 0));
    }

    // The walk visited everything it could list. Flag absent files only now.
    let trusted = !summary.overflow;
    // Finalization reads the scan snapshot before flagging missing files.
    // A deferred transaction can fail immediately on that read-to-write
    // upgrade (SQLITE_BUSY_SNAPSHOT), regardless of the busy timeout.
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let finalized = if trusted {
        index::finalize(&tx, root_id, scan_id, &summary.unreadable_dirs)?
    } else {
        Finalized::Marked { missing: 0 }
    };
    let (status, label, missing) = match finalized {
        Finalized::SuspiciouslyEmpty => (ScanStatus::Offline, "offline", 0),
        Finalized::Marked { missing } if summary.unreadable_dirs.is_empty() && trusted => {
            tx.execute(
                "UPDATE library_roots SET last_complete_scan_at = ?2, online = 1 WHERE id = ?1",
                params![root_id, finished_at],
            )?;
            (ScanStatus::Complete, "complete", missing)
        }
        Finalized::Marked { missing } => (ScanStatus::Partial, "partial", missing),
    };
    if status == ScanStatus::Offline {
        index::set_root_online(&tx, root_id, false)?;
    }
    let orphans = index::collect_garbage(&tx)?;
    index::finish_scan(&tx, scan_id, label, &counters.totals(missing), finished_at)?;
    tx.commit()?;
    for hash in &orphans {
        artwork::remove(&env.artwork_dir, hash);
    }
    Ok(outcome(status, &counters, missing, orphans.len()))
}

#[cfg(test)]
mod tests;
