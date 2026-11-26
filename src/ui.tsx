import type { LucideIcon } from "lucide-react";
import { createPortal } from "react-dom";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";

export function IconButton({
  label,
  children,
  disabled,
  pressed,
  primary,
  className = "",
  expanded,
  onClick,
}: {
  label: string;
  children: ReactNode;
  disabled?: boolean;
  pressed?: boolean;
  primary?: boolean;
  className?: string;
  expanded?: boolean;
  onClick: (event: React.MouseEvent<HTMLButtonElement>) => void;
}) {
  return (
    <button
      className={`icon-button ${primary ? "transport-primary" : ""} ${className}`}
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={pressed}
      aria-expanded={expanded}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

export interface MenuItem {
  label: string;
  Icon?: LucideIcon;
  disabled?: boolean;
  danger?: boolean;
  onSelect: () => void;
}

interface MenuState {
  label: string;
  x: number;
  y: number;
  /** Top of the control that opened it, or the pointer. */
  top: number;
  items: MenuItem[];
  origin: HTMLElement | null;
}

interface Surface {
  openMenu: (
    event: React.MouseEvent,
    label: string,
    items: (MenuItem | false | null | undefined)[],
  ) => void;
  notify: (message: string) => void;
}

const SurfaceContext = createContext<Surface | null>(null);

/** Owns the one open menu and the one transient message. */
export function SurfaceProvider({ children }: { children: ReactNode }) {
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [message, setMessage] = useState<{ text: string; id: number }>();
  const openMenu = useCallback<Surface["openMenu"]>((event, label, items) => {
    event.preventDefault();
    event.stopPropagation();
    const target = event.currentTarget as HTMLElement;
    const box = target.getBoundingClientRect();
    // Keyboard and button activation anchor to the control, not a pointer.
    const pointer = event.type === "contextmenu" && event.clientX > 0;
    setMenu({
      label,
      items: items.filter((item): item is MenuItem => Boolean(item)),
      x: pointer ? event.clientX : box.left,
      y: pointer ? event.clientY : box.bottom + 4,
      top: pointer ? event.clientY : box.top,
      origin:
        document.activeElement instanceof HTMLElement &&
        document.activeElement !== document.body
          ? document.activeElement
          : target,
    });
  }, []);
  const notify = useCallback(
    (text: string) => setMessage({ text, id: Date.now() }),
    [],
  );
  useEffect(() => {
    if (!message) return;
    const timer = setTimeout(() => setMessage(undefined), 4200);
    return () => clearTimeout(timer);
  }, [message]);
  const value = useMemo(() => ({ openMenu, notify }), [notify, openMenu]);
  return (
    <SurfaceContext value={value}>
      {children}
      {menu && <Menu state={menu} onClose={() => setMenu(null)} />}
      <div className="toast-region" role="status" aria-live="polite">
        {message && (
          <p key={message.id} className="toast">
            {message.text}
          </p>
        )}
      </div>
    </SurfaceContext>
  );
}

export function useSurface() {
  const surface = useContext(SurfaceContext);
  if (!surface) throw new Error("SurfaceProvider is missing");
  return surface;
}

function Menu({ state, onClose }: { state: MenuState; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const menu = ref.current;
    if (!menu) return;
    const { width, height } = menu.getBoundingClientRect();
    const left = Math.max(8, Math.min(state.x, innerWidth - width - 8));
    const below = Math.max(8, Math.min(state.y, innerHeight - height - 8));
    // Near the foot of the window a menu opens upwards from its control.
    const above = state.top - height - 4;
    const top = below < state.y && above >= 8 ? above : below;
    menu.style.left = `${left}px`;
    menu.style.top = `${top}px`;
    // It grows out of the control or pointer that asked for it.
    menu.style.transformOrigin = `${state.x - left}px ${
      top === above ? height : Math.max(0, state.y - top)
    }px`;
    menu.querySelector<HTMLElement>("button:enabled")?.focus();
  }, [state]);
  useEffect(() => {
    const away = (event: Event) => {
      if (!ref.current?.contains(event.target as Node)) onClose();
    };
    // Scrolling moves the anchor, so the menu would point at the wrong row.
    document.addEventListener("pointerdown", away, true);
    document.addEventListener("wheel", away, { passive: true });
    window.addEventListener("resize", onClose);
    return () => {
      document.removeEventListener("pointerdown", away, true);
      document.removeEventListener("wheel", away);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);
  function close() {
    onClose();
    if (state.origin?.isConnected) state.origin.focus({ preventScroll: true });
  }
  function step(direction: number, edge?: "first" | "last") {
    const buttons = [
      ...(ref.current?.querySelectorAll<HTMLElement>("button:enabled") ?? []),
    ];
    if (!buttons.length) return;
    const current = buttons.indexOf(document.activeElement as HTMLElement);
    const index =
      edge === "first"
        ? 0
        : edge === "last"
          ? buttons.length - 1
          : (current + direction + buttons.length) % buttons.length;
    buttons[index].focus();
  }
  return (
    <>
      {/* The caption drags the window and would swallow the dismissing press. */}
      <div className="menu-scrim" />
      <div
        ref={ref}
        className="menu"
        role="menu"
        aria-label={state.label}
        onKeyDown={(event) => {
          event.stopPropagation();
          if (event.key === "Escape" || event.key === "Tab") {
            event.preventDefault();
            close();
          } else if (event.key === "ArrowDown") {
            event.preventDefault();
            step(1);
          } else if (event.key === "ArrowUp") {
            event.preventDefault();
            step(-1);
          } else if (event.key === "Home") step(0, "first");
          else if (event.key === "End") step(0, "last");
        }}
      >
        {state.items.map(({ label, Icon, disabled, danger, onSelect }) => (
          <button
            key={label}
            role="menuitem"
            type="button"
            className={danger ? "danger" : undefined}
            disabled={disabled}
            onClick={() => {
              close();
              onSelect();
            }}
          >
            {Icon && <Icon aria-hidden="true" />} {label}
          </button>
        ))}
      </div>
    </>
  );
}

/** Open modals. The shell stays inert until the last one closes. */
let modals = 0;

function holdShell() {
  const shell = document.querySelector<HTMLElement>(".shell, .compact-stage");
  modals += 1;
  if (shell) shell.inert = true;
  return () => {
    modals -= 1;
    if (shell && modals === 0) shell.inert = false;
  };
}

const FOCUSABLE =
  "button:enabled, input:enabled, textarea:enabled, select:enabled, a[href]";

/**
 * A modal that traps focus, closes on Escape and restores focus. It is not a
 * native `showModal` dialog: that would make the window's own minimize,
 * maximize and close buttons inert along with the page.
 */
export function Dialog({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  // Read while rendering: an autofocused field moves focus before effects run.
  const [origin] = useState(() => document.activeElement);
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (!dialog.contains(document.activeElement)) {
      (dialog.querySelector<HTMLElement>(FOCUSABLE) ?? dialog).focus();
    }
    const release = holdShell();
    return () => {
      release();
      if (origin instanceof HTMLElement && origin.isConnected) origin.focus();
    };
  }, [origin]);
  function trap(event: React.KeyboardEvent) {
    event.stopPropagation();
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    }
    if (event.key !== "Tab") return;
    const stops = [
      ...(ref.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? []),
    ];
    const first = stops[0];
    const last = stops[stops.length - 1];
    const at = document.activeElement;
    if (event.shiftKey ? at === first || at === ref.current : at === last) {
      event.preventDefault();
      (event.shiftKey ? last : first)?.focus();
    }
  }
  return createPortal(
    <div
      className="dialog-layer"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        ref={ref}
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        onKeyDown={trap}
      >
        <h2>{title}</h2>
        {children}
      </div>
    </div>,
    document.body,
  );
}

// ---------------------------------------------------------------------------
// Reordering
// ---------------------------------------------------------------------------

interface DragState {
  id: string;
  /** Row to insert before, or null for the end of the list. */
  before: string | null;
}

/**
 * Pointer reordering by stable id. Rows carry `data-reorder-id`. The list may
 * change mid-drag: nothing is mutated until release, and a row that vanished
 * cancels the move. Rust makes the final decision.
 */
export function useReorder(
  container: RefObject<HTMLElement | null>,
  onMove: (id: string, before: string | null) => void,
) {
  const [drag, setDrag] = useState<DragState | null>(null);
  const live = useRef<DragState | null>(null);
  const start = useCallback(
    (id: string, event: React.PointerEvent<HTMLElement>) => {
      if (event.button !== 0) return;
      event.preventDefault();
      const handle = event.currentTarget;
      handle.setPointerCapture(event.pointerId);
      const update = (next: DragState | null) => {
        live.current = next;
        setDrag(next);
      };
      update({ id, before: id });
      const rows = () => [
        ...(container.current?.querySelectorAll<HTMLElement>(
          "[data-reorder-id]",
        ) ?? []),
      ];
      const move = (moving: PointerEvent) => {
        const scroller = container.current;
        if (scroller) {
          const box = scroller.getBoundingClientRect();
          // The dock floats over the end of the list, so scroll above it.
          const covered =
            parseFloat(getComputedStyle(scroller).scrollPaddingBottom) || 0;
          if (moving.clientY < box.top + 36) scroller.scrollTop -= 12;
          if (moving.clientY > box.bottom - covered - 36)
            scroller.scrollTop += 12;
        }
        const candidates = rows();
        const next = candidates.find((row) => {
          const box = row.getBoundingClientRect();
          return moving.clientY < box.top + box.height / 2;
        });
        const last = candidates[candidates.length - 1];
        // Past the last mounted row of a longer list is not the list's end.
        const atEnd = last?.dataset.reorderLast === "true";
        update({
          id,
          before:
            next?.dataset.reorderId ??
            (atEnd ? null : (last?.dataset.reorderId ?? null)),
        });
      };
      const finish = (commit: boolean) => {
        handle.removeEventListener("pointermove", move);
        handle.removeEventListener("pointerup", drop);
        handle.removeEventListener("pointercancel", cancel);
        window.removeEventListener("keydown", escape, true);
        const result = live.current;
        update(null);
        if (!commit || !result) return;
        const order = rows().map((row) => row.dataset.reorderId);
        const at = order.indexOf(result.id);
        const unchanged =
          result.before === result.id ||
          (at >= 0 && order[at + 1] === (result.before ?? undefined));
        if (!unchanged) onMove(result.id, result.before);
      };
      const drop = () => finish(true);
      const cancel = () => finish(false);
      const escape = (key: KeyboardEvent) => {
        if (key.key === "Escape") {
          key.stopPropagation();
          finish(false);
        }
      };
      handle.addEventListener("pointermove", move);
      handle.addEventListener("pointerup", drop);
      handle.addEventListener("pointercancel", cancel);
      window.addEventListener("keydown", escape, true);
    },
    [container, onMove],
  );
  const rowClass = useCallback(
    (id: string, last: boolean) =>
      drag == null
        ? ""
        : drag.id === id
          ? "dragging"
          : drag.before === id
            ? "drop-before"
            : drag.before === null && last
              ? "drop-after"
              : "",
    [drag],
  );
  return { start, rowClass, active: drag != null };
}
