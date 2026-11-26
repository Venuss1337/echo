import {
  keepPreviousData,
  useQueries,
  useQuery,
  type UseQueryResult,
} from "@tanstack/react-query";
import { ArrowRight, Ellipsis, ListMusic, Play, Shuffle } from "lucide-react";
import { Fragment, useMemo, type ReactNode } from "react";
import { Link, useParams } from "react-router";
import type {
  AlbumRow,
  ArtistRole,
  PlayTarget,
  TrackRow,
} from "../generated/contracts";
import { usePlayer } from "../player";
import { useField } from "../shell/Field";
import { IconButton, useSurface, type MenuItem } from "../ui";
import { libraryApi } from "./api";
import { Sleeve } from "./Artwork";
import { parseId } from "./Browse";
import { longDuration, plural } from "./format";
import { useLibrary } from "./LibraryProvider";
import {
  AlbumCard,
  Line,
  ListHead,
  artistPath,
  listedTrack,
  useCollectionMenu,
} from "./rows";
import { HeartButton, useUser } from "./UserData";

const PAGE = 200;

/** A full, non-virtual listing for one release. Bounded at 2,000 tracks. */
export function useAlbumTracks(id: number | null, total: number) {
  const { dataKey } = useLibrary();
  const pages = Math.min(10, Math.ceil(total / PAGE));
  const results = useQueries({
    queries: Array.from({ length: id == null ? 0 : pages }, (_, page) => ({
      queryKey: ["library", "album-tracks", dataKey, id, page],
      queryFn: () =>
        libraryApi.tracks({
          sort: "album" as const,
          albumId: id,
          artistId: null,
          offset: page * PAGE,
          limit: PAGE,
        }),
      placeholderData: keepPreviousData,
      staleTime: Infinity,
      gcTime: 60_000,
    })),
  });
  return results.flatMap(({ data }) => data?.rows ?? []);
}

/** Type shrinks with length so a very long title still reads as a title. */
export function titleScale(title: string) {
  return title.length > 64
    ? "title-longest"
    : title.length > 34
      ? "title-longer"
      : title.length > 18
        ? "title-long"
        : "";
}

export function Missing({
  noun,
  query,
}: {
  noun: string;
  query: UseQueryResult;
}) {
  if (query.isPending) return <section className="page" aria-busy="true" />;
  const gone = query.error === ("notFound" as unknown);
  return (
    <section className="page">
      <h1>{gone ? `${noun} not found` : `${noun} unavailable`}</h1>
      <p className="list-note">
        {gone
          ? "It may have left your library when its files moved or changed."
          : "ECHO couldn’t read it just now."}
      </p>
      {!gone && (
        <button
          type="button"
          className="quiet"
          onClick={() => void query.refetch()}
        >
          Try again
        </button>
      )}
    </section>
  );
}

export function PlayActions({
  label,
  target,
  disabled,
  extra = [],
  children,
}: {
  label: string;
  target: PlayTarget;
  disabled: boolean;
  extra?: MenuItem[];
  children?: ReactNode;
}) {
  const { playLibrary } = usePlayer();
  const { openMenu } = useSurface();
  const menu = useCollectionMenu(label, target).slice(1);
  return (
    <div className="actions">
      <button
        type="button"
        className="pill"
        disabled={disabled}
        onClick={() => playLibrary(target, "replace")}
      >
        <Play aria-hidden="true" /> Play
      </button>
      <IconButton
        className="ringed"
        label={`Shuffle ${label}`}
        disabled={disabled}
        onClick={() => playLibrary(target, "replace", true)}
      >
        <Shuffle aria-hidden="true" />
      </IconButton>
      {children}
      <IconButton
        className="ringed"
        label={`More for ${label}`}
        onClick={(event) =>
          openMenu(event, label, [
            ...menu.map((item) => ({ ...item, disabled })),
            ...extra,
          ])
        }
      >
        <Ellipsis aria-hidden="true" />
      </IconButton>
    </div>
  );
}

function discs(tracks: TrackRow[]) {
  const groups = new Map<number, TrackRow[]>();
  for (const track of tracks) {
    const disc = track.discNo ?? 1;
    groups.set(disc, [...(groups.get(disc) ?? []), track]);
  }
  return [...groups];
}

export function AlbumPage() {
  const id = parseId(useParams().id);
  const { dataKey } = useLibrary();
  const { addToPlaylist } = useUser();
  const query = useQuery({
    queryKey: ["library", "album", dataKey, id],
    queryFn: () => libraryApi.album(id as number),
    enabled: id != null,
    placeholderData: (previous) => (previous?.id === id ? previous : undefined),
    staleTime: Infinity,
    gcTime: 60_000,
    retry: false,
  });
  const album = query.data;
  const tracks = useAlbumTracks(album?.id ?? null, album?.tracks ?? 0);
  const groups = useMemo(() => discs(tracks), [tracks]);
  const seed = album ? `${album.title}\u0001${album.artist}` : "";
  useField(album ? { id: album.artworkId, seed } : null);
  if (!album) return <Missing noun="Album" query={query} />;
  const missing = album.tracks - album.availableTracks;
  let index = -1;
  return (
    <article className="page album-page">
      <div className="leaf-print">
        <Sleeve
          id={album.artworkId}
          title={album.title}
          artist={album.artist}
          seed={seed}
        />
      </div>
      <header className="leaf-head">
        <p className="kicker">
          {["Album", album.year].filter(Boolean).join(", ")}
        </p>
        <h1 className={titleScale(album.title)}>{album.title}</h1>
        <p className="byline">
          {album.artistId != null ? (
            <Link to={artistPath(album.artistId)}>{album.artist}</Link>
          ) : (
            "Unknown artist"
          )}
        </p>
        <p className="meta">
          {plural(album.tracks, "track")}
          <i />
          {longDuration(album.durationMs)}
          {groups.length > 1 && (
            <>
              <i />
              {plural(groups.length, "disc")}
            </>
          )}
          {missing > 0 && (
            <>
              <i />
              {missing === album.tracks
                ? "unavailable right now"
                : `${plural(missing, "track")} unavailable`}
            </>
          )}
        </p>
        <PlayActions
          label={album.title}
          target={{ kind: "album", id: album.id, startTrack: null }}
          disabled={album.availableTracks === 0}
          extra={[
            {
              label: "Add to playlist…",
              Icon: ListMusic,
              onSelect: () =>
                addToPlaylist({
                  kind: "album",
                  id: album.id,
                  label: album.title,
                }),
            },
          ]}
        >
          <HeartButton
            className="ringed"
            target={{ kind: "album", id: album.id }}
            initial={album.favorite}
            title={album.title}
          />
        </PlayActions>
      </header>
      <h2 className="visually-hidden">Track listing</h2>
      <div
        className="listing no-album no-art"
        role="table"
        aria-label={`Tracks on ${album.title}`}
      >
        <ListHead album={false} art={false} />
        {groups.map(([disc, rows]) => (
          <Fragment key={disc}>
            {groups.length > 1 && (
              <h3 className="disc-title" role="row">
                <span role="cell">Disc {disc}</span>
              </h3>
            )}
            {rows.map((track) => {
              index += 1;
              return (
                <Line
                  key={track.id}
                  item={listedTrack(track)}
                  number={track.trackNo ?? index + 1}
                  rowIndex={index}
                  play={{
                    kind: "album",
                    id: album.id,
                    startTrack: track.id,
                  }}
                  showAlbum={false}
                  showArt={false}
                  showArtist={track.artistId !== album.artistId}
                />
              );
            })}
          </Fragment>
        ))}
      </div>
    </article>
  );
}

function useArtistAlbums(id: number | null, role: ArtistRole) {
  const { dataKey } = useLibrary();
  return useQuery({
    queryKey: ["library", "artist-albums", dataKey, id, role],
    queryFn: () =>
      libraryApi.albums({
        sort: "year",
        artistId: id,
        role,
        offset: 0,
        limit: 200,
      }),
    enabled: id != null,
    staleTime: Infinity,
    gcTime: 60_000,
  }).data;
}

/** Up to three of the artist's own covers as one open sleeve. */
function CoverFan({ albums, name }: { albums: AlbumRow[]; name: string }) {
  const withArt = albums.filter(({ artworkId }) => artworkId != null);
  return (
    <div className="cover-fan" aria-hidden="true">
      {withArt.length > 0 ? (
        withArt
          .slice(0, 3)
          .reverse()
          .map((album) => (
            <Sleeve
              key={album.id}
              id={album.artworkId}
              title={album.title}
              artist={album.artist}
              seed={album.title}
            />
          ))
      ) : (
        <Sleeve id={null} title={name} artist="" seed={name} />
      )}
    </div>
  );
}

export function ArtistPage() {
  const id = parseId(useParams().id);
  const { dataKey } = useLibrary();
  const query = useQuery({
    queryKey: ["library", "artist", dataKey, id],
    queryFn: () => libraryApi.artist(id as number),
    enabled: id != null,
    placeholderData: (previous) => (previous?.id === id ? previous : undefined),
    staleTime: Infinity,
    gcTime: 60_000,
    retry: false,
  });
  const artist = query.data;
  const main = useArtistAlbums(id, "main");
  const appears = useArtistAlbums(id, "appears");
  const tracks = useQuery({
    queryKey: ["library", "artist-tracks", dataKey, id],
    queryFn: () =>
      libraryApi.tracks({
        sort: "album",
        albumId: null,
        artistId: id,
        offset: 0,
        limit: 8,
      }),
    enabled: id != null,
    staleTime: Infinity,
    gcTime: 60_000,
  }).data;
  const leading = main?.rows.find(({ artworkId }) => artworkId != null);
  const artwork = leading?.artworkId ?? artist?.artworkId ?? null;
  useField(artist && main ? { id: artwork, seed: artist.name } : null);
  if (!artist) return <Missing noun="Artist" query={query} />;
  const albums = main?.rows ?? [];
  const guest = appears?.rows ?? [];
  const label = encodeURIComponent(artist.name.slice(0, 120));
  return (
    <article className="page artist-page">
      <header className="artist-head">
        <div className="artist-name-block">
          <p className="kicker">Artist</p>
          <h1 className={titleScale(artist.name)}>{artist.name}</h1>
          <p className="meta">
            {albums.length > 0 && (
              <>
                {plural(albums.length, "album")}
                <i />
              </>
            )}
            {guest.length > 0 && (
              <>
                on {plural(guest.length, "other release")}
                <i />
              </>
            )}
            {plural(artist.tracks, "track")}
          </p>
          <PlayActions
            label={artist.name}
            target={{ kind: "artist", id: artist.id }}
            disabled={artist.tracks === 0 && albums.length === 0}
          />
        </div>
        {main && <CoverFan albums={[...albums, ...guest]} name={artist.name} />}
      </header>
      {albums.length > 0 && (
        <section aria-labelledby="artist-albums" className="shelf">
          <h2 id="artist-albums">Albums</h2>
          <div className="cards" role="list">
            {albums.map((album) => (
              <div key={album.id} role="listitem">
                <AlbumCard album={album} />
              </div>
            ))}
          </div>
        </section>
      )}
      {tracks && tracks.rows.length > 0 && (
        <section aria-labelledby="artist-tracks" className="shelf">
          <div className="shelf-title">
            <h2 id="artist-tracks">Tracks</h2>
            {tracks.total > tracks.rows.length && (
              <Link
                to={`/library/tracks?artist=${artist.id}&sort=album&label=${label}`}
              >
                All {tracks.total} tracks <ArrowRight aria-hidden="true" />
              </Link>
            )}
          </div>
          <div
            className="listing"
            role="table"
            aria-label={`Tracks by ${artist.name}`}
          >
            <ListHead />
            {tracks.rows.map((track, index) => (
              <Line
                key={track.id}
                item={listedTrack(track)}
                number={index + 1}
                rowIndex={index}
                play={{ kind: "track", id: track.id }}
              />
            ))}
          </div>
        </section>
      )}
      {guest.length > 0 && (
        <section aria-labelledby="artist-appears" className="shelf">
          <h2 id="artist-appears">Appears on</h2>
          <div className="cards" role="list">
            {guest.map((album) => (
              <div key={album.id} role="listitem">
                <AlbumCard album={album} />
              </div>
            ))}
          </div>
        </section>
      )}
    </article>
  );
}
