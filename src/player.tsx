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
import type {
  PickAction,
  PlayerIntent,
  PlayerView,
  PlayTarget,
  QueueMode,
} from "./generated/contracts";

const faults: Record<string, string> = {
  busy: "The player is busy. Try again in a moment.",
  invalid: "That action is unavailable for the current track.",
  unavailable: "Local audio is unavailable in this build or installation.",
  fileUnavailable: "The selected file is missing or cannot be opened locally.",
  decodeFailed:
    "ECHO couldn’t start audio. Try a different file or output device.",
  outputUnavailable:
    "Windows audio output is unavailable. Connect an output device, then press Play to retry.",
  pickerUnavailable: "Windows couldn’t open the file picker.",
  nothingPlayable:
    "None of those files can be played right now. Check that the drive is connected.",
  notFound: "That item is no longer in your library.",
  playbackDisabled: "Station playback is unavailable in this build.",
  storageFailed: "ECHO couldn’t read its library database.",
};

export function describeFault(error: unknown) {
  return typeof error === "string" &&
    Object.prototype.hasOwnProperty.call(faults, error)
    ? faults[error]
    : "ECHO couldn’t contact the audio service.";
}

interface PlayerBinding {
  native: boolean;
  view: PlayerView | undefined;
  connecting: boolean;
  fault: string | null;
  reconnect: () => void;
  act: (intent: PlayerIntent) => void;
  pick: (action: PickAction) => void;
  /** Plays or queues indexed tracks. Rust resolves the files from its index. */
  playLibrary: (target: PlayTarget, mode: QueueMode, shuffle?: boolean) => void;
  playStation: (id: string) => void;
}

const PlayerContext = createContext<PlayerBinding | null>(null);

export function newerView(
  previous: PlayerView | undefined,
  incoming: PlayerView,
) {
  if (!previous) return incoming;
  if (BigInt(incoming.revision) <= BigInt(previous.revision)) return previous;
  const sameQueue =
    incoming.tracks.length === previous.tracks.length &&
    incoming.tracks.every(
      (row, index) =>
        row.id === previous.tracks[index].id &&
        row.title === previous.tracks[index].title &&
        row.subtitle === previous.tracks[index].subtitle &&
        row.artworkId === previous.tracks[index].artworkId &&
        row.unavailable === previous.tracks[index].unavailable,
    );
  return sameQueue ? { ...incoming, tracks: previous.tracks } : incoming;
}

export function PlayerProvider({ children }: { children: ReactNode }) {
  const native = isTauri();
  const [view, setView] = useState<PlayerView>();
  const [fault, setFault] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(native);
  const [attempt, setAttempt] = useState(0);
  const accept = useCallback((incoming: PlayerView) => {
    setView((previous) => newerView(previous, incoming));
  }, []);
  const report = useCallback((error: unknown) => {
    setFault(describeFault(error));
  }, []);
  useEffect(() => {
    if (!native) return;
    let cancelled = false;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let channel: Channel<PlayerView> | undefined;
    let retries = 0;
    async function connect() {
      try {
        channel = new Channel<PlayerView>((incoming) => {
          if (!cancelled) accept(incoming);
        });
        await invoke<PlayerView>("player_subscribe", { channel });
        const snapshot = await invoke<PlayerView>("player_snapshot");
        if (!cancelled) {
          accept(snapshot);
          setConnecting(false);
        }
      } catch (error) {
        if (cancelled) return;
        if ((error === "opening" || error === "busy") && retries++ < 30) {
          retry = setTimeout(() => void connect(), 300);
        } else {
          report(error);
          setConnecting(false);
        }
      }
    }
    function refresh() {
      void invoke<PlayerView>("player_snapshot").then(
        (snapshot) => {
          if (!cancelled) accept(snapshot);
        },
        () => undefined,
      );
    }
    void connect();
    window.addEventListener("focus", refresh);
    return () => {
      cancelled = true;
      clearTimeout(retry);
      window.removeEventListener("focus", refresh);
      if (channel) channel.onmessage = () => undefined;
    };
  }, [accept, attempt, native, report]);

  const command = useCallback(
    (name: string, args: Record<string, unknown>) => {
      setFault(null);
      void invoke<PlayerView>(name, args).then(accept, report);
    },
    [accept, report],
  );
  const act = useCallback(
    (intent: PlayerIntent) => command("player_act", { intent }),
    [command],
  );
  const pick = useCallback(
    (action: PickAction) => command("player_pick", { action }),
    [command],
  );
  const playLibrary = useCallback(
    (target: PlayTarget, mode: QueueMode, shuffle = false) =>
      command("library_play", { target, mode, shuffle }),
    [command],
  );
  const playStation = useCallback(
    (id: string) => command("radio_play", { id }),
    [command],
  );
  const value = useMemo<PlayerBinding>(
    () => ({
      native,
      view,
      connecting,
      fault,
      reconnect: () => {
        setFault(null);
        setConnecting(true);
        setAttempt((previous) => previous + 1);
      },
      act,
      pick,
      playLibrary,
      playStation,
    }),
    [act, connecting, fault, native, pick, playLibrary, playStation, view],
  );
  return <PlayerContext value={value}>{children}</PlayerContext>;
}

export function usePlayer() {
  const player = useContext(PlayerContext);
  if (!player) throw new Error("PlayerProvider is missing");
  return player;
}
