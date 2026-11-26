import { invoke } from "@tauri-apps/api/core";
import type {
  AlbumPage,
  AlbumQuery,
  AlbumRow,
  ArtistPage,
  ArtistQuery,
  ArtistRow,
  ArtworkSize,
  FavoriteAlbum,
  FavoriteTarget,
  HistoryPage,
  HistoryScope,
  HomeView,
  LibraryFault,
  MediaPage,
  Palette,
  PlaylistAct,
  PlaylistCard,
  PlaylistOutcome,
  PlaylistPage,
  SearchResults,
  TrackPage,
  TrackQuery,
  TrackRow,
} from "../generated/contracts";

const faults: Record<LibraryFault, string> = {
  opening: "ECHO is still opening your library. Try again in a moment.",
  unavailable: "The music library is unavailable in this session.",
  busy: "ECHO is busy with another request. Try again in a moment.",
  invalid: "That request couldn’t be completed.",
  notFound: "That item is no longer in your library.",
  alreadyAdded: "That folder is already part of your library.",
  overlapping:
    "That folder overlaps one you already added. Remove the other folder first.",
  unsupportedLocation:
    "ECHO can only add folders on local drives, and not through shortcuts or junctions.",
  pickerUnavailable: "Windows couldn’t open the folder picker.",
  nothingPlayable:
    "None of those files can be played right now. Check that the drive is connected.",
  storageFailed: "ECHO couldn’t read or write its library database.",
};

export function describeLibraryFault(error: unknown): string {
  return typeof error === "string" &&
    Object.prototype.hasOwnProperty.call(faults, error)
    ? faults[error as LibraryFault]
    : "ECHO couldn’t reach the library service.";
}

/** Thin wrappers over the scoped commands. Every argument is a row id or a
 * bounded value; the renderer never supplies a filesystem path. */
export const libraryApi = {
  tracks: (query: TrackQuery) => invoke<TrackPage>("library_tracks", { query }),
  albums: (query: AlbumQuery) => invoke<AlbumPage>("library_albums", { query }),
  artists: (query: ArtistQuery) =>
    invoke<ArtistPage>("library_artists", { query }),
  search: (text: string, limit: number) =>
    invoke<SearchResults>("library_search", { text, limit }),
  track: (id: number) => invoke<TrackRow>("library_track", { id }),
  album: (id: number) => invoke<AlbumRow>("library_album", { id }),
  artist: (id: number) => invoke<ArtistRow>("library_artist", { id }),
  home: () => invoke<HomeView>("library_home"),
  artwork: async (id: number, size: ArtworkSize) =>
    new Uint8Array(await invoke<ArrayBuffer>("library_artwork", { id, size })),
  palette: (id: number) => invoke<Palette>("library_palette", { id }),
  playlists: () => invoke<PlaylistCard[]>("playlists_list"),
  playlist: (id: number) => invoke<PlaylistCard>("playlist_get", { id }),
  playlistEntries: (id: number, offset: number, limit: number) =>
    invoke<PlaylistPage>("playlist_entries", { id, offset, limit }),
  playlistAct: (action: PlaylistAct) =>
    invoke<PlaylistOutcome>("playlist_act", { action }),
  favoriteTracks: (offset: number, limit: number) =>
    invoke<MediaPage>("favorite_tracks", { offset, limit }),
  favoriteAlbums: (offset: number, limit: number) =>
    invoke<FavoriteAlbum[]>("favorite_albums", { offset, limit }),
  favorite: (target: FavoriteTarget, on: boolean) =>
    invoke<void>("favorite_act", { target, on }),
  history: (offset: number, limit: number) =>
    invoke<HistoryPage>("history_page", { offset, limit }),
  clearHistory: (scope: HistoryScope) =>
    invoke<number>("history_clear", { scope }),
};
