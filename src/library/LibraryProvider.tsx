import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import type { LibrarySummary } from "../generated/contracts";
import { describeLibraryFault } from "./api";

type Busy = "add" | "remove" | "scan" | null;

interface LibraryBinding {
  native: boolean;
  summary: LibrarySummary | undefined;
  /** Changes whenever indexed rows may have changed, so pages refetch. */
  dataKey: string;
  connecting: boolean;
  fault: string | null;
  busy: Busy;
  addRoot: () => void;
  removeRoot: (id: number) => void;
  scan: (root?: number) => void;
  cancelScan: () => void;
  dismissFault: () => void;
}

const LibraryContext = createContext<LibraryBinding | null>(null);

/** The IPC boundary is untrusted input: accept only a well-formed summary. */
export function isSummary(value: unknown): value is LibrarySummary {
  if (typeof value !== "object" || value === null) return false;
  const summary = value as Record<string, unknown>;
  return (
    typeof summary.revision === "string" &&
    /^\d{1,20}$/.test(summary.revision) &&
    typeof summary.userRevision === "string" &&
    Array.isArray(summary.roots) &&
    ["tracks", "albums", "artists", "unavailableTracks"].every(
      (field) => typeof summary[field] === "number",
    )
  );
}

/** Ignores summaries older than the one already shown. */
export function newerSummary(
  previous: LibrarySummary | undefined,
  incoming: LibrarySummary,
) {
  if (!previous) return incoming;
  return BigInt(incoming.revision) > BigInt(previous.revision)
    ? incoming
    : previous;
}

export function dataKeyOf(summary: LibrarySummary | undefined) {
  if (!summary) return "none";
  const { tracks, albums, artists, unavailableTracks, roots } = summary;
  // A finished scan may have re-read tags without changing any count.
  const scans = roots.map(
    ({ id, lastScan }) =>
      `${id}.${lastScan?.indexed ? lastScan.finishedAt : 0}`,
  );
  return `${tracks}:${albums}:${artists}:${unavailableTracks}:${scans.join()}`;
}

export function LibraryProvider({ children }: { children: ReactNode }) {
  const native = isTauri();
  const [summary, setSummary] = useState<LibrarySummary>();
  const [fault, setFault] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(native);
  const [busy, setBusy] = useState<Busy>(null);

  const accept = useCallback((incoming: unknown) => {
    if (isSummary(incoming)) {
      setSummary((previous) => newerSummary(previous, incoming));
    }
  }, []);

  useEffect(() => {
    if (!native) return;
    let cancelled = false;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let channel: Channel<LibrarySummary> | undefined;
    let retries = 0;
    async function connect() {
      try {
        channel = new Channel<LibrarySummary>((incoming) => {
          if (!cancelled) accept(incoming);
        });
        const first = await invoke<unknown>("library_subscribe", {
          channel,
        });
        if (!isSummary(first)) throw new Error("Unexpected library summary");
        if (!cancelled) {
          accept(first);
          setConnecting(false);
        }
      } catch (error) {
        if (cancelled) return;
        if ((error === "opening" || error === "busy") && retries++ < 30) {
          retry = setTimeout(() => void connect(), 300);
        } else {
          // Leave `summary` unset: pages fall back to the status panel.
          setConnecting(false);
        }
      }
    }
    void connect();
    return () => {
      cancelled = true;
      clearTimeout(retry);
      if (channel) channel.onmessage = () => undefined;
    };
  }, [accept, native]);

  const run = useCallback(
    (
      kind: Exclude<Busy, null>,
      command: string,
      args?: Record<string, unknown>,
    ) => {
      setFault(null);
      setBusy(kind);
      invoke<unknown>(command, args).then(
        (result) => {
          accept(result);
          setBusy(null);
        },
        (error) => {
          setFault(describeLibraryFault(error));
          setBusy(null);
        },
      );
    },
    [accept],
  );

  const value = useMemo<LibraryBinding>(
    () => ({
      native,
      summary,
      dataKey: dataKeyOf(summary),
      connecting,
      fault,
      busy,
      addRoot: () => run("add", "library_add_root"),
      removeRoot: (id) => run("remove", "library_remove_root", { id }),
      scan: (root) => run("scan", "library_scan", { root: root ?? null }),
      cancelScan: () => run("scan", "library_cancel_scan"),
      dismissFault: () => setFault(null),
    }),
    [busy, connecting, fault, native, run, summary],
  );
  return <LibraryContext value={value}>{children}</LibraryContext>;
}

export function useLibrary() {
  const library = useContext(LibraryContext);
  if (!library) throw new Error("LibraryProvider is missing");
  return library;
}
