import { keepPreviousData, useQuery } from "@tanstack/react-query";
import {
  Heart,
  History as HistoryIcon,
  ListMusic,
  Pencil,
  Plus,
  Trash2,
} from "lucide-react";
import {
  useCallback,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { Link, useNavigate, useParams } from "react-router";
import type {
  FavoriteAlbum,
  HistoryScope,
  PlaylistCard,
} from "../generated/contracts";
import { useField } from "../shell/Field";
import { Dialog, IconButton, useReorder, useSurface } from "../ui";
import { describeLibraryFault, libraryApi } from "./api";
import { Artwork, Mosaic } from "./Artwork";
import { ListState, parseId } from "./Browse";
import { useAtmosphere } from "./covers";
import { Missing, PlayActions, titleScale } from "./Detail";
import { ago, longDuration, plural } from "./format";
import { useLibrary } from "./LibraryProvider";
import { AlbumCard, Line, ListHead, PendingLine, ROW_HEIGHT } from "./rows";
import { HeartButton, PlaylistForm, usePlaylists, useUser } from "./UserData";
import { usePagedList } from "./usePagedList";

function Empty({
  Icon,
  title,
  children,
}: {
  Icon: typeof Heart;
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="empty-block">
      <Icon aria-hidden="true" />
      <h2>{title}</h2>
      <p>{children}</p>
    </div>
  );
}

function PlaylistTile({
  playlist,
  featured,
}: {
  playlist: PlaylistCard;
  featured: boolean;
}) {
  const atmosphere = useAtmosphere(playlist.artworkIds[0]);
  return (
    <Link
      className={featured ? "playlist-tile featured" : "playlist-tile"}
      to={`/playlists/${playlist.id}`}
      style={atmosphere as CSSProperties}
    >
      <Mosaic ids={playlist.artworkIds} seed={playlist.name} large={featured} />
      <span className="tile-copy">
        <strong>{playlist.name}</strong>
        {playlist.description && <em>{playlist.description}</em>}
        <small>
          {plural(playlist.tracks, "track")}
          {playlist.tracks > 0 && ` · ${longDuration(playlist.durationMs)}`}
        </small>
      </span>
    </Link>
  );
}

export function Playlists() {
  const playlists = usePlaylists();
  const { createPlaylist } = useUser();
  return (
    <section className="page">
      <header className="page-title">
        <h1>Playlists</h1>
        <button type="button" className="pill" onClick={createPlaylist}>
          <Plus aria-hidden="true" /> New playlist
        </button>
      </header>
      {playlists.isError ? (
        <p className="list-note" role="alert">
          Playlists couldn’t be loaded.
        </p>
      ) : playlists.data?.length === 0 ? (
        <Empty Icon={ListMusic} title="Nothing sequenced yet">
          A playlist keeps its order, its duplicates and its missing tracks.
          Start one here, or from any track’s menu.
        </Empty>
      ) : (
        <div className="playlist-wall" role="list">
          {playlists.data?.map((playlist, index) => (
            <div
              key={playlist.id}
              role="listitem"
              className={index === 0 ? "wall-lead" : undefined}
            >
              <PlaylistTile playlist={playlist} featured={index === 0} />
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

export function PlaylistPage() {
  const id = parseId(useParams().id);
  const { summary } = useLibrary();
  const { playlistAct } = useUser();
  const { notify } = useSurface();
  const navigate = useNavigate();
  const scroller = useRef<HTMLDivElement>(null);
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const revision = summary?.userRevision;
  const query = useQuery({
    queryKey: ["user", "playlist", revision, id],
    queryFn: () => libraryApi.playlist(id as number),
    enabled: id != null,
    placeholderData: (previous) => (previous?.id === id ? previous : undefined),
    staleTime: Infinity,
    gcTime: 60_000,
    retry: false,
  });
  const playlist = query.data;
  const key = useMemo(() => [id, revision], [id, revision]);
  const fetchPage = useCallback(
    (offset: number, limit: number) =>
      libraryApi.playlistEntries(id ?? 0, offset, limit),
    [id],
  );
  const list = usePagedList({
    scope: "playlist",
    key,
    fetchPage,
    scroller,
    rowHeight: ROW_HEIGHT,
  });
  const move = useCallback(
    (entry: string, before: string | null) => {
      if (id == null) return;
      void playlistAct({
        kind: "move",
        id,
        entryId: Number(entry),
        before: before == null ? null : Number(before),
      });
    },
    [id, playlistAct],
  );
  const reorder = useReorder(scroller, move);
  useField(
    playlist
      ? { id: playlist.artworkIds[0] ?? null, seed: playlist.name }
      : null,
  );
  if (!playlist || id == null) return <Missing noun="Playlist" query={query} />;
  const playable = playlist.tracks - playlist.unavailable;
  return (
    <article className="page fill-page playlist-page">
      <header className="playlisthead">
        <Mosaic ids={playlist.artworkIds} seed={playlist.name} large />
        <div className="playlist-copy">
          <p className="kicker">Playlist</p>
          <h1 className={titleScale(playlist.name)}>{playlist.name}</h1>
          {playlist.description && (
            <p className="lede">{playlist.description}</p>
          )}
          <p className="meta">
            {plural(playlist.tracks, "track")}
            {playlist.tracks > 0 && (
              <>
                <i />
                {longDuration(playlist.durationMs)}
              </>
            )}
            {playlist.unavailable > 0 && (
              <>
                <i />
                {plural(playlist.unavailable, "track")} unavailable
              </>
            )}
          </p>
          <PlayActions
            label={playlist.name}
            target={{ kind: "playlist", id, startEntry: null }}
            disabled={playable === 0}
            extra={[
              {
                label: "Edit details…",
                Icon: Pencil,
                onSelect: () => setEditing(true),
              },
              {
                label: "Delete playlist…",
                Icon: Trash2,
                danger: true,
                onSelect: () => setDeleting(true),
              },
            ]}
          />
        </div>
      </header>
      <div
        className="virtual-table"
        role="table"
        aria-label={`Tracks in ${playlist.name}`}
        aria-rowcount={list.total + 1}
      >
        <ListHead />
        <div className="virtual-scroll" ref={scroller} tabIndex={-1}>
          <ListState
            list={list}
            noun="Playlist tracks"
            empty="This playlist is empty. Add tracks from any listing’s menu."
          >
            <div
              className="virtual-canvas"
              role="rowgroup"
              style={{ height: list.virtualizer.getTotalSize() }}
            >
              {list.items.map((item) => {
                const entry = list.rowAt(item.index);
                const last = item.index === list.total - 1;
                const above = list.rowAt(item.index - 1);
                const below = list.rowAt(item.index + 2);
                const entryId = entry && String(entry.entryId);
                return (
                  <div
                    key={entryId ?? item.key}
                    className={`virtual-row ${entryId ? reorder.rowClass(entryId, last) : ""}`}
                    data-reorder-id={entryId}
                    data-reorder-last={last}
                    style={{ transform: `translateY(${item.start}px)` }}
                  >
                    {entry && entryId ? (
                      <Line
                        item={entry.media}
                        number={item.index + 1}
                        rowIndex={item.index}
                        play={{
                          kind: "playlist",
                          id,
                          startEntry: entry.entryId,
                        }}
                        grip={(event) => reorder.start(entryId, event)}
                        moves={{
                          up: above
                            ? () => move(entryId, String(above.entryId))
                            : undefined,
                          down: !last
                            ? () =>
                                move(
                                  entryId,
                                  below ? String(below.entryId) : null,
                                )
                            : undefined,
                        }}
                        removeLabel="Remove from playlist"
                        onRemove={() =>
                          void playlistAct({
                            kind: "remove",
                            id,
                            entryIds: [entry.entryId],
                          })
                        }
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
      {editing && (
        <PlaylistForm
          title="Edit playlist"
          submit="Save"
          initial={playlist}
          onClose={() => setEditing(false)}
          onSubmit={async (name, description) =>
            (await playlistAct({ kind: "edit", id, name, description })) != null
          }
        />
      )}
      {deleting && (
        <Dialog title="Delete playlist?" onClose={() => setDeleting(false)}>
          <p className="dialog-lede">
            <strong>{playlist.name}</strong> and its order will be removed. Your
            music files, favorites and whatever is playing stay as they are.
          </p>
          <div className="dialog-actions">
            <button
              type="button"
              className="quiet"
              autoFocus
              onClick={() => setDeleting(false)}
            >
              Keep playlist
            </button>
            <button
              type="button"
              className="pill"
              onClick={() =>
                void playlistAct({ kind: "delete", id }).then((done) => {
                  if (done == null) return;
                  setDeleting(false);
                  notify(`Deleted ${playlist.name}`);
                  void navigate("/playlists", { replace: true });
                })
              }
            >
              Delete playlist
            </button>
          </div>
        </Dialog>
      )}
    </article>
  );
}

function LostAlbum({ favorite }: { favorite: FavoriteAlbum }) {
  return (
    <article className="album-card unavailable">
      <div className="album-art">
        <Artwork
          id={favorite.artworkId}
          seed={`${favorite.title}\u0001${favorite.artist}`}
          label={favorite}
        />
        <div className="album-actions">
          <HeartButton
            target={{ kind: "favoriteAlbum", id: favorite.favoriteId }}
            initial
            title={favorite.title}
          />
        </div>
      </div>
      <span className="album-title">{favorite.title}</span>
      <span className="album-sub">{favorite.artist} · not in library</span>
    </article>
  );
}

export function FavoriteAlbumCard({ favorite }: { favorite: FavoriteAlbum }) {
  return favorite.album ? (
    <AlbumCard album={favorite.album} detail={false} />
  ) : (
    <LostAlbum favorite={favorite} />
  );
}

export function Favorites() {
  const { summary } = useLibrary();
  const revision = summary?.userRevision;
  const scroller = useRef<HTMLDivElement>(null);
  const albums = useQuery({
    queryKey: ["user", "favorite-albums", revision],
    queryFn: () => libraryApi.favoriteAlbums(0, 200),
    enabled: summary != null,
    placeholderData: keepPreviousData,
    staleTime: Infinity,
    gcTime: 60_000,
  }).data;
  const key = useMemo(() => [revision], [revision]);
  const fetchPage = useCallback(
    (offset: number, limit: number) => libraryApi.favoriteTracks(offset, limit),
    [],
  );
  const list = usePagedList({
    scope: "favorites",
    key,
    fetchPage,
    scroller,
    rowHeight: ROW_HEIGHT,
  });
  const nothing =
    albums?.length === 0 && !list.loading && !list.failed && list.total === 0;
  return (
    <section className="page fill-page">
      <header className="page-title">
        <h1>Favorites</h1>
        {list.total > 0 && (
          <PlayActions
            label="favorites"
            target={{ kind: "favorites", startSource: null }}
            disabled={false}
          />
        )}
      </header>
      {nothing ? (
        <Empty Icon={Heart} title="No favorites yet">
          Mark a track or album with the heart and it stays here, even while its
          drive is unplugged.
        </Empty>
      ) : (
        <>
          {albums && albums.length > 0 && (
            <div
              className="ribbon"
              role="list"
              aria-label="Favorite albums"
              tabIndex={0}
            >
              {albums.map((favorite) => (
                <div key={favorite.favoriteId} role="listitem">
                  <FavoriteAlbumCard favorite={favorite} />
                </div>
              ))}
            </div>
          )}
          <div
            className="virtual-table"
            role="table"
            aria-label="Favorite tracks"
            aria-rowcount={list.total + 1}
          >
            <ListHead />
            <div className="virtual-scroll" ref={scroller} tabIndex={-1}>
              <ListState
                list={list}
                noun="Favorites"
                empty="No favorite tracks yet."
              >
                <div
                  className="virtual-canvas"
                  role="rowgroup"
                  style={{ height: list.virtualizer.getTotalSize() }}
                >
                  {list.items.map((item) => {
                    const media = list.rowAt(item.index);
                    return (
                      <div
                        key={media?.sourceId ?? item.key}
                        className="virtual-row"
                        style={{ transform: `translateY(${item.start}px)` }}
                      >
                        {media ? (
                          <Line
                            item={media}
                            number={item.index + 1}
                            rowIndex={item.index}
                            play={{
                              kind: "favorites",
                              startSource: media.sourceId,
                            }}
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
        </>
      )}
    </section>
  );
}

const SCOPES: { scope: HistoryScope; label: string }[] = [
  { scope: "lastHour", label: "Last hour" },
  { scope: "lastDay", label: "Last 24 hours" },
  { scope: "all", label: "Everything" },
];

export function History() {
  const { summary } = useLibrary();
  const { notify } = useSurface();
  const revision = summary?.userRevision;
  const scroller = useRef<HTMLDivElement>(null);
  const [clearing, setClearing] = useState(false);
  const [scope, setScope] = useState<HistoryScope>("lastHour");
  const key = useMemo(() => [revision], [revision]);
  const fetchPage = useCallback(
    (offset: number, limit: number) => libraryApi.history(offset, limit),
    [],
  );
  const list = usePagedList({
    scope: "history",
    key,
    fetchPage,
    scroller,
    rowHeight: ROW_HEIGHT,
  });
  return (
    <section className="page fill-page">
      <header className="page-title">
        <h1>History</h1>
        {list.total > 0 && (
          <button
            type="button"
            className="quiet"
            onClick={() => setClearing(true)}
          >
            Clear history…
          </button>
        )}
      </header>
      {!list.loading && !list.failed && list.total === 0 ? (
        <Empty Icon={HistoryIcon} title="No listening recorded">
          A track counts after thirty seconds, or half of a shorter track. The
          record stays on this computer.
        </Empty>
      ) : (
        <div
          className="virtual-table"
          role="table"
          aria-label="Listening history"
          aria-rowcount={list.total + 1}
        >
          <ListHead time="Played" />
          <div className="virtual-scroll" ref={scroller} tabIndex={-1}>
            <ListState list={list} noun="History" empty="">
              <div
                className="virtual-canvas"
                role="rowgroup"
                style={{ height: list.virtualizer.getTotalSize() }}
              >
                {list.items.map((item) => {
                  const row = list.rowAt(item.index);
                  return (
                    <div
                      key={row?.playId ?? item.key}
                      className="virtual-row"
                      style={{ transform: `translateY(${item.start}px)` }}
                    >
                      {row ? (
                        <Line
                          item={row.media}
                          rowIndex={item.index}
                          play={{
                            kind: "track",
                            id: row.media.trackId ?? 0,
                          }}
                          trailing={ago(row.startedAt)}
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
      )}
      {clearing && (
        <Dialog
          title="Clear listening history"
          onClose={() => setClearing(false)}
        >
          <p className="dialog-lede">
            This removes the record of what you played. Playlists, favorites and
            files are not affected. It can’t be undone.
          </p>
          <fieldset className="choices">
            <legend>Clear</legend>
            {SCOPES.map((option) => (
              <label key={option.scope}>
                <input
                  type="radio"
                  name="scope"
                  checked={scope === option.scope}
                  onChange={() => setScope(option.scope)}
                />
                {option.label}
              </label>
            ))}
          </fieldset>
          <div className="dialog-actions">
            <button
              type="button"
              className="quiet"
              onClick={() => setClearing(false)}
            >
              Keep history
            </button>
            <button
              type="button"
              className="pill"
              onClick={() =>
                void libraryApi.clearHistory(scope).then(
                  (removed) => {
                    setClearing(false);
                    notify(`Cleared ${plural(removed, "play")}`);
                  },
                  (error) => notify(describeLibraryFault(error)),
                )
              }
            >
              Clear history
            </button>
          </div>
        </Dialog>
      )}
    </section>
  );
}

export function NewPlaylistButton() {
  const { createPlaylist } = useUser();
  return (
    <IconButton label="New playlist" onClick={createPlaylist}>
      <Plus aria-hidden="true" />
    </IconButton>
  );
}
