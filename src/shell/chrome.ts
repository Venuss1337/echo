import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import type {
  CaptionBounds,
  WindowIntent,
  WindowView,
} from "../generated/contracts";

const RESTORED: WindowView = {
  maximized: false,
  minimized: false,
  maximizeHovered: false,
  maximizePressed: false,
};

let minimized = false;
let mini = false;
const miniListeners = new Set<() => void>();

/** Mirrors the native mini mode using the existing caption subscription. */
export function useMiniWindow() {
  return useSyncExternalStore(
    (notify) => {
      miniListeners.add(notify);
      return () => {
        miniListeners.delete(notify);
      };
    },
    () => mini,
  );
}

export function returnFromMini() {
  void invoke("window_act", { intent: "returnFromMini" }).catch(
    () => undefined,
  );
}

/**
 * Whether anyone could see the page. WebView2 does not report a minimized
 * window as hidden, so the window itself says so.
 */
export function presented() {
  return !document.hidden && !minimized;
}

/** The desktop window around the page. In a browser there is none to control. */
export function useWindowChrome() {
  const native = isTauri();
  const [view, setView] = useState(RESTORED);
  useEffect(() => {
    if (!native) return;
    let live = true;
    const accept = (next: WindowView) => {
      if (!live) return;
      minimized = next.minimized;
      const nextMini = next.mini === true;
      if (mini !== nextMini) {
        mini = nextMini;
        for (const notify of miniListeners) notify();
      }
      setView(next);
    };
    const channel = new Channel<WindowView>(accept);
    invoke<WindowView | undefined>("window_subscribe", { channel }).then(
      (next) => next && accept(next),
      () => undefined,
    );
    return () => {
      live = false;
      minimized = false;
    };
  }, [native]);
  const act = useCallback((intent: WindowIntent) => {
    void invoke("window_act", { intent }).catch(() => undefined);
  }, []);
  return { native, view, act };
}

/** Tells Rust where the maximize button is drawn, for Snap Layouts. */
export function reportCaption(bounds: CaptionBounds | null) {
  void invoke("window_caption", { bounds }).catch(() => undefined);
}

function onFocusChange(notify: () => void) {
  window.addEventListener("focus", notify);
  window.addEventListener("blur", notify);
  return () => {
    window.removeEventListener("focus", notify);
    window.removeEventListener("blur", notify);
  };
}

/** Whether this window is the active one, as Windows captions show. */
export function useWindowActive() {
  return useSyncExternalStore(onFocusChange, () => document.hasFocus());
}

/** A live media query that also works where matchMedia is absent. */
export function useMedia(query: string) {
  const subscribe = useCallback(
    (notify: () => void) => {
      if (typeof matchMedia !== "function") return () => undefined;
      const media = matchMedia(query);
      media.addEventListener("change", notify);
      return () => media.removeEventListener("change", notify);
    },
    [query],
  );
  return useSyncExternalStore(
    subscribe,
    () => typeof matchMedia === "function" && matchMedia(query).matches,
  );
}
