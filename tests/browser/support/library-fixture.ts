import type { Page } from "@playwright/test";
import type {
  PlayerView,
  RadioCollectionAct,
  RadioSort,
  Station,
  StationEdit,
  StationQuery,
} from "../../../src/generated/contracts";

/**
 * A contract-accurate fake of the desktop commands for browser tests.
 *
 * Library rows are generated on demand from an index, so a 20,000-track
 * library costs no memory and the real renderer must page and virtualize.
 * Playlists, favorites, history and the queue are small mutable state.
 *
 * Fixture names and covers are test data only and never ship.
 */
export interface LibraryFixtureOptions {
  radio?: {
    stations: Station[];
    enabled: boolean;
    searchDelayMs?: number;
    outage?: boolean;
  };
  /** Twelve tracks make an album; artists repeat every seven albums. */
  tracks: number;
  roots?: {
    id: number;
    name: string;
    path: string;
    online: boolean;
    tracks: number;
  }[];
  scan?: {
    rootId: number;
    phase: "discovering" | "reading" | "finalizing";
    discovered: number;
    processed: number;
    totalKnown: boolean;
  } | null;
  /** Track ids at or below this value are unavailable. */
  unavailableUpTo?: number;
  latencyMs?: number;
  /** Plausible names instead of numbered ones, for visual review. */
  editorial?: boolean;
  /** Albums up to this id have no cover. */
  noArt?: boolean;
  playlists?: {
    id: number;
    name: string;
    description?: string;
    trackIds: number[];
    /** Entry positions whose source left the library. */
    lost?: number[];
  }[];
  favoriteTracks?: number[];
  favoriteAlbums?: number[];
  /** Track ids, most recent first. */
  history?: number[];
  /** Track ids in the queue; the first is loaded. */
  queue?: number[];
  player?: Partial<PlayerView>;
  /** Discs per album, for multi-disc layouts. */
  discs?: number;
  longTitles?: boolean;
  /**
   * Review captures only: covers used in album order instead of the drawn
   * motifs, as data URLs or flat `sheet:fill:mark` drawings, with palettes.
   */
  covers?: { image: string; palette: [string, string, string] }[];
}

export async function installLibraryFixture(
  page: Page,
  options: LibraryFixtureOptions,
) {
  await page.addInitScript((config: LibraryFixtureOptions) => {
    const PER_ALBUM = 12;
    const total = config.tracks;
    const albumCount = Math.ceil(total / PER_ALBUM);
    const artistCount = Math.min(7, albumCount);
    const roots = config.roots ?? [
      { id: 1, name: "Music", path: "D:\\Music", online: true, tracks: total },
    ];
    const log: { command: string; args: unknown }[] = [];
    const radioStations = [...(config.radio?.stations ?? [])];
    const radioEdits = new Map<string, StationEdit>();
    const cancelled = new Set<number>();
    let radioSort: RadioSort = "popular";
    let revision = 1;
    let userRevision = 1;
    let scan = config.scan ?? null;
    let sendLibrary: ((value: unknown) => void) | undefined;
    let sendPlayer: ((value: unknown) => void) | undefined;
    let callbacks = 0;
    const channels = new Map<number, (value: unknown) => void>();
    const windowLog = { maximized: false, minimized: false };
    // Tauri channels deliver messages strictly in index order starting at 0.
    let libraryIndex = 0;
    let playerIndex = 0;
    let windowIndex = 0;
    let sendWindow: ((value: unknown) => void) | undefined;
    const windowView = () => ({
      maximized: windowLog.maximized,
      minimized: windowLog.minimized,
      maximizeHovered: false,
      maximizePressed: false,
    });

    const pad = (value: number) => String(value).padStart(5, "0");
    const pick = (list: string[], n: number) => list[n % list.length];
    const heads = [
      "Afterglow",
      "Concrete Hours",
      "Petal Static",
      "Glass Current",
      "Field Notes",
      "Liquid Form",
      "Low Meadow",
      "Paper Weather",
      "Salt Index",
      "North Landing",
    ];
    const songs = [
      "Slow Horizon",
      "Parallel Walls",
      "Soft Interference",
      "Undertow",
      "Quiet Weather",
      "Mercury Pool",
      "Last Light",
      "In the Garden",
      "Blue Refraction",
      "Fluid Memory",
      "Harbour, Before Rain",
    ];
    const people = [
      "Mira Vale",
      "Forma Unit",
      "Iona Bloom",
      "Aven Drift",
      "Ellis Fen",
      "Silver Index",
      "Ostra",
    ];
    const artistName = (n: number) =>
      config.editorial ? pick(people, n) : `Fixture Quartet ${n + 1}`;
    const albumName = (n: number) =>
      config.longTitles && n === 0
        ? "Recordings for an Unfinished Film About the Weather Over a City That Was Moved Twice, Vol. 2"
        : config.editorial
          ? pick(heads, n) +
            (n >= heads.length ? ` ${Math.floor(n / heads.length) + 1}` : "")
          : `Fixture Suite ${pad(n + 1)}`;
    const trackName = (n: number) =>
      config.editorial ? pick(songs, n * 7 + 3) : `Fixture track ${pad(n + 1)}`;
    const albumArtist = (n: number) => n % Math.max(1, artistCount);
    const artOf = (n: number) =>
      config.noArt || (!config.editorial && n % 4 === 3) ? null : n + 1;

    const favoriteTracks = new Set(config.favoriteTracks ?? []);
    const favoriteAlbums = new Set(config.favoriteAlbums ?? []);
    let history = [...(config.history ?? [])];
    let nextEntry = 1;
    const playlists = (config.playlists ?? []).map((playlist) => ({
      ...playlist,
      description: playlist.description ?? "",
      entries: playlist.trackIds.map((track, at) => ({
        id: nextEntry++,
        track,
        lost: playlist.lost?.includes(at) ?? false,
      })),
    }));
    let nextPlaylist = 1 + Math.max(0, ...playlists.map(({ id }) => id));

    function track(index: number) {
      const album = Math.floor(index / PER_ALBUM);
      const artist = albumArtist(album);
      const discs = config.discs ?? 1;
      const inAlbum = index % PER_ALBUM;
      const perDisc = PER_ALBUM / discs;
      return {
        id: index + 1,
        title: trackName(index),
        artist: artistName(artist),
        album: albumName(album),
        albumId: album + 1,
        artistId: artist + 1,
        trackNo: (inAlbum % perDisc) + 1,
        discNo: Math.floor(inAlbum / perDisc) + 1,
        year: 1990 + (album % 30),
        durationMs: 120_000 + ((index * 37) % 190) * 1000,
        available: index + 1 > (config.unavailableUpTo ?? 0),
        artworkId: artOf(album),
        codec: "FLAC",
        favorite: favoriteTracks.has(index + 1),
      };
    }
    function album(index: number) {
      const artist = albumArtist(index);
      const tracks = Math.min(PER_ALBUM, total - index * PER_ALBUM);
      return {
        id: index + 1,
        title: albumName(index),
        artist: artistName(artist),
        artistId: artist + 1,
        year: 1990 + (index % 30),
        artworkId: artOf(index),
        tracks,
        availableTracks: tracks,
        durationMs: tracks * 206_000,
        favorite: favoriteAlbums.has(index + 1),
      };
    }
    function artist(index: number) {
      const albums = Math.ceil((albumCount - index) / Math.max(1, artistCount));
      return {
        id: index + 1,
        name: artistName(index),
        albums,
        tracks: albums * PER_ALBUM,
        artworkId: artOf(index),
      };
    }
    function media(id: number, lost = false) {
      const row = track(id - 1);
      return {
        sourceId: id + 100_000,
        trackId: lost ? null : id,
        title: row.title,
        artist: row.artist,
        album: row.album,
        albumId: lost ? null : row.albumId,
        artworkId: row.artworkId,
        durationMs: row.durationMs,
        available: !lost && row.available,
        favorite: favoriteTracks.has(id),
      };
    }
    function card(playlist: (typeof playlists)[number]) {
      const ids = playlist.entries.map(
        ({ track: id }) => track(id - 1).artworkId,
      );
      return {
        id: playlist.id,
        name: playlist.name,
        description: playlist.description,
        tracks: playlist.entries.length,
        unavailable: playlist.entries.filter(({ lost }) => lost).length,
        durationMs: playlist.entries.length * 206_000,
        artworkIds: [...new Set(ids.filter((id) => id != null))].slice(0, 4),
        updatedAt: 1_780_000_000,
      };
    }
    const favoriteAlbum = (id: number) => ({
      favoriteId: id + 500,
      title: albumName(id - 1),
      artist: artistName(albumArtist(id - 1)),
      artworkId: artOf(id - 1),
      album: album(id - 1),
    });
    const historyRow = (id: number, at: number) => ({
      playId: history.length - at,
      startedAt: Math.floor(Date.now() / 1000) - 400 - at * 4000,
      listenedMs: 180_000,
      media: media(id),
    });

    function summary() {
      return {
        revision: String(revision),
        userRevision: String(userRevision),
        roots: roots.map((root) => ({
          ...root,
          unavailableTracks: root.online
            ? (config.unavailableUpTo ?? 0)
            : root.tracks,
          lastScanAt: 1_780_000_000,
          lastScan: {
            status: "complete",
            finishedAt: 1_780_000_000,
            discovered: root.tracks,
            indexed: 0,
            unchanged: root.tracks,
            skipped: 0,
            failed: 0,
            missing: 0,
          },
        })),
        tracks: total,
        albums: albumCount,
        artists: artistCount,
        unavailableTracks: config.unavailableUpTo ?? 0,
        scan: scan
          ? {
              ...scan,
              indexed: 0,
              unchanged: 0,
              skipped: 0,
              failed: 0,
              queued: 0,
            }
          : null,
      };
    }
    function publish() {
      revision += 1;
      sendLibrary?.({ index: libraryIndex++, message: summary() });
    }
    function userChanged() {
      userRevision += 1;
      publish();
    }
    const delay = (value: unknown) =>
      config.latencyMs
        ? new Promise((resolve) =>
            setTimeout(() => resolve(value), config.latencyMs),
          )
        : value;

    let nextQueue = 1;
    const queued = (id: number) => {
      const row = track(id - 1);
      return {
        id: String(nextQueue++),
        title: row.title,
        subtitle: row.artist,
        artworkId: row.artworkId,
        trackId: id,
        unavailable: false,
      };
    };
    const initial = (config.queue ?? []).map(queued);
    let view: PlayerView = {
      revision: "1",
      generation: "1",
      state: initial.length ? "paused" : "idle",
      positionMs: initial.length ? 74_000 : 0,
      durationMs: initial.length ? 231_000 : null,
      seekable: initial.length > 0,
      volume: 72,
      muted: false,
      repeat: "off",
      shuffled: false,
      tracks: initial,
      current: initial[0]?.id ?? null,
      hasPrevious: false,
      hasNext: initial.length > 1,
      failure: null,
      ...config.player,
    } as PlayerView;
    function updatePlayer(changes: Partial<PlayerView>) {
      view = {
        ...view,
        ...changes,
        revision: String(BigInt(view.revision) + 1n),
      };
      const at = view.tracks.findIndex(({ id }) => id === view.current);
      view = {
        ...view,
        hasPrevious: at > 0,
        hasNext: at + 1 < view.tracks.length,
      };
      sendPlayer?.({ index: playerIndex++, message: view });
      return view;
    }
    const start = (current: string | null) =>
      updatePlayer({
        live: undefined,
        current,
        state: "playing",
        positionMs: 0,
        durationMs: 231_000,
        seekable: true,
        generation: String(Number(view.generation) + 1),
      });

    function motif(id: number, size: number) {
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = size;
      const c = canvas.getContext("2d");
      if (!c) return canvas;
      const hues = [348, 32, 22, 196, 74, 218, 284, 158, 8, 44];
      const hue = hues[(id - 1) % hues.length] + Math.floor(id / 10) * 23;
      const u = size / 100;
      const field = c.createLinearGradient(0, 0, 40 * u, 100 * u);
      field.addColorStop(0, `hsl(${hue} 46% 52%)`);
      field.addColorStop(1, `hsl(${hue - 24} 48% 17%)`);
      c.fillStyle = field;
      c.fillRect(0, 0, size, size);
      const kind = id % 5;
      if (kind === 1) {
        c.fillStyle = `hsl(${hue + 18} 92% 68%)`;
        c.beginPath();
        c.arc(50 * u, 42 * u, 25 * u, 0, 7);
        c.fill();
        for (let layer = 0; layer < 4; layer++) {
          c.fillStyle = `hsl(${hue - 20} 38% ${34 - layer * 7}%)`;
          c.beginPath();
          c.moveTo(0, size);
          for (let x = 0; x <= 100; x += 2)
            c.lineTo(
              x * u,
              (58 +
                layer * 10 +
                Math.sin(x / (7 + layer * 3) + layer * 2) * 5 +
                Math.sin(x / 3) * 1.4) *
                u,
            );
          c.lineTo(size, size);
          c.fill();
        }
      } else if (kind === 2) {
        for (let n = 0; n < 9; n++) {
          c.fillStyle = `hsl(${hue} 14% ${n % 2 ? 70 - n * 6 : 16 + n * 3}%)`;
          c.beginPath();
          c.moveTo((8 + n * 9) * u, size);
          c.lineTo((30 + n * 8) * u, (6 + n * 7) * u);
          c.lineTo((46 + n * 8) * u, (6 + n * 7) * u);
          c.lineTo((30 + n * 11) * u, size);
          c.fill();
        }
      } else if (kind === 3) {
        c.fillStyle = "#0d0b0c";
        c.fillRect(0, 0, size, size);
        c.translate(50 * u, 46 * u);
        for (let n = 0; n < 64; n++) {
          c.rotate(2.399);
          c.fillStyle = `hsl(${hue + (n % 18)} 92% ${64 - n * 0.5}% / 0.72)`;
          c.beginPath();
          c.ellipse(
            n * 0.52 * u,
            0,
            (4 + n * 0.16) * u,
            (2 + n * 0.06) * u,
            0,
            0,
            7,
          );
          c.fill();
        }
      } else if (kind === 4) {
        c.lineWidth = 0.6 * u;
        for (let n = 0; n < 44; n++) {
          c.strokeStyle = `hsl(${hue} 72% ${48 + (n % 9) * 5}% / 0.5)`;
          c.beginPath();
          c.moveTo(-5 * u, (n * 2.6 - 6) * u);
          c.bezierCurveTo(
            30 * u,
            (96 - n * 2) * u,
            64 * u,
            (n * 3 - 30) * u,
            105 * u,
            (70 - n) * u,
          );
          c.stroke();
        }
      } else {
        c.fillStyle = `hsl(${hue - 30} 40% 14% / 0.75)`;
        for (let n = 0; n < 130; n++) {
          const x = (n * 61) % 100;
          c.fillRect(x * u, (64 + ((n * 17) % 30)) * u, 0.5 * u, 26 * u);
        }
        c.fillStyle = `hsl(${hue + 30} 60% 78%)`;
        c.beginPath();
        c.arc(70 * u, 26 * u, 8 * u, 0, 7);
        c.fill();
      }
      return canvas;
    }
    const reviewCover = (id: number) =>
      config.covers?.[(id - 1) % config.covers.length];
    function flat(spec: string, size: number) {
      const [, fill, mark] = spec.split(":");
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = size;
      const c = canvas.getContext("2d");
      if (!c) return canvas;
      c.fillStyle = fill;
      c.fillRect(0, 0, size, size);
      c.strokeStyle = mark;
      c.lineWidth = size / 64;
      c.arc(size / 2, size / 2, size / 4, 0, 7);
      c.stroke();
      return canvas;
    }
    async function jpeg(id: number, size: number) {
      const review = reviewCover(id);
      if (review && !review.image.startsWith("sheet:"))
        return (await fetch(review.image)).arrayBuffer();
      const blob = await new Promise<Blob | null>((resolve) =>
        (review ? flat(review.image, size) : motif(id, size)).toBlob(
          resolve,
          "image/jpeg",
          0.88,
        ),
      );
      return blob ? blob.arrayBuffer() : new ArrayBuffer(0);
    }
    // Representative outputs of the Rust extractor for the motif hues.
    function palette(id: number) {
      const review = reviewCover(id);
      if (review) {
        const [ink, glow, accent] = review.palette;
        return { ink, glow, accent };
      }
      const table = [
        ["#3b1a23", "#8c3347", "#f0a8b5"],
        ["#33241a", "#6d5233", "#e6c9a4"],
        ["#35190c", "#8a3f12", "#f2b07d"],
        ["#10303a", "#1f6273", "#93d6e6"],
        ["#272b10", "#59621d", "#d2dc93"],
        ["#1a2638", "#35527e", "#a9c4ee"],
        ["#2c1c38", "#633b7c", "#d6b4ea"],
        ["#14302a", "#2a6657", "#97d9c8"],
        ["#3a1b15", "#8b3a2b", "#f1ad9f"],
        ["#312614", "#6d552a", "#e3cb92"],
      ];
      const [ink, glow, accent] = table[(id - 1) % table.length];
      return { ink, glow, accent };
    }

    function subset(query: {
      albumId: number | null;
      artistId: number | null;
    }) {
      if (query.albumId != null) {
        const from = (query.albumId - 1) * PER_ALBUM;
        return Array.from(
          { length: Math.max(0, Math.min(PER_ALBUM, total - from)) },
          (_, offset) => from + offset,
        );
      }
      if (query.artistId != null) {
        const out: number[] = [];
        for (
          let a = query.artistId - 1;
          a < albumCount;
          a += Math.max(1, artistCount)
        )
          for (let t = 0; t < PER_ALBUM; t++)
            if (a * PER_ALBUM + t < total) out.push(a * PER_ALBUM + t);
        return out;
      }
      return null;
    }
    const found = <T>(value: T | undefined | null | false): T => {
      if (value == null || value === false) throw "notFound";
      return value;
    };
    function playTracks(target: {
      kind: string;
      id?: number;
      ids?: number[];
      startTrack?: number | null;
      startEntry?: number | null;
      startSource?: number | null;
    }): { ids: number[]; at: number } {
      switch (target.kind) {
        case "track":
          return { ids: [target.id as number], at: 0 };
        case "tracks":
          return { ids: target.ids as number[], at: 0 };
        case "album": {
          const ids = subset({
            albumId: target.id as number,
            artistId: null,
          })!.map((n) => n + 1);
          return { ids, at: Math.max(0, ids.indexOf(target.startTrack ?? -1)) };
        }
        case "artist":
          return {
            ids: subset({ albumId: null, artistId: target.id as number })!.map(
              (n) => n + 1,
            ),
            at: 0,
          };
        case "playlist": {
          const entries = found(
            playlists.find(({ id }) => id === target.id),
          ).entries.filter(({ lost }) => !lost);
          return {
            ids: entries.map((entry) => entry.track),
            at: Math.max(
              0,
              entries.findIndex(({ id }) => id === target.startEntry),
            ),
          };
        }
        default:
          return { ids: [...favoriteTracks], at: 0 };
      }
    }

    Object.assign(window, {
      testLibraryLog: log,
      testLibrary: {
        setScan(next: typeof scan) {
          scan = next;
          publish();
        },
        updatePlayer,
        breakPlayer() {
          sendPlayer?.({
            index: playerIndex++,
            message: {
              ...view,
              revision: String(BigInt(view.revision) + 1n),
              tracks: [null],
              current: "1",
            },
          });
        },
        minimize(next: boolean) {
          windowLog.minimized = next;
          sendWindow?.({ index: windowIndex++, message: windowView() });
        },
        advance() {
          const at = view.tracks.findIndex(({ id }) => id === view.current);
          start(view.tracks[at + 1]?.id ?? null);
        },
      },
      isTauri: true,
      __TAURI_INTERNALS__: {
        transformCallback: (callback: (value: unknown) => void) => {
          callbacks += 1;
          channels.set(callbacks, callback);
          return callbacks;
        },
        unregisterCallback: () => undefined,
        invoke: async (command: string, args: Record<string, unknown> = {}) => {
          log.push({ command, args });
          switch (command) {
            case "radio_playback_available":
              return config.radio?.enabled ?? false;
            case "radio_preferences":
              if (args.sort) radioSort = args.sort as RadioSort;
              return radioSort;
            case "radio_search": {
              const query = args.query as StationQuery;
              const request = Number(args.request);
              if (config.radio?.searchDelayMs)
                await new Promise((resolve) =>
                  setTimeout(resolve, config.radio?.searchDelayMs),
                );
              if (cancelled.delete(request)) throw "cancelled";
              if (config.radio?.outage) throw "directoryUnavailable";
              const matches = radioStations.filter(
                (row) =>
                  !row.manual &&
                  row.name.toLowerCase().includes(query.name.toLowerCase()) &&
                  (!query.country || query.country === row.country) &&
                  row.language
                    .toLowerCase()
                    .includes(query.language.toLowerCase()) &&
                  row.tags.toLowerCase().includes(query.tag.toLowerCase()),
              );
              return {
                stations: matches.slice(query.offset, query.offset + 24),
                next: null,
                stale: false,
              };
            }
            case "radio_cancel":
              cancelled.add(Number(args.request));
              return;
            case "radio_collection":
              return {
                stations: radioStations.filter((station) =>
                  args.shelf === "favorites"
                    ? station.favorite
                    : args.shelf === "manual"
                      ? station.manual
                      : false,
                ),
                next: null,
                stale: false,
              };
            case "radio_station":
              return found(
                radioStations.find((station) => station.id === args.id),
              );
            case "radio_edit":
              return found(radioEdits.get(String(args.id)));
            case "radio_artwork": {
              const image = new Uint8Array(
                await jpeg(config.covers ? 1 : 4, 320),
              );
              const colors =
                config.covers?.[0]?.palette.join("") ?? "#18182b#373451#f1c4df";
              const bytes = new Uint8Array(21 + image.length);
              bytes.set(new TextEncoder().encode(colors));
              bytes.set(image, 21);
              return bytes.buffer;
            }
            case "radio_collection_act": {
              const action = args.action as RadioCollectionAct;
              if (action.kind === "save") {
                const edit = action.station;
                if (edit.url.includes("127.0.0.1")) throw "invalidSource";
                const id = edit.id ?? "manual:1";
                const index = radioStations.findIndex((row) => row.id === id);
                const station = {
                  id,
                  name: edit.name,
                  country: edit.country,
                  language: edit.language,
                  tags: edit.tags,
                  manual: true,
                  favorite: radioStations[index]?.favorite ?? false,
                  artwork: false,
                  codec: "",
                  bitrate: 0,
                  reportedOnline: null,
                };
                radioEdits.set(id, { ...edit, id });
                if (index < 0) radioStations.push(station);
                else radioStations[index] = station;
                return station;
              }
              const index = radioStations.findIndex(
                (station) => station.id === action.id,
              );
              if (index < 0) throw "notFound";
              if (action.kind === "favorite") {
                radioStations[index] = {
                  ...radioStations[index],
                  favorite: action.enabled,
                };
                return radioStations[index];
              }
              radioEdits.delete(action.id);
              radioStations.splice(index, 1);
              return null;
            }
            case "radio_play": {
              if (!config.radio?.enabled) throw "playbackDisabled";
              const station = found(
                radioStations.find((row) => row.id === args.id),
              );
              return updatePlayer({
                live: {
                  station,
                  status: "playing",
                  title: "Night Shift — Soft Signal",
                },
                state: "playing",
                current: null,
                positionMs: 0,
                durationMs: null,
                seekable: false,
              });
            }
            case "app_get_status":
              return { library: "empty" };
            case "window_subscribe":
              sendWindow = channels.get((args.channel as { id: number }).id);
              return windowView();
            case "window_act":
              if (args.intent === "toggleMaximize") {
                windowLog.maximized = !windowLog.maximized;
                sendWindow?.({ index: windowIndex++, message: windowView() });
              }
              return undefined;
            case "window_caption":
              return undefined;
            case "player_subscribe":
              sendPlayer = channels.get((args.channel as { id: number }).id);
              return view;
            case "player_snapshot":
              return view;
            case "player_act": {
              const intent = args.intent as Record<string, unknown>;
              const id = intent.id as string;
              switch (intent.kind) {
                case "pause":
                  return updatePlayer({
                    state: "paused",
                    live: view.live
                      ? { ...view.live, status: "paused", title: null }
                      : undefined,
                  });
                case "resume":
                  return updatePlayer({
                    state: "playing",
                    live: view.live
                      ? { ...view.live, status: "playing" }
                      : undefined,
                  });
                case "play":
                  return start(id);
                case "next":
                case "previous": {
                  const at = view.tracks.findIndex(
                    (row) => row.id === view.current,
                  );
                  return start(
                    view.tracks[at + (intent.kind === "next" ? 1 : -1)].id,
                  );
                }
                case "shuffle":
                  return updatePlayer({ shuffled: intent.enabled as boolean });
                case "repeat":
                  return updatePlayer({
                    repeat: intent.mode as PlayerView["repeat"],
                  });
                case "mute":
                  return updatePlayer({ muted: intent.enabled as boolean });
                case "volume":
                  return updatePlayer({ volume: intent.value as number });
                case "seek":
                  return updatePlayer({
                    positionMs: intent.milliseconds as number,
                  });
                case "stop":
                  return updatePlayer({
                    state: "idle",
                    positionMs: 0,
                    live: undefined,
                  });
                case "remove":
                  if (
                    id === view.current ||
                    !view.tracks.some((row) => row.id === id)
                  )
                    throw "invalid";
                  return updatePlayer({
                    tracks: view.tracks.filter((row) => row.id !== id),
                  });
                case "clearUpcoming": {
                  const at = view.tracks.findIndex(
                    (row) => row.id === view.current,
                  );
                  return updatePlayer({ tracks: view.tracks.slice(0, at + 1) });
                }
                case "moveBefore": {
                  const first =
                    view.tracks.findIndex((row) => row.id === view.current) + 1;
                  const before = intent.before as string | null;
                  const from = view.tracks.findIndex((row) => row.id === id);
                  const target =
                    before == null
                      ? view.tracks.length
                      : view.tracks.findIndex((row) => row.id === before);
                  // Like Rust: played or vanished rows cannot be moved or targeted.
                  if (from < first || target < first) throw "invalid";
                  const rows = [...view.tracks];
                  const [moved] = rows.splice(from, 1);
                  rows.splice(target > from ? target - 1 : target, 0, moved);
                  return updatePlayer({ tracks: rows });
                }
                default:
                  return view;
              }
            }
            case "player_pick":
              return view;
            case "library_subscribe":
              sendLibrary = channels.get((args.channel as { id: number }).id);
              return summary();
            case "library_summary":
              return summary();
            case "library_tracks": {
              const query = args.query as {
                sort: string;
                albumId: number | null;
                artistId: number | null;
                offset: number;
                limit: number;
              };
              const some = subset(query);
              const count = some ? some.length : total;
              const rows = [];
              for (
                let n = query.offset;
                n < Math.min(count, query.offset + query.limit);
                n++
              )
                rows.push(
                  track(
                    some
                      ? some[n]
                      : query.sort === "recent"
                        ? total - 1 - n
                        : n,
                  ),
                );
              return delay({ total: count, offset: query.offset, rows });
            }
            case "library_albums": {
              const query = args.query as {
                sort: string;
                artistId: number | null;
                role: string | null;
                offset: number;
                limit: number;
              };
              const ids: number[] = [];
              for (let a = 0; a < albumCount; a++) {
                const mine =
                  query.artistId == null ||
                  albumArtist(a) === query.artistId - 1;
                if (query.role === "appears" ? a === query.artistId : mine)
                  ids.push(a);
              }
              if (query.sort === "recent") ids.reverse();
              return delay({
                total: ids.length,
                offset: query.offset,
                rows: ids
                  .slice(query.offset, query.offset + query.limit)
                  .map(album),
              });
            }
            case "library_artists": {
              const query = args.query as { offset: number; limit: number };
              const rows = [];
              for (
                let n = query.offset;
                n < Math.min(artistCount, query.offset + query.limit);
                n++
              )
                rows.push(artist(n));
              return delay({ total: artistCount, offset: query.offset, rows });
            }
            case "library_track":
              return found(
                (args.id as number) <= total && track((args.id as number) - 1),
              );
            case "library_album":
              return found(
                (args.id as number) <= albumCount &&
                  album((args.id as number) - 1),
              );
            case "library_artist":
              return found(
                (args.id as number) <= artistCount &&
                  artist((args.id as number) - 1),
              );
            case "library_home": {
              const seen = [
                ...new Set(
                  history.map((id) => Math.floor((id - 1) / PER_ALBUM)),
                ),
              ];
              return {
                recentAlbums: seen.slice(0, 10).map(album),
                addedAlbums: Array.from(
                  { length: Math.min(10, albumCount) },
                  (_, n) => album(albumCount - 1 - n),
                ),
                favoriteAlbums: [...favoriteAlbums].map(favoriteAlbum),
                recentTracks: history.slice(0, 6).map(historyRow),
                frequent: [],
                playlists: playlists.slice(0, 6).map(card),
              };
            }
            case "library_search": {
              const words = String(args.text)
                .toLowerCase()
                .trim()
                .split(/\s+/)
                .filter(Boolean);
              const limit = Number(args.limit);
              const matches = (name: string) => {
                const tokens = name.toLowerCase().split(/\s+/);
                return (
                  words.length > 0 &&
                  words.every((word) =>
                    tokens.some((token) => token.startsWith(word)),
                  )
                );
              };
              const tracks = [];
              const albums = [];
              const artists = [];
              for (let i = 0; i < total && tracks.length < limit; i++) {
                const row = track(i);
                if (matches(`${row.title} ${row.artist} ${row.album}`))
                  tracks.push(row);
              }
              for (let a = 0; a < albumCount && albums.length < limit; a++)
                if (matches(`${albumName(a)} ${artistName(albumArtist(a))}`))
                  albums.push(album(a));
              for (let r = 0; r < artistCount && artists.length < limit; r++)
                if (matches(artistName(r))) artists.push(artist(r));
              return delay({ tracks, albums, artists });
            }
            case "library_artwork":
              return jpeg(Number(args.id), args.size === "large" ? 640 : 160);
            case "library_palette":
              return palette(Number(args.id));
            case "library_play": {
              const { ids, at } = playTracks(args.target as never);
              if (ids.length === 0) throw "nothingPlayable";
              const rows = ids.slice(0, 1024).map(queued);
              if (args.mode === "replace") {
                view = { ...view, tracks: rows };
                return start(rows[at].id);
              }
              const index = view.tracks.findIndex(
                (row) => row.id === view.current,
              );
              const list = [...view.tracks];
              list.splice(
                args.mode === "next" ? index + 1 : list.length,
                0,
                ...rows,
              );
              return updatePlayer({ tracks: list });
            }
            case "playlists_list":
              return playlists.map(card);
            case "playlist_get":
              return found(
                playlists.map(card).find(({ id }) => id === args.id),
              );
            case "playlist_entries": {
              const { entries } = found(
                playlists.find(({ id }) => id === args.id),
              );
              const from = args.offset as number;
              return {
                total: entries.length,
                offset: from,
                rows: entries
                  .slice(from, from + (args.limit as number))
                  .map((entry) => ({
                    entryId: entry.id,
                    media: media(entry.track, entry.lost),
                  })),
              };
            }
            case "playlist_act": {
              const action = args.action as Record<string, unknown>;
              const add = (list: (typeof playlists)[number], ids: number[]) =>
                ids.forEach((id) =>
                  list.entries.push({
                    id: nextEntry++,
                    track: id,
                    lost: false,
                  }),
                );
              if (action.kind === "create") {
                const created = {
                  id: nextPlaylist++,
                  name: String(action.name).trim(),
                  description: String(action.description),
                  trackIds: [],
                  entries: [] as (typeof playlists)[number]["entries"],
                };
                add(created as never, action.trackIds as number[]);
                playlists.unshift(created as never);
                userChanged();
                return { id: created.id, changed: created.entries.length };
              }
              const list = found(playlists.find(({ id }) => id === action.id));
              let changed = 1;
              if (action.kind === "edit") {
                list.name = String(action.name).trim();
                list.description = String(action.description);
              } else if (action.kind === "delete") {
                playlists.splice(playlists.indexOf(list), 1);
              } else if (action.kind === "addTracks") {
                add(list, action.trackIds as number[]);
              } else if (action.kind === "addAlbum") {
                const ids = subset({
                  albumId: action.albumId as number,
                  artistId: null,
                })!;
                add(
                  list,
                  ids.map((n) => n + 1),
                );
                changed = ids.length;
              } else if (action.kind === "remove") {
                const gone = action.entryIds as number[];
                list.entries = list.entries.filter(
                  ({ id }) => !gone.includes(id),
                );
              } else if (action.kind === "move") {
                const from = list.entries.findIndex(
                  ({ id }) => id === action.entryId,
                );
                found(from >= 0);
                const [moved] = list.entries.splice(from, 1);
                const at =
                  action.before == null
                    ? list.entries.length
                    : list.entries.findIndex(({ id }) => id === action.before);
                found(at >= 0);
                list.entries.splice(at, 0, moved);
              }
              userChanged();
              return { id: list.id, changed };
            }
            case "favorite_tracks":
              return {
                total: favoriteTracks.size,
                offset: args.offset,
                rows: [...favoriteTracks]
                  .slice(args.offset as number)
                  .map((id) => media(id)),
              };
            case "favorite_albums":
              return [...favoriteAlbums].map(favoriteAlbum);
            case "favorite_act": {
              const target = args.target as { kind: string; id: number };
              const set = target.kind.toLowerCase().includes("album")
                ? favoriteAlbums
                : favoriteTracks;
              const id =
                target.kind === "source"
                  ? target.id - 100_000
                  : target.kind === "favoriteAlbum"
                    ? target.id - 500
                    : target.id;
              if (args.on) set.add(id);
              else set.delete(id);
              userChanged();
              return undefined;
            }
            case "history_page":
              return {
                total: history.length,
                offset: args.offset,
                rows: history
                  .slice(
                    args.offset as number,
                    (args.offset as number) + (args.limit as number),
                  )
                  .map((id, n) => historyRow(id, n + (args.offset as number))),
              };
            case "history_clear": {
              const removed = history.length;
              history = [];
              userChanged();
              return removed;
            }
            case "library_add_root":
              roots.push({
                id: roots.length + 1,
                name: "New folder",
                path: "E:\\New folder",
                online: true,
                tracks: 0,
              });
              revision += 1;
              return summary();
            case "library_remove_root":
              roots.splice(
                roots.findIndex((root) => root.id === args.id),
                1,
              );
              revision += 1;
              return summary();
            case "library_scan":
            case "library_cancel_scan":
              if (command === "library_cancel_scan") scan = null;
              publish();
              return undefined;
            default:
              throw "unavailable";
          }
        },
      },
    });
  }, options);
}

export interface Logged {
  command: string;
  args: Record<string, unknown>;
}

export async function calls(page: Page, command: string) {
  return page.evaluate(
    (name) =>
      (window as unknown as { testLibraryLog: Logged[] }).testLibraryLog.filter(
        (entry) => entry.command === name,
      ),
    command,
  );
}

/** Fonts and every visible cover must be settled before a capture. */
export async function settle(page: Page) {
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(150);
  await page.waitForFunction(() =>
    Array.from(document.images).every((image) => image.complete),
  );
}
