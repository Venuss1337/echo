import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { FolderOpen, FolderPlus } from "lucide-react";
import type { ReactNode } from "react";
import {
  NavLink,
  Navigate,
  Outlet,
  Route,
  Routes,
  useLocation,
  useSearchParams,
} from "react-router";
import { usePlayer } from "../player";
import { libraryApi } from "./api";
import { Albums, ArtistLine, Artists, Tracks } from "./Browse";
import { Folders, ScanProgress } from "./Folders";
import { plural } from "./format";
import { useLibrary } from "./LibraryProvider";
import { AlbumCard, Line, listedTrack } from "./rows";

const tabs = [
  { to: "/library/tracks", label: "Tracks" },
  { to: "/library/albums", label: "Albums" },
  { to: "/library/artists", label: "Artists" },
  { to: "/library/folders", label: "Folders" },
];

/** The library has three honest "not ready" states that never show data. */
export type Gate = "opening" | "unavailable" | "error" | null;

export function LibraryRoutes() {
  return (
    <Routes>
      <Route element={<Frame />}>
        <Route index element={<Navigate to="tracks" replace />} />
        <Route path="tracks" element={<Tracks />} />
        <Route path="albums" element={<Albums />} />
        <Route path="artists" element={<Artists />} />
        <Route path="folders" element={<Folders />} />
        <Route path="*" element={<Navigate to="/library/tracks" replace />} />
      </Route>
    </Routes>
  );
}

/** Scan progress and offline drives, wherever the library is shown. */
export function LibraryNotices({ scan = true }: { scan?: boolean }) {
  const { summary, fault, dismissFault, busy, cancelScan } = useLibrary();
  const offline = summary?.roots.filter((root) => !root.online) ?? [];
  const scanning = scan ? summary?.scan : null;
  const root = summary?.roots.find(({ id }) => id === scanning?.rootId);
  return (
    <>
      {fault && (
        <p className="notice" role="alert">
          {fault}{" "}
          <button className="link-button" type="button" onClick={dismissFault}>
            Dismiss
          </button>
        </p>
      )}
      {scanning && (
        <ScanProgress
          scan={scanning}
          name={root?.name ?? "your music"}
          busy={busy === "scan"}
          onCancel={cancelScan}
        />
      )}
      {offline.length > 0 && (
        <p className="notice" role="status">
          {offline.length === 1
            ? `${offline[0].name} is offline.`
            : `${plural(offline.length, "folder")} are offline.`}{" "}
          Tracks from {offline.length === 1 ? "it" : "them"} stay listed as
          unavailable and return with the drive.
        </p>
      )}
    </>
  );
}

export function EmptyLibrary() {
  const { native, summary, addRoot, busy } = useLibrary();
  const { view, pick } = usePlayer();
  return (
    <div className="empty-library">
      <div role="status">
        <h2>Your library is empty</h2>
        <p>
          Choose a folder of music. ECHO indexes it in the background and never
          changes your files.
        </p>
      </div>
      <div className="actions">
        {native && summary && (
          <button
            className="pill"
            type="button"
            disabled={busy != null}
            onClick={addRoot}
          >
            <FolderPlus aria-hidden="true" />{" "}
            {busy === "add" ? "Choosing…" : "Add music folder"}
          </button>
        )}
        {view && (
          <button
            className="quiet"
            type="button"
            onClick={() => pick("listen")}
          >
            <FolderOpen aria-hidden="true" /> Open audio files
          </button>
        )}
      </div>
    </div>
  );
}

function Frame() {
  const { summary } = useLibrary();
  const { pathname } = useLocation();
  const folders = pathname === "/library/folders";
  const empty = (summary?.roots.length ?? 0) === 0;
  return (
    <section className="page fill-page library-page">
      <h1 className="visually-hidden">Library</h1>
      {empty && !folders ? (
        <EmptyLibrary />
      ) : (
        <>
          <nav className="tabs" aria-label="Library sections">
            {tabs.map(({ to, label }) => (
              <NavLink key={to} to={to}>
                {label}
              </NavLink>
            ))}
          </nav>
          {!folders && <LibraryNotices />}
          <Outlet />
        </>
      )}
    </section>
  );
}

const SEARCH_LIMIT = 24;

/** Results for the title bar's field, which owns the query text. */
export function Search() {
  const [params] = useSearchParams();
  const { dataKey } = useLibrary();
  const text = (params.get("q") ?? "").slice(0, 200);
  const results = useQuery({
    queryKey: ["library", "search", dataKey, text],
    queryFn: () => libraryApi.search(text, SEARCH_LIMIT),
    enabled: text.trim().length > 0,
    placeholderData: keepPreviousData,
    staleTime: Infinity,
    gcTime: 30_000,
  });
  const data = text.trim() ? results.data : undefined;
  const empty =
    data && !data.tracks.length && !data.albums.length && !data.artists.length;
  return (
    <section className="page search-page">
      <header className="page-title">
        <h1>
          <span className="visually-hidden">Search</span>
          <span aria-hidden="true">{text.trim() || "Search"}</span>
        </h1>
      </header>
      {results.isError && (
        <p className="list-note" role="alert">
          Search couldn’t run.{" "}
          <button
            className="link-button"
            type="button"
            onClick={() => void results.refetch()}
          >
            Try again
          </button>
        </p>
      )}
      <p className="visually-hidden" role="status">
        {data
          ? `${plural(data.tracks.length, "track")}, ${plural(data.albums.length, "album")}, ${plural(data.artists.length, "artist")}`
          : ""}
      </p>
      {!text.trim() && (
        <p className="list-note">
          Type in the field above. Matches begin at the start of any word in a
          title, album or name.
        </p>
      )}
      {empty && (
        <p className="list-note">
          Nothing matches <q>{text}</q>.
        </p>
      )}
      {data && data.artists.length > 0 && (
        <Found id="found-artists" title="Artists">
          <div
            className="artist-results"
            role="list"
            aria-label="Artist results"
          >
            {data.artists.slice(0, 6).map((artist) => (
              <ArtistLine key={artist.id} artist={artist} />
            ))}
          </div>
        </Found>
      )}
      {data && data.albums.length > 0 && (
        <Found id="found-albums" title="Albums">
          <div className="cards" role="list">
            {data.albums.map((album) => (
              <div key={album.id} role="listitem">
                <AlbumCard album={album} />
              </div>
            ))}
          </div>
        </Found>
      )}
      {data && data.tracks.length > 0 && (
        <Found id="found-tracks" title="Tracks">
          <div role="table" aria-label="Track results" className="listing">
            {data.tracks.map((track, index) => (
              <Line
                key={track.id}
                item={listedTrack(track)}
                number={index + 1}
                rowIndex={index}
                play={{ kind: "track", id: track.id }}
              />
            ))}
          </div>
        </Found>
      )}
    </section>
  );
}

function Found({
  id,
  title,
  children,
}: {
  id: string;
  title: string;
  children: ReactNode;
}) {
  return (
    <section aria-labelledby={id} className="shelf">
      <h2 id={id}>{title}</h2>
      {children}
    </section>
  );
}
