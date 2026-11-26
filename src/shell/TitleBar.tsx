import {
  ChevronLeft,
  ChevronRight,
  PanelLeft,
  Search as SearchIcon,
} from "lucide-react";
import { useEffect, useRef, useState, type RefObject } from "react";
import {
  Link,
  useLocation,
  useNavigate,
  useNavigationType,
  useSearchParams,
} from "react-router";
import { IconButton } from "../ui";
import { reportCaption, useWindowActive, useWindowChrome } from "./chrome";
import { EchoMark } from "./EchoMark";

/** Where this entry sits in the session's own history. */
function useTrail() {
  const location = useLocation();
  const type = useNavigationType();
  const index = Number(window.history.state?.idx ?? 0);
  const [trail, setTrail] = useState({ key: location.key, end: index });
  if (trail.key !== location.key) {
    setTrail({
      key: location.key,
      // Pushing discards whatever was ahead.
      end: type === "PUSH" ? index : Math.max(trail.end, index),
    });
  }
  return { back: index > 0, forward: index < trail.end };
}

/**
 * The one search field. It lives in the title bar so it is always in the same
 * place, and it owns the query text of the Search page.
 */
export function SearchField({
  field,
}: {
  field: RefObject<HTMLInputElement | null>;
}) {
  const { pathname } = useLocation();
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const searching = pathname === "/search";
  const text = searching ? (params.get("q") ?? "").slice(0, 200) : "";
  const [draft, setDraft] = useState(text);
  const [seen, setSeen] = useState(text);
  // Back, forward and leaving the page change the text underneath the field.
  if (seen !== text) {
    setSeen(text);
    setDraft(text);
  }
  useEffect(() => {
    const next = draft.trim();
    if (next === text.trim() || (!searching && !next)) return;
    const timer = setTimeout(
      () =>
        void navigate(
          next ? `/search?q=${encodeURIComponent(draft)}` : "/search",
          { replace: searching },
        ),
      160,
    );
    return () => clearTimeout(timer);
  }, [draft, navigate, searching, text]);
  useEffect(() => {
    // Arriving on the page without a query means "let me type".
    if (searching && !field.current?.value) field.current?.focus();
  }, [field, searching]);
  return (
    <form
      className="search-field"
      role="search"
      onSubmit={(event) => event.preventDefault()}
    >
      <SearchIcon aria-hidden="true" />
      <input
        ref={field}
        type="search"
        aria-label="Search library"
        placeholder="Search your library"
        autoComplete="off"
        spellCheck={false}
        maxLength={200}
        value={draft}
        onChange={(event) => setDraft(event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key !== "Escape") return;
          event.stopPropagation();
          if (draft) setDraft("");
          else event.currentTarget.blur();
        }}
      />
      <kbd aria-hidden="true">/</kbd>
    </form>
  );
}

export function TitleBar({
  search,
  immersive,
  collapsed,
  inert,
  onSidebar,
  onLeave,
}: {
  search: RefObject<HTMLInputElement | null>;
  immersive: boolean;
  collapsed: boolean;
  /** Behind a modal surface. */
  inert: boolean;
  onSidebar: () => void;
  onLeave: () => void;
}) {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const trail = useTrail();
  return (
    <header
      className="titlebar"
      data-searching={pathname === "/search"}
      inert={inert}
    >
      {immersive ? (
        <IconButton label="Close Now Playing" onClick={onLeave}>
          <ChevronLeft aria-hidden="true" />
        </IconButton>
      ) : (
        <div className="titlebar-trail">
          <IconButton
            label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
            className="sidebar-toggle"
            expanded={!collapsed}
            onClick={onSidebar}
          >
            <PanelLeft aria-hidden="true" />
          </IconButton>
          <IconButton
            label="Back"
            disabled={!trail.back}
            onClick={() => void navigate(-1)}
          >
            <ChevronLeft aria-hidden="true" />
          </IconButton>
          <IconButton
            label="Forward"
            className="forward"
            disabled={!trail.forward}
            onClick={() => void navigate(1)}
          >
            <ChevronRight aria-hidden="true" />
          </IconButton>
        </div>
      )}
      {immersive ? (
        <Link className="wordmark" to="/" aria-label="ECHO home">
          <EchoMark /> <span>ECHO</span>
        </Link>
      ) : (
        <>
          <SearchField field={search} />
          <Link className="icon-button search-link" to="/search" title="Search">
            <SearchIcon aria-label="Search" />
          </Link>
        </>
      )}
    </header>
  );
}

/** Windows' own caption glyphs, drawn at 10 px on the pixel grid. */
function Glyph({ d }: { d: string }) {
  return (
    <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
      <path d={d} />
    </svg>
  );
}

/**
 * Minimize, maximize and close. The maximize button is also covered by a
 * native hit target (see `window.rs`), which is where its hover comes from.
 */
export function Caption() {
  const { native, view, act } = useWindowChrome();
  const active = useWindowActive();
  const maximize = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const button = maximize.current;
    if (!native || !button) return;
    const report = () => {
      const box = button.getBoundingClientRect();
      reportCaption(
        box.width > 0
          ? {
              right: document.documentElement.clientWidth - box.right,
              top: box.top,
              width: box.width,
              height: box.height,
            }
          : null,
      );
    };
    // Offsets are from the right edge, so only the button's own size matters.
    const observer = new ResizeObserver(report);
    observer.observe(button);
    return () => {
      observer.disconnect();
      reportCaption(null);
    };
  }, [native]);
  if (!native) return null;
  return (
    <div
      className={active ? "caption" : "caption inactive"}
      role="group"
      aria-label="Window"
    >
      <button
        type="button"
        aria-label="Minimize"
        onClick={() => act("minimize")}
      >
        <Glyph d="M0 5.5h10" />
      </button>
      <button
        ref={maximize}
        type="button"
        aria-label={view.maximized ? "Restore" : "Maximize"}
        data-hovered={view.maximizeHovered || undefined}
        data-pressed={view.maximizePressed || undefined}
        onClick={() => act("toggleMaximize")}
      >
        {view.maximized ? (
          <Glyph d="M.5 2.5h7v7h-7zM2.5 2.5v-1a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-1" />
        ) : (
          <Glyph d="M1.5.5h7a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1h-7a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1z" />
        )}
      </button>
      <button
        type="button"
        className="close"
        aria-label="Close"
        onClick={() => act("close")}
      >
        <Glyph d="M.5.5l9 9M9.5.5l-9 9" />
      </button>
    </div>
  );
}
