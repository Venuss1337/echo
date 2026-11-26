import { useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { NavLink, Route, Routes, useLocation, useNavigate } from "react-router";
import { CompactPlayer } from "./Compact";
import { Dock } from "./Dock";
import { Home, Threshold } from "./Home";
import {
  Favorites,
  History,
  PlaylistPage,
  Playlists,
} from "./library/Collections";
import { AlbumPage, ArtistPage } from "./library/Detail";
import { LibraryRoutes, Search, type Gate } from "./library/LibraryLayout";
import { LibraryProvider } from "./library/LibraryProvider";
import { UserProvider } from "./library/UserData";
import { NowPlaying } from "./NowPlaying";
import { PlayerProvider, usePlayer } from "./player";
import { QueuePanel } from "./Queue";
import type { Station } from "./generated/contracts";
import { RadioPage, stationLight } from "./radio/Radio";
import { StationInspector } from "./radio/StationInspector";
import { returnFromMini, useMedia, useMiniWindow } from "./shell/chrome";
import { Field, FieldProvider, useLight, useRoomLight } from "./shell/Field";
import type { InspectorPane } from "./shell/Inspector";
import { choreograph } from "./shell/motion";
import { Sidebar, type SidebarMode } from "./shell/Sidebar";
import { TitleBar } from "./shell/TitleBar";
import { getAppStatus } from "./status";
import { nowSeed, useExpand, useNowTrack, usePlayToggle } from "./transport";
import { SurfaceProvider } from "./ui";

export default function App() {
  return (
    <SurfaceProvider>
      <PlayerProvider>
        <LibraryProvider>
          <UserProvider>
            <FieldProvider>
              <Workspace />
            </FieldProvider>
          </UserProvider>
        </LibraryProvider>
      </PlayerProvider>
    </SurfaceProvider>
  );
}

function typing(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    (target.isContentEditable ||
      /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName))
  );
}

const SIDEBAR_KEY = "echo.sidebar";

/** Remembers whether the listener prefers the rail. Losing it is harmless. */
function useSidebarPreference() {
  const [rail, setRail] = useState(() => {
    try {
      return localStorage.getItem(SIDEBAR_KEY) === "rail";
    } catch {
      return false;
    }
  });
  const prefer = useCallback((next: boolean) => {
    setRail(next);
    try {
      localStorage.setItem(SIDEBAR_KEY, next ? "rail" : "full");
    } catch {
      // Storage may be unavailable; the choice then lasts for this session.
    }
  }, []);
  return [rail, prefer] as const;
}

/**
 * How much of the photograph a page lets in. Pages made of one composition are
 * `lit`; pages with a listing to read keep the light in their header; the rest
 * of the library has only its colour.
 */
function exposure(pathname: string) {
  if (/^\/(album|playlists)\/./.test(pathname) || pathname === "/radio")
    return "read";
  return pathname === "/" ||
    pathname.startsWith("/artist/") ||
    pathname === "/now-playing"
    ? "lit"
    : "low";
}

/** Scrolling into the collection lowers the light behind it. */
const DIM_AFTER_PX = 150;

function Workspace() {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const { view, act } = usePlayer();
  const { toggle } = usePlayToggle();
  const now = useNowTrack();
  const expand = useExpand();
  const [pane, setPane] = useState<InspectorPane | null>(null);
  const [station, setStation] = useState<Station | "new">("new");
  const [compactEdition, setCompact] = useState(false);
  const mini = useMiniWindow();
  const compact = compactEdition || mini;
  const [drawer, setDrawer] = useState(false);
  const [scrolled, setScrolled] = useState(false);
  const [rail, preferRail] = useSidebarPreference();
  const main = useRef<HTMLElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const immersive = pathname === "/now-playing";
  const small = useMedia("(max-width: 680px)");
  const tight = useMedia("(max-width: 1020px)");
  // The inspector needs the room the labels would take.
  const shared = useMedia("(max-width: 1240px)") && pane != null;
  const mode: SidebarMode = small
    ? "drawer"
    : tight || shared || rail
      ? "rail"
      : "full";
  const light = useLight(
    view?.live
      ? stationLight(view.live.station)
      : now
        ? { id: now.artworkId, seed: nowSeed(now) }
        : null,
  );
  useRoomLight(light);
  const status = useQuery({
    queryKey: ["app-status"],
    queryFn: getAppStatus,
    retry: false,
    refetchInterval: ({ state }) =>
      state.data?.library === "opening" && state.status !== "error"
        ? 300
        : false,
  });
  const gate: Gate = status.isError
    ? "error"
    : status.isPending || status.data.library === "opening"
      ? "opening"
      : status.data.library === "unavailable"
        ? "unavailable"
        : null;
  const threshold = {
    gate,
    retrying: status.isFetching,
    onRetry: () => void status.refetch(),
  };

  const toggleQueue = useCallback(() => {
    choreograph("inspector", () =>
      setPane((open) => (open === "queue" ? null : "queue")),
    );
  }, []);
  const closeStation = () => choreograph("inspector", () => setPane(null));
  const showStation = (selection: Station | "new") => {
    setStation(selection);
    if (pane !== "station") choreograph("inspector", () => setPane("station"));
  };
  const showCompact = useCallback(
    (next: boolean) => {
      if (!next && mini) returnFromMini();
      choreograph("compact", () => setCompact(next));
    },
    [mini],
  );
  const toggleSidebar = useCallback(() => {
    if (small || tight || shared) setDrawer((open) => !open);
    else choreograph("sidebar", () => preferRail(!rail));
  }, [preferRail, rail, shared, small, tight]);

  // A drawer is only a drawer while there is no room beside the page.
  if (drawer && !small && !tight && !shared) setDrawer(false);
  // Leaving a page also leaves its drawer and its scroll position.
  const [visited, setVisited] = useState(pathname);
  if (visited !== pathname) {
    setVisited(pathname);
    setDrawer(false);
    setScrolled(false);
    if (pane === "station") setPane(null);
  }
  useEffect(() => {
    // Only the page scrolls back to its top; focus stays where it was.
    main.current?.scrollTo?.(0, 0);
  }, [pathname]);

  useEffect(() => {
    function shortcut(event: KeyboardEvent) {
      if (
        event.defaultPrevented ||
        event.ctrlKey ||
        event.metaKey ||
        event.altKey ||
        document.querySelector("[aria-modal=true]")
      )
        return;
      if (event.key === "Escape") {
        // The compact edition hides everything else, so it goes first.
        if (compact) showCompact(false);
        else if (pane) choreograph("inspector", () => setPane(null));
        else if (immersive) expand(false);
        return;
      }
      if (typing(event.target)) return;
      const onControl =
        event.target instanceof HTMLElement &&
        event.target.closest("button, a, [role=menu]") != null;
      switch (event.key) {
        case " ":
          if (onControl || !view) return;
          event.preventDefault();
          toggle();
          break;
        case "n":
          if (view?.hasNext) act({ kind: "next" });
          break;
        case "p":
          if (view?.hasPrevious) act({ kind: "previous" });
          break;
        case "m":
          if (view) act({ kind: "mute", enabled: !view.muted });
          break;
        case "q":
          toggleQueue();
          break;
        case "/":
          event.preventDefault();
          setCompact(false);
          if (search.current?.offsetParent) search.current.focus();
          else void navigate("/search");
          break;
      }
    }
    window.addEventListener("keydown", shortcut);
    return () => window.removeEventListener("keydown", shortcut);
  }, [
    act,
    compact,
    expand,
    immersive,
    navigate,
    pane,
    showCompact,
    toggle,
    toggleQueue,
    view,
  ]);

  if (compact) return <CompactPlayer onExit={() => showCompact(false)} />;

  const gated = (page: React.ReactNode) =>
    gate ? (
      <section className="page home">
        <h1 className="visually-hidden">Library</h1>
        <Threshold {...threshold} />
      </section>
    ) : (
      page
    );
  const level = gate ? "lit" : exposure(pathname);
  return (
    <div
      className={immersive ? "shell is-immersive" : "shell"}
      data-sidebar={immersive ? "none" : mode}
      data-exposure={scrolled ? "low" : level}
      data-inspector={pane ?? undefined}
    >
      <Field light={light} />
      <a
        className="skip"
        href="#main"
        onClick={(event) => {
          event.preventDefault();
          main.current?.focus();
        }}
      >
        Skip to content
      </a>
      <TitleBar
        search={search}
        immersive={immersive}
        collapsed={mode !== "full" && !drawer}
        inert={drawer}
        onSidebar={toggleSidebar}
        onLeave={() => expand(false)}
      />
      {!immersive && (
        <Sidebar
          pathname={pathname}
          mode={mode}
          open={drawer}
          onClose={() => setDrawer(false)}
        />
      )}
      <div className="plane" aria-hidden="true" hidden={level !== "low"} />
      <main
        id="main"
        className="canvas"
        ref={main}
        tabIndex={-1}
        inert={drawer}
        onScroll={(event) => {
          if (event.target === event.currentTarget)
            setScrolled(event.currentTarget.scrollTop > DIM_AFTER_PX);
        }}
      >
        <Routes>
          <Route path="/" element={<Home {...threshold} />} />
          <Route path="/search" element={gated(<Search />)} />
          <Route
            path="/radio"
            element={gated(<RadioPage onDetails={showStation} />)}
          />
          <Route path="/library/*" element={gated(<LibraryRoutes />)} />
          <Route path="/album/:id" element={gated(<AlbumPage />)} />
          <Route path="/artist/:id" element={gated(<ArtistPage />)} />
          <Route path="/playlists" element={gated(<Playlists />)} />
          <Route path="/playlists/:id" element={gated(<PlaylistPage />)} />
          <Route path="/favorites" element={gated(<Favorites />)} />
          <Route path="/history" element={gated(<History />)} />
          <Route path="/now-playing" element={<NowPlaying />} />
          <Route
            path="*"
            element={
              <section className="page">
                <h1>Page unavailable</h1>
                <NavLink className="quiet" to="/">
                  Return home
                </NavLink>
              </section>
            }
          />
        </Routes>
      </main>
      {pane === "queue" && <QueuePanel onClose={toggleQueue} />}
      {pane === "station" && (
        <StationInspector
          key={station === "new" ? "new" : station.id}
          selection={station}
          onClose={closeStation}
          onSaved={setStation}
        />
      )}
      <Dock
        inert={drawer}
        immersive={immersive}
        queueOpen={pane === "queue"}
        onQueue={toggleQueue}
        onCompact={() => showCompact(true)}
      />
    </div>
  );
}
