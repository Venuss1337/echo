import {
  Disc3,
  FolderCog,
  FolderOpen,
  Heart,
  History as HistoryIcon,
  House,
  Library,
  ListMusic,
  Radio,
  Search as SearchIcon,
  UserRound,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useRef } from "react";
import { Link, NavLink } from "react-router";
import { Mosaic } from "../library/Artwork";
import { NewPlaylistButton } from "../library/Collections";
import { usePlaylists } from "../library/UserData";
import { usePlayer } from "../player";
import { EchoMark } from "./EchoMark";

export type SidebarMode = "full" | "rail" | "drawer";

/** Grouped by intent: go somewhere, browse everything, return to what is kept. */
const groups: { label: string; items: [string, string, LucideIcon][] }[] = [
  {
    label: "Start",
    items: [
      ["/", "Home", House],
      ["/search", "Search", SearchIcon],
      ["/radio", "Radio", Radio],
    ],
  },
  {
    label: "Collection",
    items: [
      ["/library/tracks", "Library", Library],
      ["/library/albums", "Albums", Disc3],
      ["/library/artists", "Artists", UserRound],
    ],
  },
  {
    label: "Kept",
    items: [
      ["/favorites", "Favorites", Heart],
      ["/history", "History", HistoryIcon],
      ["/playlists", "Playlists", ListMusic],
    ],
  },
];

/** Which entry owns the current page, including its detail pages. */
function owner(pathname: string) {
  if (pathname.startsWith("/album/")) return "/library/albums";
  if (pathname.startsWith("/artist/")) return "/library/artists";
  if (pathname.startsWith("/playlists/")) return "";
  if (pathname.startsWith("/library")) {
    return /\/(albums|artists|folders)$/.test(pathname)
      ? pathname
      : "/library/tracks";
  }
  return pathname;
}

export function Sidebar({
  pathname,
  mode,
  open,
  onClose,
}: {
  pathname: string;
  mode: SidebarMode;
  /** A drawer that is showing over the page. */
  open: boolean;
  onClose: () => void;
}) {
  const playlists = usePlaylists();
  const { view, pick } = usePlayer();
  const aside = useRef<HTMLElement>(null);
  const active = owner(pathname);
  const modal = open && mode !== "full";
  useEffect(() => {
    if (!modal) return;
    const origin = document.activeElement;
    aside.current?.querySelector<HTMLElement>("nav a")?.focus();
    return () => {
      if (origin instanceof HTMLElement && origin.isConnected) origin.focus();
    };
  }, [modal]);
  const item = (to: string, name: string, Icon: LucideIcon) => (
    <Link
      key={to}
      to={to}
      title={name}
      aria-current={active === to ? "page" : undefined}
      onClick={onClose}
    >
      <Icon aria-hidden="true" /> <span>{name}</span>
    </Link>
  );
  return (
    <>
      {modal && <div className="drawer-scrim" onPointerDown={onClose} />}
      <aside
        ref={aside}
        className="sidebar"
        data-open={modal || undefined}
        onKeyDown={(event) => {
          if (modal && event.key === "Escape") {
            event.stopPropagation();
            onClose();
          }
        }}
      >
        <div className="sidebar-head">
          <Link
            className="wordmark"
            to="/"
            aria-label="ECHO home"
            onClick={onClose}
          >
            <EchoMark /> <span>ECHO</span>
          </Link>
        </div>
        <nav aria-label="Workspace">
          {groups.map(({ label, items }) => (
            <div key={label} className="nav-group">
              {items.map(([to, name, Icon]) => item(to, name, Icon))}
            </div>
          ))}
          <div className="nav-group nav-playlists">
            <div className="nav-label">
              <span>Your playlists</span> <NewPlaylistButton />
            </div>
            {playlists.data?.slice(0, 40).map((playlist) => (
              <NavLink
                key={playlist.id}
                to={`/playlists/${playlist.id}`}
                title={playlist.name}
                onClick={onClose}
              >
                <Mosaic ids={playlist.artworkIds} seed={playlist.name} />{" "}
                <span>{playlist.name}</span>
              </NavLink>
            ))}
          </div>
        </nav>
        <div className="sidebar-foot">
          {view && (
            <button
              type="button"
              title="Open local audio"
              onClick={() => {
                onClose();
                pick("listen");
              }}
            >
              <FolderOpen aria-hidden="true" /> <span>Open local audio</span>
            </button>
          )}
          {item("/library/folders", "Music folders", FolderCog)}
        </div>
      </aside>
    </>
  );
}
