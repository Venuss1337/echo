import { useQuery } from "@tanstack/react-query";
import {
  Pause,
  Play,
  Repeat,
  Repeat1,
  Shuffle,
  SkipBack,
  SkipForward,
  Square,
  TriangleAlert,
  Volume1,
  Volume2,
  VolumeX,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useLocation, useNavigate } from "react-router";
import type { PlayerView } from "./generated/contracts";
import { libraryApi } from "./library/api";
import { useLibrary } from "./library/LibraryProvider";
import { describeFault, usePlayer } from "./player";
import { liveLabels } from "./radio/api";
import { presented } from "./shell/chrome";
import { choreograph } from "./shell/motion";
import { IconButton } from "./ui";

export const descriptions = {
  idle: "Ready",
  loading: "Opening track…",
  playing: "Playing",
  paused: "Paused",
  buffering: "Buffering…",
  stopping: "Stopping…",
  ended: "Track ended",
  error: "Track unavailable",
};

/** The loaded queue entry, enriched from the index when it came from there. */
export function useNowTrack() {
  const { view } = usePlayer();
  const { dataKey } = useLibrary();
  const row = view?.tracks.find(({ id }) => id === view.current);
  const trackId = row?.trackId ?? null;
  const detail = useQuery({
    queryKey: ["library", "track", dataKey, trackId],
    queryFn: () => libraryApi.track(trackId as number),
    enabled: trackId != null,
    staleTime: Infinity,
    gcTime: 60_000,
    retry: false,
  }).data;
  if (!row) return null;
  return {
    entryId: row.id,
    trackId: detail?.id ?? null,
    title: row.title,
    artist: row.subtitle ?? "",
    album: detail?.album ?? "",
    albumId: detail?.albumId ?? null,
    artistId: detail?.artistId ?? null,
    artworkId: detail?.artworkId ?? row.artworkId,
    favorite: detail?.favorite ?? false,
    codec: detail?.codec ?? "",
    year: detail?.year ?? null,
  };
}

/** The placeholder identity of whatever is loaded; nothing is the house. */
export function nowSeed(now: ReturnType<typeof useNowTrack>) {
  return now ? `${now.album}\u0001${now.artist}` : "";
}

/** Enters or leaves the exhibition with a shared artwork transition. */
export function useExpand() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  return useCallback(
    (open: boolean) => {
      const active = pathname === "/now-playing";
      if (open === active) return;
      choreograph(
        "expand",
        () => {
          if (open) void navigate("/now-playing");
          else if ((window.history.state?.idx ?? 0) > 0) void navigate(-1);
          else void navigate("/");
        },
        () => (document.querySelector(".immersive") != null) === open,
      );
    },
    [navigate, pathname],
  );
}

/** Play, pause or resume, whichever the confirmed state allows. */
export function usePlayToggle() {
  const { view, act } = usePlayer();
  const live = view?.live;
  const pauseLive =
    live &&
    ["playing", "buffering", "connecting", "reconnecting"].includes(
      live.status,
    );
  const pausing = view?.state === "playing" || view?.state === "buffering";
  const busy = view?.state === "loading" || view?.state === "stopping";
  const start =
    view?.tracks.find(({ id }) => id === view.current) ?? view?.tracks[0];
  const resumable = view?.state === "paused";
  if (live)
    return {
      pausing: Boolean(pauseLive),
      disabled: live.status === "unsupported" || live.status === "rejected",
      toggle: () => act({ kind: pauseLive ? "pause" : "resume" }),
    };
  return {
    pausing,
    disabled: !view || busy || (!pausing && !resumable && !start),
    toggle: () => {
      if (pausing) act({ kind: "pause" });
      else if (resumable) act({ kind: "resume" });
      else if (start) act({ kind: "play", id: start.id });
    },
  };
}

export function TransportButtons() {
  const { view, act } = usePlayer();
  const { pausing, disabled, toggle } = usePlayToggle();
  if (!view) return null;
  if (view.live)
    return (
      <div className="transport-buttons live-transport">
        <IconButton
          primary
          label={pausing ? "Pause station" : "Reconnect to live radio"}
          disabled={disabled}
          onClick={toggle}
        >
          {pausing ? (
            <Pause aria-hidden="true" className="solid" />
          ) : (
            <Play aria-hidden="true" className="solid" />
          )}
        </IconButton>
        <IconButton label="Stop station" onClick={() => act({ kind: "stop" })}>
          <Square aria-hidden="true" className="solid" />
        </IconButton>
      </div>
    );
  const busy = view.state === "loading" || view.state === "stopping";
  const repeat =
    view.repeat === "off" ? "all" : view.repeat === "all" ? "one" : "off";
  return (
    <div className="transport-buttons">
      <IconButton
        label="Shuffle"
        className="mode"
        pressed={view.shuffled}
        onClick={() => act({ kind: "shuffle", enabled: !view.shuffled })}
      >
        <Shuffle aria-hidden="true" />
      </IconButton>
      <IconButton
        label="Previous track"
        disabled={!view.hasPrevious}
        onClick={() => act({ kind: "previous" })}
      >
        <SkipBack aria-hidden="true" className="solid" />
      </IconButton>
      <IconButton
        primary
        label={pausing ? "Pause" : "Play"}
        disabled={disabled}
        onClick={toggle}
      >
        {pausing ? (
          <Pause aria-hidden="true" className="solid" />
        ) : (
          <Play aria-hidden="true" className="solid" />
        )}
      </IconButton>
      <IconButton
        label="Next track"
        disabled={!view.hasNext || busy}
        onClick={() => act({ kind: "next" })}
      >
        <SkipForward aria-hidden="true" className="solid" />
      </IconButton>
      <IconButton
        label={`Repeat ${view.repeat}. Set repeat ${repeat}`}
        className="mode"
        pressed={view.repeat !== "off"}
        onClick={() => act({ kind: "repeat", mode: repeat })}
      >
        {view.repeat === "one" ? (
          <Repeat1 aria-hidden="true" />
        ) : (
          <Repeat aria-hidden="true" />
        )}
      </IconButton>
    </div>
  );
}

export function VolumeControl() {
  const { view, act } = usePlayer();
  const commit = useCallback(
    (value: number) => act({ kind: "volume", value }),
    [act],
  );
  if (!view) return null;
  const Icon =
    view.muted || view.volume === 0
      ? VolumeX
      : view.volume < 45
        ? Volume1
        : Volume2;
  return (
    <div className="volume">
      <IconButton
        label={view.muted ? "Unmute" : "Mute"}
        pressed={view.muted}
        onClick={() => act({ kind: "mute", enabled: !view.muted })}
      >
        <Icon aria-hidden="true" />
      </IconButton>
      <CommitSlider
        label="Volume"
        value={view.volume}
        maximum={100}
        onCommit={commit}
      />
    </div>
  );
}

export function Fault() {
  const { view, fault } = usePlayer();
  if (!fault && (view?.state !== "error" || view.live)) return null;
  return (
    <p className="transport-error" role="alert">
      <TriangleAlert aria-hidden="true" />{" "}
      <span>{fault ?? describeFault(view?.failure ?? "decodeFailed")}</span>
    </p>
  );
}

export function LiveLine({ live }: { live: NonNullable<PlayerView["live"]> }) {
  return (
    <div className="timeline live-line">
      <span className="live-mark" data-playing={live.status === "playing"}>
        LIVE
      </span>
      <span role="status">{liveLabels[live.status]}</span>
    </div>
  );
}

export function formatTime(milliseconds: number) {
  const seconds = Math.floor(milliseconds / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** The groove: the whole track as one line, with the needle where it is. */
export function Timeline({ view }: { view: PlayerView }) {
  const { act } = usePlayer();
  const [sample, setSample] = useState<{
    revision: string;
    position: number;
  }>();
  const duration = view.durationMs;
  const { generation, positionMs, revision, state } = view;
  useEffect(() => {
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    if (duration == null || state !== "playing" || motion.matches) return;
    const anchor = performance.now();
    const timer = setInterval(() => {
      if (!motion.matches && presented()) {
        setSample({
          revision,
          position: Math.min(
            duration,
            positionMs + Math.min(750, performance.now() - anchor),
          ),
        });
      }
    }, 120);
    return () => clearInterval(timer);
  }, [duration, positionMs, revision, state]);
  const position =
    state === "playing" && sample?.revision === revision
      ? sample.position
      : positionMs;
  const seek = useCallback(
    (milliseconds: number) => act({ kind: "seek", generation, milliseconds }),
    [act, generation],
  );
  return (
    <div className="timeline">
      <span>{formatTime(position)}</span>
      <CommitSlider
        label="Seek"
        value={position}
        maximum={duration ?? 0}
        step={1000}
        disabled={!view.seekable || duration == null || state === "stopping"}
        preview={formatTime}
        onCommit={seek}
      />
      <span>{duration == null ? "–:––" : formatTime(duration)}</span>
    </div>
  );
}

function CommitSlider({
  label,
  value,
  maximum,
  step = 1,
  disabled,
  preview,
  onCommit,
}: {
  label: string;
  value: number;
  maximum: number;
  step?: number;
  disabled?: boolean;
  /** Names the value under the pointer before it is committed. */
  preview?: (value: number) => string;
  onCommit: (value: number) => void;
}) {
  // Pointer drags may emit a change every frame. Commit once on release,
  // keyboard input on keyup, and assisted input after an idle interval.
  const [draft, setDraft] = useState<number | null>(null);
  const [dragging, setDragging] = useState(false);
  const [hover, setHover] = useState<number | null>(null);
  useEffect(() => {
    if (draft == null || dragging || disabled) return;
    const timer = setTimeout(() => {
      onCommit(draft);
      setDraft(null);
    }, 180);
    return () => clearTimeout(timer);
  }, [disabled, draft, dragging, onCommit]);
  function flush() {
    setDragging(false);
    if (draft != null && !disabled) onCommit(draft);
    setDraft(null);
  }
  const top = Math.max(1, Math.floor(maximum));
  const shown = Math.min(maximum, draft ?? Math.floor(value));
  // The browser snaps the needle to whole steps, so the fill must as well.
  const needle = Math.min(top, Math.round(shown / step) * step);
  const named = dragging ? shown / top : hover;
  return (
    <span className="slider">
      <input
        type="range"
        className="transport-slider"
        aria-label={label}
        aria-valuetext={preview ? preview(shown) : undefined}
        min={0}
        max={top}
        step={step}
        value={shown}
        style={
          { "--filled": `${(needle / top) * 100}%` } as React.CSSProperties
        }
        disabled={disabled}
        onChange={(event) => setDraft(event.currentTarget.valueAsNumber)}
        onPointerDown={() => setDragging(true)}
        onPointerUp={flush}
        onPointerMove={(event) => {
          if (!preview || event.pointerType !== "mouse") return;
          const box = event.currentTarget.getBoundingClientRect();
          setHover(
            Math.min(1, Math.max(0, (event.clientX - box.left) / box.width)),
          );
        }}
        onPointerLeave={() => setHover(null)}
        onPointerCancel={() => {
          setDragging(false);
          setDraft(null);
        }}
        onBlur={flush}
        onKeyUp={flush}
      />
      {preview && named != null && !disabled && (
        <output
          className="slider-preview"
          aria-hidden="true"
          style={{ left: `${named * 100}%` }}
        >
          {preview(named * maximum)}
        </output>
      )}
    </span>
  );
}
