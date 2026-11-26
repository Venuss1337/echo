import {
  AudioLines,
  Disc3,
  Ellipsis,
  GripVertical,
  ListEnd,
  ListMusic,
  ListPlus,
  Pause,
  Play,
  Shuffle,
  Square,
  SquareCheck,
  Trash2,
  TriangleAlert,
  UserRound,
} from "lucide-react";
import { memo, useRef, type MouseEvent, type ReactNode } from "react";
import { Link, useNavigate } from "react-router";
import type { AlbumRow, PlayTarget, TrackRow } from "../generated/contracts";
import { usePlayer } from "../player";
import { choreograph } from "../shell/motion";
import { IconButton, useSurface, type MenuItem } from "../ui";
import { Artwork } from "./Artwork";
import { count, duration, plural } from "./format";
import { HeartButton, useUser } from "./UserData";

/** Must match `--row` and `--artist-row` in tokens.css. */
export const ROW_HEIGHT = 56;
export const ARTIST_ROW_HEIGHT = 76;

/** The fields every listing row needs, from a library or user-data row. */
export interface Listed {
  trackId: number | null;
  sourceId?: number;
  title: string;
  artist: string;
  album: string;
  albumId: number | null;
  artistId?: number | null;
  artworkId: number | null;
  durationMs: number;
  available: boolean;
  favorite: boolean;
}

export const listedTrack = (track: TrackRow): Listed => ({
  ...track,
  trackId: track.id,
});

/** The queue entry currently loaded, for marking rows. */
export function useCurrentTrack() {
  const { view } = usePlayer();
  const row = view?.tracks.find(({ id }) => id === view.current);
  return {
    trackId: row?.trackId ?? null,
    playing: view?.state === "playing" || view?.state === "buffering",
  };
}

export const albumPath = (id: number) => `/album/${id}`;
export const artistPath = (id: number) => `/artist/${id}`;

export const Line = memo(function Line({
  item,
  number,
  rowIndex,
  play,
  showAlbum = true,
  showArt = true,
  showArtist = true,
  selected = false,
  onSelect,
  onRemove,
  removeLabel,
  moves,
  grip,
  trailing,
}: {
  item: Listed;
  number?: number;
  /** Zero-based position, exposed to assistive technology. */
  rowIndex: number;
  play: PlayTarget;
  showAlbum?: boolean;
  /** Off where the cover is already the subject of the page. */
  showArt?: boolean;
  /** Off where every track shares the page's artist. */
  showArtist?: boolean;
  selected?: boolean;
  onSelect?: () => void;
  onRemove?: () => void;
  removeLabel?: string;
  moves?: { up?: () => void; down?: () => void };
  grip?: (event: React.PointerEvent<HTMLElement>) => void;
  trailing?: ReactNode;
}) {
  const { playLibrary, act, view } = usePlayer();
  const { openMenu, notify } = useSurface();
  const { addToPlaylist } = useUser();
  const navigate = useNavigate();
  const current = useCurrentTrack();
  const isCurrent = item.trackId != null && item.trackId === current.trackId;
  const { trackId } = item;
  const start = () => playLibrary(play, "replace");
  const items: (MenuItem | false | null | undefined)[] = [
    trackId != null && {
      label: "Play next",
      Icon: ListEnd,
      disabled: !item.available,
      onSelect: () => {
        playLibrary({ kind: "track", id: trackId }, "next");
        notify(`${item.title} plays next`);
      },
    },
    trackId != null && {
      label: "Add to queue",
      Icon: ListPlus,
      disabled: !item.available,
      onSelect: () => {
        playLibrary({ kind: "track", id: trackId }, "append");
        notify(`Added ${item.title} to the queue`);
      },
    },
    trackId != null && {
      label: "Add to playlist…",
      Icon: ListMusic,
      onSelect: () =>
        addToPlaylist({ kind: "tracks", ids: [trackId], label: item.title }),
    },
    onSelect && {
      label: selected ? "Deselect" : "Select",
      Icon: selected ? SquareCheck : Square,
      onSelect,
    },
    item.albumId != null && {
      label: "Go to album",
      Icon: Disc3,
      onSelect: () => void navigate(albumPath(item.albumId as number)),
    },
    item.artistId != null && {
      label: "Go to artist",
      Icon: UserRound,
      onSelect: () => void navigate(artistPath(item.artistId as number)),
    },
    moves?.up && { label: "Move up", onSelect: moves.up },
    moves?.down && { label: "Move down", onSelect: moves.down },
    onRemove && {
      label: removeLabel ?? "Remove",
      Icon: Trash2,
      danger: true,
      onSelect: onRemove,
    },
  ];
  const classes = [
    "track-row",
    item.available ? "" : "unavailable",
    isCurrent ? "is-current" : "",
    selected ? "is-selected" : "",
    grip ? "has-grip" : "",
  ].join(" ");
  return (
    <div
      className={classes}
      role="row"
      aria-rowindex={rowIndex + 2}
      aria-selected={onSelect ? selected : undefined}
      aria-current={isCurrent ? "true" : undefined}
      onDoubleClick={() => item.available && start()}
      onClick={(event) => {
        if (onSelect && (event.ctrlKey || event.metaKey)) onSelect();
      }}
      onContextMenu={(event) => openMenu(event, item.title, items)}
    >
      <div role="cell" className="cell-lead">
        {grip && (
          <button
            type="button"
            className="grip"
            aria-label={`Reorder ${item.title}. Use Alt with the arrow keys.`}
            title="Drag to reorder, or Alt + arrow keys"
            onPointerDown={grip}
            onKeyDown={(event) => {
              if (!event.altKey) return;
              const step =
                event.key === "ArrowUp"
                  ? moves?.up
                  : event.key === "ArrowDown"
                    ? moves?.down
                    : undefined;
              if (step) {
                event.preventDefault();
                step();
              }
            }}
          >
            <GripVertical aria-hidden="true" />
          </button>
        )}
        <span className="cell-number" aria-hidden="true">
          {isCurrent ? (
            <AudioLines className={current.playing ? "" : "is-paused"} />
          ) : (
            number != null && String(number).padStart(2, "0")
          )}
        </span>
        {item.available && (
          <IconButton
            className="row-play"
            label={
              isCurrent && current.playing
                ? `Pause ${item.title}`
                : `Play ${item.title}`
            }
            onClick={() => {
              if (isCurrent && current.playing) act({ kind: "pause" });
              else if (isCurrent && view?.state === "paused")
                act({ kind: "resume" });
              else start();
            }}
          >
            {isCurrent && current.playing ? (
              <Pause aria-hidden="true" />
            ) : (
              <Play aria-hidden="true" />
            )}
          </IconButton>
        )}
      </div>
      {showArt && (
        <div role="cell" className="cell-art">
          <Artwork
            id={item.artworkId}
            seed={`${item.album}\u0001${item.artist}`}
          />
        </div>
      )}
      <div role="cell" className="cell-title">
        <span className="line-title" title={item.title}>
          {item.title}
        </span>
        {(showArtist || !item.available) && (
          <span className="line-sub">
            {!item.available && (
              <span className="tag">
                <TriangleAlert aria-hidden="true" /> Unavailable
              </span>
            )}
            {showArtist && (
              <span className="artist-text">
                {item.artistId != null ? (
                  <Link to={artistPath(item.artistId)}>{item.artist}</Link>
                ) : (
                  item.artist || "Unknown artist"
                )}
              </span>
            )}
          </span>
        )}
      </div>
      <div role="cell" className="cell-album">
        {showAlbum &&
          (item.albumId != null ? (
            <Link to={albumPath(item.albumId)} title={item.album}>
              {item.album}
            </Link>
          ) : (
            item.album
          ))}
      </div>
      <div role="cell" className="cell-heart">
        {(trackId != null || item.sourceId != null) && (
          <HeartButton
            target={
              trackId != null
                ? { kind: "track", id: trackId }
                : { kind: "source", id: item.sourceId as number }
            }
            initial={item.favorite}
            title={item.title}
          />
        )}
      </div>
      <div role="cell" className="cell-time">
        {trailing ?? duration(item.durationMs)}
      </div>
      <div role="cell" className="cell-actions">
        <IconButton
          label={`More for ${item.title}`}
          onClick={(event) => openMenu(event, item.title, items)}
        >
          <Ellipsis aria-hidden="true" />
        </IconButton>
      </div>
    </div>
  );
});

export function PendingLine({ rowIndex }: { rowIndex: number }) {
  return (
    <div
      className="track-row pending"
      role="row"
      aria-rowindex={rowIndex + 2}
      aria-busy="true"
    >
      <span role="cell" className="cell-lead" />
      <span role="cell" className="cell-art">
        <span className="art" />
      </span>
      <span role="cell" className="cell-title">
        <span className="skeleton-line" />
      </span>
    </div>
  );
}

export function ListHead({ album = true, art = true, time = "Time" }) {
  return (
    <div className="table-head" role="row" aria-rowindex={1}>
      <span role="columnheader" className="cell-lead">
        <span className="visually-hidden">Number</span>
      </span>
      {art && (
        <span role="columnheader" className="cell-art">
          <span className="visually-hidden">Artwork</span>
        </span>
      )}
      <span role="columnheader" className="cell-title">
        Title
      </span>
      <span role="columnheader" className="cell-album">
        {album ? "Album" : <span className="visually-hidden">Album</span>}
      </span>
      <span role="columnheader" className="cell-heart">
        <span className="visually-hidden">Favorite</span>
      </span>
      <span role="columnheader" className="cell-time">
        {time}
      </span>
      <span role="columnheader" className="cell-actions">
        <span className="visually-hidden">Actions</span>
      </span>
    </div>
  );
}

/** Queue actions shared by every collection header. */
export function useCollectionMenu(
  label: string,
  target: PlayTarget,
): MenuItem[] {
  const { playLibrary } = usePlayer();
  const { notify } = useSurface();
  return [
    {
      label: "Shuffle",
      Icon: Shuffle,
      onSelect: () => playLibrary(target, "replace", true),
    },
    {
      label: "Play next",
      Icon: ListEnd,
      onSelect: () => {
        playLibrary(target, "next");
        notify(`${label} plays next`);
      },
    },
    {
      label: "Add to queue",
      Icon: ListPlus,
      onSelect: () => {
        playLibrary(target, "append");
        notify(`Added ${label} to the queue`);
      },
    },
  ];
}

export const AlbumCard = memo(function AlbumCard({
  album,
  detail = true,
}: {
  album: AlbumRow;
  detail?: boolean;
}) {
  const { playLibrary } = usePlayer();
  const { openMenu } = useSurface();
  const { addToPlaylist } = useUser();
  const navigate = useNavigate();
  const cover = useRef<HTMLAnchorElement>(null);
  const gone = album.availableTracks === 0;
  // The record itself travels from the shelf to its page.
  const open = (event: MouseEvent) => {
    const print = cover.current;
    if (
      !print ||
      event.button !== 0 ||
      event.ctrlKey ||
      event.metaKey ||
      event.shiftKey ||
      event.altKey
    )
      return;
    event.preventDefault();
    print.style.viewTransitionName = "record";
    choreograph(
      "record",
      () => {
        print.style.viewTransitionName = "";
        void navigate(albumPath(album.id));
      },
      () => document.querySelector(".album-page .sleeve") != null,
    );
  };
  const target = { kind: "album", id: album.id, startTrack: null } as const;
  const menu = useCollectionMenu(album.title, target);
  const items = [
    ...menu.map((item) => ({ ...item, disabled: gone })),
    {
      label: "Add to playlist…",
      Icon: ListMusic,
      onSelect: () =>
        addToPlaylist({ kind: "album", id: album.id, label: album.title }),
    },
  ];
  const missing = album.tracks - album.availableTracks;
  return (
    <article
      className={gone ? "album-card unavailable" : "album-card"}
      onContextMenu={(event) => openMenu(event, album.title, items)}
    >
      <div className="album-art">
        <Link
          ref={cover}
          to={albumPath(album.id)}
          tabIndex={-1}
          aria-hidden="true"
          className="album-cover"
          onClick={open}
        >
          <Artwork
            id={album.artworkId}
            seed={`${album.title}\u0001${album.artist}`}
            label={{ title: album.title, artist: album.artist }}
          />
        </Link>
        <div className="album-actions">
          <IconButton
            primary
            label={`Play ${album.title}`}
            disabled={gone}
            onClick={() => playLibrary(target, "replace")}
          >
            <Play aria-hidden="true" />
          </IconButton>
          <IconButton
            label={`More for ${album.title}`}
            onClick={(event) => openMenu(event, album.title, items)}
          >
            <Ellipsis aria-hidden="true" />
          </IconButton>
        </div>
      </div>
      <Link
        className="album-title"
        to={albumPath(album.id)}
        title={album.title}
        onClick={open}
      >
        {album.title}
      </Link>
      <span className="album-sub">
        {album.artistId != null ? (
          <Link to={artistPath(album.artistId)}>{album.artist}</Link>
        ) : (
          "Unknown artist"
        )}
      </span>
      {detail && (
        <span className="album-sub">
          {album.year ? `${album.year} · ` : ""}
          {plural(album.tracks, "track")}
          {gone
            ? " · unavailable"
            : missing > 0
              ? ` · ${count(missing)} missing`
              : ""}
        </span>
      )}
    </article>
  );
});
