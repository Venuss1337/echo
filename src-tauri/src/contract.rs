//! Wire types and deterministic TypeScript generation.

use serde::{Deserialize, Serialize};
use ts_rs::{Config, TS};

/// Sanitized state of the app-owned library database.
#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LibraryState {
    Opening,
    Empty,
    Unavailable,
}

/// Read-only renderer bootstrap status.
#[derive(Clone, Copy, Debug, Serialize, TS)]
pub struct AppStatus {
    pub library: LibraryState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PlayerState {
    Idle,
    Loading,
    Playing,
    Paused,
    Buffering,
    Stopping,
    Ended,
    Error,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RepeatMode {
    Off,
    One,
    All,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueTrack {
    pub id: String,
    pub title: String,
    /// Artist for rows that came from the library; `None` for picked files.
    pub subtitle: Option<String>,
    /// Cached artwork for library rows.
    pub artwork_id: Option<u32>,
    /// Index row the entry came from. It may no longer exist.
    pub track_id: Option<u32>,
    /// The last attempt to open this entry failed.
    pub unavailable: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlayerView {
    pub revision: String,
    pub generation: String,
    pub state: PlayerState,
    pub position_ms: f64,
    pub duration_ms: Option<f64>,
    pub seekable: bool,
    pub volume: u8,
    pub muted: bool,
    pub repeat: RepeatMode,
    pub shuffled: bool,
    pub tracks: Vec<QueueTrack>,
    pub current: Option<String>,
    pub has_previous: bool,
    pub has_next: bool,
    pub failure: Option<PlayerFault>,
    // Omit this field for local playback and for sessions without a live source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub live: Option<LiveRadio>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LiveState {
    Connecting,
    Buffering,
    Playing,
    Reconnecting,
    Paused,
    Offline,
    Unsupported,
    Rejected,
    OutputUnavailable,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LiveRadio {
    pub station: Station,
    pub status: LiveState,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum PlayerIntent {
    Play {
        id: String,
    },
    Pause,
    Resume,
    Stop,
    Next,
    Previous,
    Seek {
        generation: String,
        milliseconds: f64,
    },
    Volume {
        value: u8,
    },
    Mute {
        enabled: bool,
    },
    Repeat {
        mode: RepeatMode,
    },
    Shuffle {
        enabled: bool,
    },
    Remove {
        id: String,
    },
    MoveBefore {
        id: String,
        before: Option<String>,
    },
    ClearUpcoming,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PlayerFault {
    Opening,
    Unavailable,
    Busy,
    Invalid,
    FileUnavailable,
    DecodeFailed,
    OutputUnavailable,
    PickerUnavailable,
}

/// Radio station presentation. Network source URLs stay with the Rust store.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Station {
    pub id: String,
    pub name: String,
    pub country: String,
    pub language: String,
    pub tags: String,
    pub codec: String,
    pub bitrate: u32,
    pub reported_online: Option<bool>,
    pub manual: bool,
    pub favorite: bool,
    pub artwork: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RadioSort {
    #[default]
    Popular,
    Name,
    Recent,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StationQuery {
    pub name: String,
    pub country: String,
    pub language: String,
    pub tag: String,
    pub sort: RadioSort,
    pub offset: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StationPage {
    pub stations: Vec<Station>,
    pub next: Option<u32>,
    pub stale: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StationShelf {
    Favorites,
    Recent,
    Manual,
}

/// Station edits keep the current playback source immutable.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StationEdit {
    pub id: Option<String>,
    pub name: String,
    pub url: String,
    pub country: String,
    pub language: String,
    pub tags: String,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum RadioCollectionAct {
    Save { station: StationEdit },
    Remove { id: String },
    Favorite { id: String, enabled: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RadioFault {
    Unavailable,
    DirectoryUnavailable,
    Invalid,
    InvalidSource,
    NotFound,
    Busy,
    Cancelled,
    Storage,
    PlaybackDisabled,
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PickAction {
    Listen,
    Append,
    Next,
}

// ---------------------------------------------------------------------------
// Library contract. Identifiers are SQLite row ids narrowed to u32 so they are
// plain JSON numbers. The renderer never supplies a path: it names rows and
// Rust resolves them against its own index.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LibraryFault {
    Opening,
    Unavailable,
    Busy,
    Invalid,
    NotFound,
    AlreadyAdded,
    Overlapping,
    UnsupportedLocation,
    PickerUnavailable,
    NothingPlayable,
    StorageFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ScanPhase {
    Queued,
    Discovering,
    Reading,
    Finalizing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ScanStatus {
    Complete,
    Partial,
    Cancelled,
    Failed,
    Interrupted,
    Offline,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRootView {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub online: bool,
    pub tracks: u32,
    pub unavailable_tracks: u32,
    /// Unix seconds of the last complete scan.
    pub last_scan_at: Option<f64>,
    pub last_scan: Option<ScanReport>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub status: ScanStatus,
    pub finished_at: Option<f64>,
    pub discovered: u32,
    pub indexed: u32,
    pub unchanged: u32,
    pub skipped: u32,
    pub failed: u32,
    pub missing: u32,
}

/// Live scan counters. `total_known` becomes true once discovery has
/// finished; until then the renderer shows a count of files found, not a
/// percentage.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScanView {
    pub root_id: u32,
    pub phase: ScanPhase,
    pub discovered: u32,
    pub processed: u32,
    pub indexed: u32,
    pub unchanged: u32,
    pub skipped: u32,
    pub failed: u32,
    pub total_known: bool,
    pub queued: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySummary {
    /// Increases whenever indexed data or scan state changes. Query caches
    /// key on it so pages refresh without polling.
    pub revision: String,
    /// Increases when playlists, favorites or history change.
    pub user_revision: String,
    pub roots: Vec<LibraryRootView>,
    pub tracks: u32,
    pub albums: u32,
    pub artists: u32,
    pub unavailable_tracks: u32,
    pub scan: Option<ScanView>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TrackRow {
    pub id: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: Option<u32>,
    pub artist_id: Option<u32>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<u32>,
    pub duration_ms: u32,
    pub available: bool,
    pub artwork_id: Option<u32>,
    pub codec: String,
    pub favorite: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AlbumRow {
    pub id: u32,
    pub title: String,
    pub artist: String,
    pub artist_id: Option<u32>,
    pub year: Option<u32>,
    pub artwork_id: Option<u32>,
    pub tracks: u32,
    pub available_tracks: u32,
    pub duration_ms: f64,
    pub favorite: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArtistRow {
    pub id: u32,
    pub name: String,
    pub albums: u32,
    pub tracks: u32,
    pub artwork_id: Option<u32>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TrackPage {
    pub total: u32,
    pub offset: u32,
    pub rows: Vec<TrackRow>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AlbumPage {
    pub total: u32,
    pub offset: u32,
    pub rows: Vec<AlbumRow>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArtistPage {
    pub total: u32,
    pub offset: u32,
    pub rows: Vec<ArtistRow>,
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TrackSort {
    Title,
    Artist,
    Album,
    Recent,
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AlbumSort {
    Title,
    Artist,
    Year,
    Recent,
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrackQuery {
    pub sort: TrackSort,
    pub album_id: Option<u32>,
    pub artist_id: Option<u32>,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AlbumQuery {
    pub sort: AlbumSort,
    pub artist_id: Option<u32>,
    /// With an artist, restricts to albums they lead or only appear on.
    pub role: Option<ArtistRole>,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ArtistRole {
    /// Album artist.
    Main,
    /// Contributing artist on someone else's album.
    Appears,
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtistQuery {
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub tracks: Vec<TrackRow>,
    pub albums: Vec<AlbumRow>,
    pub artists: Vec<ArtistRow>,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PlayTarget {
    Track { id: u32 },
    Album { id: u32, start_track: Option<u32> },
    Artist { id: u32 },
    Playlist { id: u32, start_entry: Option<u32> },
    Favorites { start_source: Option<u32> },
    Tracks { ids: Vec<u32> },
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum QueueMode {
    Replace,
    Append,
    Next,
}

// ---------------------------------------------------------------------------
// User data. A source id is durable: it outlives the index row it came from.
// ---------------------------------------------------------------------------

/// A track as user data sees it. `track_id` is null for a tombstone, whose
/// text comes from the snapshot taken before its index row was removed.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaRow {
    pub source_id: u32,
    pub track_id: Option<u32>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_id: Option<u32>,
    pub artwork_id: Option<u32>,
    pub duration_ms: u32,
    pub available: bool,
    pub favorite: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistCard {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub tracks: u32,
    pub unavailable: u32,
    pub duration_ms: f64,
    /// Up to four distinct covers in playlist order.
    pub artwork_ids: Vec<u32>,
    pub updated_at: f64,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntry {
    pub entry_id: u32,
    pub media: MediaRow,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistPage {
    pub total: u32,
    pub offset: u32,
    pub rows: Vec<PlaylistEntry>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaPage {
    pub total: u32,
    pub offset: u32,
    pub rows: Vec<MediaRow>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteAlbum {
    pub favorite_id: u32,
    pub title: String,
    pub artist: String,
    pub artwork_id: Option<u32>,
    /// Null while no indexed album has this identity.
    pub album: Option<AlbumRow>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRow {
    pub play_id: u32,
    pub started_at: f64,
    pub listened_ms: f64,
    pub media: MediaRow,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub total: u32,
    pub offset: u32,
    pub rows: Vec<HistoryRow>,
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum HistoryScope {
    LastHour,
    LastDay,
    All,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HomeView {
    pub recent_albums: Vec<AlbumRow>,
    pub added_albums: Vec<AlbumRow>,
    pub favorite_albums: Vec<FavoriteAlbum>,
    pub recent_tracks: Vec<HistoryRow>,
    pub frequent: Vec<MediaRow>,
    pub playlists: Vec<PlaylistCard>,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PlaylistAct {
    Create {
        name: String,
        description: String,
        track_ids: Vec<u32>,
    },
    Edit {
        id: u32,
        name: String,
        description: String,
    },
    Delete {
        id: u32,
    },
    AddTracks {
        id: u32,
        track_ids: Vec<u32>,
    },
    AddAlbum {
        id: u32,
        album_id: u32,
    },
    Remove {
        id: u32,
        entry_ids: Vec<u32>,
    },
    Move {
        id: u32,
        entry_id: u32,
        before: Option<u32>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistOutcome {
    pub id: u32,
    /// Entries added, removed or moved.
    pub changed: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum FavoriteTarget {
    Track { id: u32 },
    Source { id: u32 },
    Album { id: u32 },
    FavoriteAlbum { id: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ArtworkSize {
    Thumb,
    Large,
}

/// `#rrggbb` colors with enforced contrast: the interface's light text on
/// `ink` and `glow`, and `accent` on the dark workspace and on `ink`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    pub ink: String,
    pub glow: String,
    pub accent: String,
}

/// The integrated title bar's view of its window. Hover and press describe the
/// maximize button, which a native window covers so Snap Layouts can work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WindowView {
    pub maximized: bool,
    /// Nothing is on screen, so the page stops interpolating.
    pub minimized: bool,
    pub maximize_hovered: bool,
    pub maximize_pressed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mini: Option<bool>,
}

/// Everything the title bar may ask of its own window. A bare string: serde
/// would ignore stray fields on the unit variants of a tagged enum.
#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum WindowIntent {
    Minimize,
    ToggleMaximize,
    Close,
    EnterMini,
    ReturnFromMini,
}

/// The drawn maximize button, in CSS pixels from the top-right corner.
#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptionBounds {
    pub right: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum WindowFault {
    Unavailable,
    Invalid,
}

/// Generates the complete bootstrap declaration file in dependency order.
pub fn typescript() -> String {
    let config = Config::default();
    let mut output = "// Generated by ECHO's gen-contracts binary.\n".to_owned();
    for declaration in [
        LibraryState::decl(&config),
        AppStatus::decl(&config),
        PlayerState::decl(&config),
        RepeatMode::decl(&config),
        QueueTrack::decl(&config),
        PlayerView::decl(&config),
        LiveState::decl(&config),
        LiveRadio::decl(&config),
        Station::decl(&config),
        RadioSort::decl(&config),
        StationQuery::decl(&config),
        StationPage::decl(&config),
        StationShelf::decl(&config),
        StationEdit::decl(&config),
        RadioCollectionAct::decl(&config),
        RadioFault::decl(&config),
        PlayerIntent::decl(&config),
        PlayerFault::decl(&config),
        PickAction::decl(&config),
        LibraryFault::decl(&config),
        ScanPhase::decl(&config),
        ScanStatus::decl(&config),
        ScanReport::decl(&config),
        ScanView::decl(&config),
        LibraryRootView::decl(&config),
        LibrarySummary::decl(&config),
        TrackRow::decl(&config),
        AlbumRow::decl(&config),
        ArtistRow::decl(&config),
        TrackPage::decl(&config),
        AlbumPage::decl(&config),
        ArtistPage::decl(&config),
        TrackSort::decl(&config),
        AlbumSort::decl(&config),
        TrackQuery::decl(&config),
        AlbumQuery::decl(&config),
        ArtistQuery::decl(&config),
        SearchResults::decl(&config),
        PlayTarget::decl(&config),
        QueueMode::decl(&config),
        ArtistRole::decl(&config),
        MediaRow::decl(&config),
        PlaylistCard::decl(&config),
        PlaylistEntry::decl(&config),
        PlaylistPage::decl(&config),
        MediaPage::decl(&config),
        FavoriteAlbum::decl(&config),
        HistoryRow::decl(&config),
        HistoryPage::decl(&config),
        HistoryScope::decl(&config),
        HomeView::decl(&config),
        PlaylistAct::decl(&config),
        PlaylistOutcome::decl(&config),
        FavoriteTarget::decl(&config),
        ArtworkSize::decl(&config),
        Palette::decl(&config),
        WindowView::decl(&config),
        WindowIntent::decl(&config),
        CaptionBounds::decl(&config),
        WindowFault::decl(&config),
    ] {
        output.push_str("export ");
        output.push_str(&declaration);
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serializes_to_renderer_contract() {
        for (library, expected) in [
            (LibraryState::Opening, "opening"),
            (LibraryState::Empty, "empty"),
            (LibraryState::Unavailable, "unavailable"),
        ] {
            assert_eq!(
                serde_json::to_value(AppStatus { library }).unwrap(),
                serde_json::json!({"library": expected})
            );
        }
    }

    #[test]
    fn rejects_renderer_paths_and_unknown_intents() {
        for payload in [
            serde_json::json!({"kind":"play","id":"1","path":"C:/track.wav"}),
            serde_json::json!({"kind":"loadfile","path":"https://example.test/"}),
            serde_json::json!({"kind":"seek","milliseconds":600}),
        ] {
            assert!(serde_json::from_value::<PlayerIntent>(payload).is_err());
        }
        assert!(matches!(
            serde_json::from_value::<PlayerIntent>(
                serde_json::json!({"kind":"seek","generation":"9007199254740994","milliseconds":600})
            ),
            Ok(PlayerIntent::Seek { .. })
        ));
    }

    #[test]
    fn library_requests_use_ids_never_paths_and_reject_unknown_fields() {
        assert!(matches!(
            serde_json::from_value::<PlayTarget>(
                serde_json::json!({"kind":"album","id":4,"startTrack":9})
            ),
            Ok(PlayTarget::Album {
                id: 4,
                start_track: Some(9)
            })
        ));
        for payload in [
            serde_json::json!({"kind":"track","id":1,"path":"C:/a.mp3"}),
            serde_json::json!({"kind":"album","id":1,"start_track":2}),
            serde_json::json!({"kind":"folder","path":"C:/Music"}),
        ] {
            assert!(serde_json::from_value::<PlayTarget>(payload).is_err());
        }
        assert!(
            serde_json::from_value::<TrackQuery>(serde_json::json!({
                "sort":"title","albumId":null,"artistId":null,"offset":0,"limit":50,"where":"1=1"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<TrackQuery>(serde_json::json!({
                "sort":"title","albumId":3,"artistId":null,"offset":0,"limit":50
            }))
            .is_ok()
        );
    }

    #[test]
    fn window_requests_name_only_fixed_actions_and_one_rectangle() {
        for (payload, accepted) in [
            (serde_json::json!("minimize"), true),
            (serde_json::json!("toggleMaximize"), true),
            (serde_json::json!("close"), true),
            (serde_json::json!("enterMini"), true),
            (serde_json::json!("returnFromMini"), true),
            (serde_json::json!("setSize"), false),
            (serde_json::json!({"close":{"label":"other"}}), false),
            (serde_json::json!({"kind":"close"}), false),
        ] {
            assert_eq!(
                serde_json::from_value::<WindowIntent>(payload).is_ok(),
                accepted
            );
        }
        let bounds = serde_json::json!({"right":46,"top":0,"width":46,"height":34});
        assert!(serde_json::from_value::<CaptionBounds>(bounds).is_ok());
        assert!(
            serde_json::from_value::<CaptionBounds>(
                serde_json::json!({"right":46,"top":0,"width":46,"height":34,"hwnd":1})
            )
            .is_err()
        );
    }

    #[test]
    fn main_window_has_only_scoped_commands() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(capability["windows"], serde_json::json!(["main"]));
        assert_eq!(
            capability["permissions"],
            serde_json::json!([
                "allow-app-get-status",
                "allow-player-snapshot",
                "allow-player-subscribe",
                "allow-player-act",
                "allow-player-pick",
                "allow-library-summary",
                "allow-library-subscribe",
                "allow-library-add-root",
                "allow-library-remove-root",
                "allow-library-scan",
                "allow-library-cancel-scan",
                "allow-library-tracks",
                "allow-library-albums",
                "allow-library-artists",
                "allow-library-search",
                "allow-library-artwork",
                "allow-library-play",
                "allow-library-track",
                "allow-library-album",
                "allow-library-artist",
                "allow-library-home",
                "allow-library-palette",
                "allow-playlists-list",
                "allow-playlist-get",
                "allow-playlist-entries",
                "allow-playlist-act",
                "allow-favorite-tracks",
                "allow-favorite-albums",
                "allow-favorite-act",
                "allow-history-page",
                "allow-history-clear",
                "allow-radio-search",
                "allow-radio-cancel",
                "allow-radio-collection",
                "allow-radio-station",
                "allow-radio-edit",
                "allow-radio-collection-act",
                "allow-radio-preferences",
                "allow-radio-artwork",
                "allow-radio-playback-available",
                "allow-radio-play",
                "allow-window-subscribe",
                "allow-window-act",
                "allow-window-caption"
            ])
        );
        assert!(capability.get("remote").is_none());
        let security = &config["app"]["security"];
        assert_eq!(security["capabilities"], serde_json::json!(["main-status"]));
        let csp = security["csp"].as_str().unwrap();
        assert!(
            csp.split("; ")
                .any(|rule| rule == "connect-src ipc: http://ipc.localhost")
        );
        assert!(csp.split("; ").any(|rule| rule == "script-src 'self'"));
        // Cached thumbnails reach the page as blob: URLs made from IPC bytes.
        // No remote image host and no data: URLs are allowed.
        assert!(csp.split("; ").any(|rule| rule == "img-src 'self' blob:"));
        assert!(csp.split("; ").any(|rule| rule == "object-src 'none'"));
    }
}
