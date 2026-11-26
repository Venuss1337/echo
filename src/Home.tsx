import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { ArrowRight, FolderPlus, FolderOpen, Pause, Play } from "lucide-react";
import type { ReactNode } from "react";
import { Link, NavLink } from "react-router";
import type { AlbumRow, HomeView } from "./generated/contracts";
import { libraryApi } from "./library/api";
import { Artwork, Mosaic, Sleeve } from "./library/Artwork";
import { FavoriteAlbumCard } from "./library/Collections";
import { titleScale } from "./library/Detail";
import { ago, longDuration, plural } from "./library/format";
import { useLibrary } from "./library/LibraryProvider";
import { LibraryNotices, type Gate } from "./library/LibraryLayout";
import { AlbumCard, albumPath } from "./library/rows";
import { usePlayer } from "./player";
import { useField } from "./shell/Field";
import { formatTime, useExpand, useNowTrack, usePlayToggle } from "./transport";

function Shelf({
  id,
  title,
  link,
  children,
}: {
  id: string;
  title: string;
  link?: { to: string; label: string };
  children: ReactNode;
}) {
  return (
    <section className="shelf" aria-labelledby={id}>
      <div className="shelf-title">
        <h2 id={id}>{title}</h2>
        {link && (
          <Link to={link.to}>
            {link.label} <ArrowRight aria-hidden="true" />
          </Link>
        )}
      </div>
      {children}
    </section>
  );
}

function Cards({ albums }: { albums: AlbumRow[] }) {
  return (
    <div className="cards one-row" role="list">
      {albums.slice(0, 6).map((album) => (
        <div key={album.id} role="listitem">
          <AlbumCard album={album} detail={false} />
        </div>
      ))}
    </div>
  );
}

/** Records standing on one ledge, the most recent one nearest. */
function Ledge({ albums }: { albums: AlbumRow[] }) {
  const [lead, ...others] = albums;
  return (
    <section className="shelf ledge-shelf" aria-labelledby="recent">
      <div className="ledge-lead">
        <AlbumCard album={lead} detail={false} />
      </div>
      <div className="shelf-title">
        <h2 id="recent">Recently played</h2>
        <Link to="/library/albums">
          View albums <ArrowRight aria-hidden="true" />
        </Link>
      </div>
      <div className="ledge" role="list">
        {others.slice(0, 5).map((album) => (
          <div key={album.id} role="listitem">
            <AlbumCard album={album} detail={false} />
          </div>
        ))}
      </div>
    </section>
  );
}

/** What the turntable shows: the loaded track, else the most relevant album. */
function Hero({ home }: { home: HomeView | undefined }) {
  const { view, playLibrary } = usePlayer();
  const { pausing, disabled, toggle } = usePlayToggle();
  const now = useNowTrack();
  const expand = useExpand();
  const album =
    home?.recentAlbums[0] ??
    home?.addedAlbums[0] ??
    home?.favoriteAlbums.find((favorite) => favorite.album)?.album ??
    null;
  const artwork = now ? now.artworkId : (album?.artworkId ?? null);
  const title = now ? now.title : (album?.title ?? "");
  const artist = now ? now.artist : (album?.artist ?? "");
  const seed = `${now ? now.album : title}\u0001${artist}`;
  // Nothing is loaded, so this album lights the room instead.
  useField(!now && album ? { id: artwork, seed } : null);
  if (!now && !album) return null;
  const position = view?.positionMs ?? 0;
  const resumable = !pausing && position > 0;
  const eyebrow = now
    ? resumable
      ? "Pick up where you left off"
      : "On the turntable"
    : home?.recentAlbums[0]
      ? "Last listened to"
      : "New in your library";
  const open = now ? now.albumId : album!.id;
  const sleeve = (
    <Sleeve id={artwork} title={title} artist={artist} seed={seed} />
  );
  return (
    <section className="hero" aria-label={eyebrow}>
      <div className="hero-copy">
        <p className="kicker">{eyebrow}</p>
        <h2 className={`hero-title ${titleScale(title)}`}>{title}</h2>
        <p className="byline">
          {artist || "Local audio"}
          {now?.album && <span className="byline-album">{now.album}</span>}
        </p>
        <div className="actions">
          <button
            type="button"
            className="pill"
            disabled={now ? disabled : album!.availableTracks === 0}
            onClick={() => {
              if (now) toggle();
              else
                playLibrary(
                  { kind: "album", id: album!.id, startTrack: null },
                  "replace",
                );
            }}
          >
            {pausing ? (
              <Pause aria-hidden="true" />
            ) : (
              <Play aria-hidden="true" />
            )}
            {now
              ? pausing
                ? "Pause"
                : resumable
                  ? "Resume"
                  : "Play"
              : "Play album"}
          </button>
          {open != null && (
            <Link
              className="icon-button ringed"
              to={albumPath(open)}
              aria-label={`Open ${now ? now.album : title}`}
              title="Open album"
            >
              <ArrowRight aria-hidden="true" />
            </Link>
          )}
          <p className="meta">
            {now && resumable && view?.durationMs != null ? (
              `${formatTime(position)} of ${formatTime(view.durationMs)}`
            ) : !now && album ? (
              <>
                {album.year && (
                  <>
                    {album.year}
                    <i />
                  </>
                )}
                {plural(album.tracks, "track")}
                <i />
                {longDuration(album.durationMs)}
              </>
            ) : null}
          </p>
        </div>
      </div>
      {now ? (
        <button
          type="button"
          className="hero-sleeve"
          aria-label="Open Now Playing"
          onClick={() => expand(true)}
        >
          {sleeve}
        </button>
      ) : (
        <Link
          className="hero-sleeve"
          to={albumPath(album!.id)}
          aria-label={`Open ${title}`}
        >
          {sleeve}
        </Link>
      )}
    </section>
  );
}

function Rotation({ home }: { home: HomeView }) {
  const { playLibrary } = usePlayer();
  const frequent = home.frequent.length >= 3;
  const rows = frequent
    ? home.frequent.map((media) => ({ key: media.sourceId, media, note: "" }))
    : home.recentTracks.map((row) => ({
        key: row.playId,
        media: row.media,
        note: ago(row.startedAt),
      }));
  return (
    <Shelf
      id="rotation"
      title={frequent ? "In heavy rotation" : "Just heard"}
      link={{ to: "/history", label: "History" }}
    >
      <ol className="rotation">
        {rows.slice(0, 5).map(({ key, media, note }) => (
          <li key={key} className={media.available ? "" : "unavailable"}>
            <button
              type="button"
              disabled={!media.available || media.trackId == null}
              aria-label={`Play ${media.title}`}
              onClick={() =>
                playLibrary(
                  { kind: "track", id: media.trackId as number },
                  "replace",
                )
              }
            >
              <Artwork
                id={media.artworkId}
                seed={`${media.album}\u0001${media.artist}`}
              />
              <span>
                <strong>{media.title}</strong>
                <small>{media.artist}</small>
              </span>
              <em>{note}</em>
            </button>
          </li>
        ))}
      </ol>
    </Shelf>
  );
}

function PlaylistIndex({ home }: { home: HomeView }) {
  return (
    <Shelf
      id="sequences"
      title="Your playlists"
      link={{ to: "/playlists", label: "All playlists" }}
    >
      <ul className="playlist-index">
        {home.playlists.slice(0, 5).map((playlist) => (
          <li key={playlist.id}>
            <Link to={`/playlists/${playlist.id}`}>
              <span>{playlist.name}</span>
              <small>{plural(playlist.tracks, "track")}</small>
              <Mosaic ids={playlist.artworkIds} seed={playlist.name} />
            </Link>
          </li>
        ))}
      </ul>
    </Shelf>
  );
}

function EmptySleeve() {
  return (
    <div className="hero-sleeve" aria-hidden="true">
      <Sleeve id={null} title="" artist="" seed="" />
    </div>
  );
}

/** Home before there is any music, and while storage is not ready. */
export function Threshold({
  gate,
  retrying,
  onRetry,
}: {
  gate: Gate;
  retrying: boolean;
  onRetry: () => void;
}) {
  const { view, pick } = usePlayer();
  const { native, summary } = useLibrary();
  const heading =
    gate === "opening"
      ? "Opening your library"
      : gate
        ? "Library unavailable"
        : "Your library is empty";
  return (
    <section className="hero threshold">
      <div className="hero-copy">
        <div role="status">
          <h2 className="hero-title title-long">{heading}</h2>
          <p className="lede">
            {gate === "opening"
              ? "Checking local storage."
              : gate === "error"
                ? "ECHO couldn’t read the desktop status."
                : gate
                  ? "Restart ECHO to retry local storage."
                  : native
                    ? "Add the folder that holds your music, or open single files."
                    : "Folder import is available in the ECHO desktop app."}
          </p>
        </div>
        <div className="actions">
          {gate === "error" && (
            <button
              className="quiet"
              type="button"
              disabled={retrying}
              onClick={onRetry}
            >
              {retrying ? "Reconnecting…" : "Retry connection"}
            </button>
          )}
          {!gate && native && summary && (
            <NavLink className="pill" to="/library/folders">
              <FolderPlus aria-hidden="true" /> Add music folder
            </NavLink>
          )}
          {!gate && view && (
            <button
              className="quiet"
              type="button"
              onClick={() => pick("listen")}
            >
              <FolderOpen aria-hidden="true" /> Open audio files
            </button>
          )}
          {!gate && !view && (
            <NavLink className="quiet" to="/library">
              View library <ArrowRight aria-hidden="true" />
            </NavLink>
          )}
        </div>
      </div>
      <EmptySleeve />
    </section>
  );
}

export function Home({
  gate,
  retrying,
  onRetry,
}: {
  gate: Gate;
  retrying: boolean;
  onRetry: () => void;
}) {
  const { summary, dataKey } = useLibrary();
  const now = useNowTrack();
  const home = useQuery({
    queryKey: ["user", "home", dataKey, summary?.userRevision],
    queryFn: libraryApi.home,
    enabled: summary != null && !gate,
    placeholderData: keepPreviousData,
    staleTime: Infinity,
    gcTime: 60_000,
  }).data;
  const something =
    !gate &&
    (now != null ||
      (summary?.tracks ?? 0) > 0 ||
      (home != null &&
        (home.playlists.length > 0 || home.recentTracks.length > 0)));
  const recent = home?.recentAlbums ?? [];
  const added = home?.addedAlbums ?? [];
  const favorites = home?.favoriteAlbums ?? [];
  // The hero already shows this release.
  const featured = now ? now.albumId : ((recent[0] ?? added[0])?.id ?? null);
  const rest = (albums: AlbumRow[]) =>
    albums.filter(({ id }) => id !== featured);
  return (
    <section className="page home">
      <h1 className={something ? "home-title" : "visually-hidden"}>
        {something ? "Listening room" : "Home"}
      </h1>
      {something ? (
        <>
          <LibraryNotices />
          <Hero home={home} />
          {rest(recent).length > 0 && <Ledge albums={rest(recent)} />}
          {home &&
            (home.recentTracks.length > 0 || home.playlists.length > 0) && (
              <div className="split">
                {home.recentTracks.length > 0 && <Rotation home={home} />}
                {home.playlists.length > 0 && <PlaylistIndex home={home} />}
              </div>
            )}
          {favorites.length > 0 && (
            <Shelf
              id="kept"
              title="Kept close"
              link={{ to: "/favorites", label: "Favorites" }}
            >
              <div className="cards one-row" role="list">
                {favorites.slice(0, 6).map((favorite) => (
                  <div key={favorite.favoriteId} role="listitem">
                    <FavoriteAlbumCard favorite={favorite} />
                  </div>
                ))}
              </div>
            </Shelf>
          )}
          {rest(added).length > 0 && (
            <Shelf
              id="added"
              title="Recently added"
              link={{
                to: "/library/albums?sort=recent",
                label: "All additions",
              }}
            >
              <Cards albums={rest(added)} />
            </Shelf>
          )}
          {home && recent.length === 0 && added.length === 0 && !now && (
            <p className="list-note">
              Nothing with album tags yet.{" "}
              <Link className="link-button" to="/library/tracks">
                Browse tracks
              </Link>
            </p>
          )}
        </>
      ) : (
        <Threshold gate={gate} retrying={retrying} onRetry={onRetry} />
      )}
    </section>
  );
}
