import { ListMusic, ListPlus, X } from "lucide-react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { Link, useSearchParams } from "react-router";
import type { AlbumSort, ArtistRow, TrackSort } from "../generated/contracts";
import { usePlayer } from "../player";
import { useSurface } from "../ui";
import { libraryApi } from "./api";
import { Artwork } from "./Artwork";
import { count, plural } from "./format";
import { useUser } from "./UserData";
import {
  ARTIST_ROW_HEIGHT,
  AlbumCard,
  Line,
  ListHead,
  PendingLine,
  ROW_HEIGHT,
  artistPath,
  listedTrack,
} from "./rows";
import { usePagedList } from "./usePagedList";

const TRACK_SORTS: { value: TrackSort; label: string }[] = [
  { value: "title", label: "Title" },
  { value: "artist", label: "Artist" },
  { value: "album", label: "Album" },
  { value: "recent", label: "Recently added" },
];
const ALBUM_SORTS: { value: AlbumSort; label: string }[] = [
  { value: "title", label: "Title" },
  { value: "artist", label: "Artist" },
  { value: "year", label: "Year" },
  { value: "recent", label: "Recently added" },
];

export function parseId(value: string | null | undefined) {
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed > 0 && parsed < 2 ** 32
    ? parsed
    : null;
}

function useSort<T extends string>(sorts: { value: T; label: string }[]) {
  const [params, setParams] = useSearchParams();
  const sort =
    sorts.find(({ value }) => value === params.get("sort"))?.value ??
    sorts[0].value;
  const control = (
    <label className="sort-control">
      <span>Sort by</span>
      <select
        value={sort}
        onChange={(event) => {
          const next = new URLSearchParams(params);
          next.set("sort", event.currentTarget.value);
          setParams(next, { replace: true });
        }}
      >
        {sorts.map(({ value, label }) => (
          <option key={value} value={value}>
            {label}
          </option>
        ))}
      </select>
    </label>
  );
  return { sort, control, params, setParams };
}

export function ListState({
  list,
  noun,
  empty,
  children,
}: {
  list: {
    failed: boolean;
    loading: boolean;
    total: number;
    refetch: () => void;
  };
  noun: string;
  empty: ReactNode;
  children: ReactNode;
}) {
  if (list.failed) {
    return (
      <p className="list-note" role="alert">
        {noun} couldn’t be loaded.{" "}
        <button className="link-button" onClick={list.refetch}>
          Try again
        </button>
      </p>
    );
  }
  if (!list.loading && list.total === 0) {
    return <div className="list-note">{empty}</div>;
  }
  return children;
}

export function Tracks() {
  const { sort, control, params, setParams } = useSort(TRACK_SORTS);
  const artist = parseId(params.get("artist"));
  const label = params.get("label")?.slice(0, 120) ?? null;
  const scroller = useRef<HTMLDivElement>(null);
  const { playLibrary } = usePlayer();
  const { notify } = useSurface();
  const { addToPlaylist } = useUser();
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const key = useMemo(() => [sort, artist], [sort, artist]);
  const fetchPage = useCallback(
    (offset: number, limit: number) =>
      libraryApi.tracks({
        sort,
        albumId: null,
        artistId: artist,
        offset,
        limit,
      }),
    [sort, artist],
  );
  const list = usePagedList({
    scope: "tracks",
    key,
    fetchPage,
    scroller,
    rowHeight: ROW_HEIGHT,
  });
  const toggle = useCallback((id: number) => {
    setSelected((previous) => {
      const next = new Set(previous);
      if (!next.delete(id) && next.size < 200) next.add(id);
      return next;
    });
  }, []);
  const ids = [...selected];
  return (
    <section className="library-section" aria-labelledby="tracks-title">
      <div className="section-bar">
        <h2 id="tracks-title" className="section-title">
          <span className="named-by-tab">Tracks </span>
          <span className="section-count">
            {list.loading ? "" : plural(list.total, "track")}
          </span>
        </h2>
        {control}
      </div>
      {artist != null && (
        <p className="filter-chip" role="status">
          Tracks with <strong>{label ?? "this artist"}</strong>
          <button
            type="button"
            className="chip-clear"
            aria-label="Show all tracks"
            onClick={() => setParams({ sort }, { replace: true })}
          >
            <X aria-hidden="true" />
          </button>
        </p>
      )}
      {ids.length > 0 && (
        <div className="selection-bar" role="group" aria-label="Selection">
          <strong role="status">{plural(ids.length, "track")} selected</strong>
          <button
            type="button"
            className="quiet"
            onClick={() =>
              addToPlaylist({
                kind: "tracks",
                ids,
                label: plural(ids.length, "track"),
              })
            }
          >
            <ListMusic aria-hidden="true" /> Add to playlist…
          </button>
          <button
            type="button"
            className="quiet"
            onClick={() => {
              playLibrary({ kind: "tracks", ids }, "append");
              notify(`Added ${plural(ids.length, "track")} to the queue`);
            }}
          >
            <ListPlus aria-hidden="true" /> Add to queue
          </button>
          <button
            type="button"
            className="quiet"
            onClick={() => setSelected(new Set())}
          >
            Clear selection
          </button>
        </div>
      )}
      <div
        className="virtual-table"
        role="table"
        aria-label="Tracks"
        aria-rowcount={list.total + 1}
      >
        <ListHead />
        <div className="virtual-scroll" ref={scroller} tabIndex={-1}>
          <ListState
            list={list}
            noun="Tracks"
            empty="No tracks yet. Add a folder or wait for the scan to finish."
          >
            <div
              className="virtual-canvas"
              role="rowgroup"
              style={{ height: list.virtualizer.getTotalSize() }}
            >
              {list.items.map((item) => {
                const track = list.rowAt(item.index);
                return (
                  <div
                    key={item.key}
                    className="virtual-row"
                    style={{ transform: `translateY(${item.start}px)` }}
                  >
                    {track ? (
                      <Line
                        item={listedTrack(track)}
                        number={item.index + 1}
                        rowIndex={item.index}
                        play={{ kind: "track", id: track.id }}
                        selected={selected.has(track.id)}
                        onSelect={() => toggle(track.id)}
                      />
                    ) : (
                      <PendingLine rowIndex={item.index} />
                    )}
                  </div>
                );
              })}
            </div>
          </ListState>
        </div>
      </div>
      <p className="visually-hidden" role="status">
        {list.loading ? "" : `${count(list.total)} tracks`}
      </p>
    </section>
  );
}

const MIN_CARD = 172;
const GAP = 20;
/** Title, artist and year lines under the artwork. */
const CARD_TEXT = 78;

/** The content width of a scroller, in whole pixels. */
function useWidth(scroller: RefObject<HTMLElement | null>) {
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) =>
      setWidth(Math.floor(entry.contentRect.width)),
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [scroller]);
  return width;
}

export function useGrid(scroller: RefObject<HTMLElement | null>) {
  const width = useWidth(scroller);
  const columns = Math.max(2, Math.floor((width + GAP) / (MIN_CARD + GAP)));
  const card = width > 0 ? (width - GAP * (columns - 1)) / columns : MIN_CARD;
  return { columns, rowHeight: Math.ceil(card + CARD_TEXT + GAP) };
}

export function Albums() {
  const { sort, control } = useSort(ALBUM_SORTS);
  const scroller = useRef<HTMLDivElement>(null);
  const grid = useGrid(scroller);
  const key = useMemo(() => [sort], [sort]);
  const fetchPage = useCallback(
    (offset: number, limit: number) =>
      libraryApi.albums({ sort, artistId: null, role: null, offset, limit }),
    [sort],
  );
  const list = usePagedList({
    scope: "albums",
    key,
    fetchPage,
    scroller,
    rowHeight: grid.rowHeight,
    group: grid.columns,
    overscan: 2,
  });
  return (
    <section className="library-section" aria-labelledby="albums-title">
      <div className="section-bar">
        <h2 id="albums-title" className="section-title">
          <span className="named-by-tab">Albums </span>
          <span className="section-count">
            {list.loading ? "" : plural(list.total, "album")}
          </span>
        </h2>
        {control}
      </div>
      <div className="virtual-scroll grid-scroll" ref={scroller} tabIndex={-1}>
        <ListState
          list={list}
          noun="Albums"
          empty="No albums yet. Albums appear once tracks have album tags."
        >
          <div
            className="virtual-canvas"
            role="list"
            aria-label="Albums"
            style={{ height: list.virtualizer.getTotalSize() }}
          >
            {list.items.map((row) => (
              <div
                key={row.key}
                className="album-row"
                style={{
                  transform: `translateY(${row.start}px)`,
                  gridTemplateColumns: `repeat(${grid.columns}, minmax(0, 1fr))`,
                }}
              >
                {Array.from({ length: grid.columns }, (_, column) => {
                  const index = row.index * grid.columns + column;
                  if (index >= list.total) return null;
                  const album = list.rowAt(index);
                  return album ? (
                    <div key={album.id} role="listitem">
                      <AlbumCard album={album} />
                    </div>
                  ) : (
                    <div key={`pending-${index}`} className="album-card">
                      <span className="album-art" />
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
        </ListState>
      </div>
    </section>
  );
}

export function ArtistLine({ artist }: { artist: ArtistRow }) {
  return (
    <div className="artist-row" role="listitem">
      <Artwork id={artist.artworkId} seed={artist.name} />
      <div className="cell-title">
        <Link
          className="artist-name"
          to={artistPath(artist.id)}
          title={artist.name}
        >
          {artist.name}
        </Link>
        <span className="line-sub">
          {plural(artist.albums, "album")} · {plural(artist.tracks, "track")}
        </span>
      </div>
    </div>
  );
}

const ARTIST_COLUMN = 340;

/** A directory: as many columns of names as the canvas has room for. */
export function Artists() {
  const scroller = useRef<HTMLDivElement>(null);
  const width = useWidth(scroller);
  const columns = Math.max(1, Math.floor(width / ARTIST_COLUMN));
  const key = useMemo(() => [], []);
  const fetchPage = useCallback(
    (offset: number, limit: number) => libraryApi.artists({ offset, limit }),
    [],
  );
  const list = usePagedList({
    scope: "artists",
    key,
    fetchPage,
    scroller,
    rowHeight: ARTIST_ROW_HEIGHT,
    group: columns,
  });
  return (
    <section className="library-section" aria-labelledby="artists-title">
      <div className="section-bar">
        <h2 id="artists-title" className="section-title">
          <span className="named-by-tab">Artists </span>
          <span className="section-count">
            {list.loading ? "" : plural(list.total, "artist")}
          </span>
        </h2>
      </div>
      <div
        className="virtual-scroll artist-directory"
        ref={scroller}
        tabIndex={-1}
      >
        <ListState
          list={list}
          noun="Artists"
          empty="No artists yet. Artists appear once tracks have artist tags."
        >
          <div
            className="virtual-canvas"
            role="list"
            aria-label="Artists"
            style={{ height: list.virtualizer.getTotalSize() }}
          >
            {list.items.map((row) => (
              <div
                key={row.key}
                className="artist-line"
                style={{
                  transform: `translateY(${row.start}px)`,
                  gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
                }}
              >
                {Array.from({ length: columns }, (_, column) => {
                  const artist = list.rowAt(row.index * columns + column);
                  return artist ? (
                    <ArtistLine key={artist.id} artist={artist} />
                  ) : null;
                })}
              </div>
            ))}
          </div>
        </ListState>
      </div>
    </section>
  );
}
