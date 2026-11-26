import {
  Ellipsis,
  FolderOpen,
  ListVideo,
  Maximize2,
  Minimize2,
  RectangleVertical,
  Square,
} from "lucide-react";
import { Link } from "react-router";
import { Sleeve } from "./library/Artwork";
import { artistPath } from "./library/rows";
import { HeartButton } from "./library/UserData";
import { usePlayer } from "./player";
import { stationLight, StationFavorite } from "./radio/Radio";
import { useLightTokens } from "./shell/Field";
import {
  Fault,
  LiveLine,
  Timeline,
  TransportButtons,
  VolumeControl,
  descriptions,
  nowSeed,
  useExpand,
  useNowTrack,
} from "./transport";
import { IconButton, useSurface } from "./ui";

/**
 * The playback dock. The record stands in it, taller than the surface that
 * holds it, and the whole track runs along its foot as one groove.
 *
 * It carries the light of what is playing, whatever the room is lit by.
 */
export function Dock({
  inert,
  immersive,
  queueOpen,
  onQueue,
  onCompact,
}: {
  /** Behind a modal surface. */
  inert: boolean;
  immersive: boolean;
  queueOpen: boolean;
  onQueue: () => void;
  onCompact: () => void;
}) {
  const { view, act, pick, native, connecting, fault, reconnect } = usePlayer();
  const { openMenu } = useSurface();
  const now = useNowTrack();
  const live = view?.live;
  const expand = useExpand();
  const seed = live?.station.id ?? nowSeed(now);
  const { style, tone, cover } = useLightTokens(
    live
      ? stationLight(live.station)
      : {
          id: now?.artworkId ?? null,
          seed,
        },
  );
  const stoppable =
    view != null &&
    ["playing", "buffering", "paused", "loading"].includes(view.state);
  // Playing and paused are already visible in the control itself.
  const quiet = view?.state === "playing" || view?.state === "paused";
  return (
    <footer
      className={`dock-slot ${tone}`}
      aria-label="Player"
      style={style}
      inert={inert}
    >
      <div className={view ? "dock" : "dock dock-idle"}>
        <button
          type="button"
          className={now || live ? "dock-sleeve" : "dock-sleeve is-empty"}
          aria-label={immersive ? "Close Now Playing" : "Open Now Playing"}
          onClick={() => expand(!immersive)}
        >
          <Sleeve
            key={now?.artworkId ?? seed}
            id={now?.artworkId ?? null}
            title=""
            artist=""
            seed={seed}
            source={cover}
            small
          />
        </button>
        <div className="dock-meta">
          <div className="dock-copy">
            <strong title={live?.station.name ?? now?.title}>
              {live?.station.name ?? now?.title ?? "Nothing playing"}
            </strong>
            <span className="dock-sub">
              {live ? (
                <span className="dock-artist">
                  {live.title ?? "Internet radio"}
                </span>
              ) : (
                now?.artist &&
                (now.artistId != null ? (
                  <Link className="dock-artist" to={artistPath(now.artistId)}>
                    {now.artist}
                  </Link>
                ) : (
                  <span className="dock-artist">{now.artist}</span>
                ))
              )}
              {!live && (
                <span
                  role={view ? "status" : undefined}
                  className={
                    quiet && now?.artist ? "visually-hidden" : "dock-state"
                  }
                >
                  {view
                    ? descriptions[view.state]
                    : connecting
                      ? "Connecting audio…"
                      : "Player idle"}
                </span>
              )}
            </span>
          </div>
          {now?.trackId != null && (
            <HeartButton
              target={{ kind: "track", id: now.trackId }}
              initial={now.favorite}
              title={now.title}
            />
          )}
          {live && <StationFavorite station={live.station} refresh />}
        </div>
        <TransportButtons />
        <div className="dock-tools">
          {view && <VolumeControl />}
          {!view && native && fault && (
            <button className="quiet" type="button" onClick={reconnect}>
              Retry audio
            </button>
          )}
          {view && (
            <IconButton label="Queue" pressed={queueOpen} onClick={onQueue}>
              <ListVideo aria-hidden="true" />
            </IconButton>
          )}
          <IconButton label="Compact player" onClick={onCompact}>
            <RectangleVertical aria-hidden="true" />
          </IconButton>
          <IconButton
            label={immersive ? "Leave Now Playing" : "Now Playing"}
            className="dock-expand"
            pressed={immersive}
            onClick={() => expand(!immersive)}
          >
            {immersive ? (
              <Minimize2 aria-hidden="true" />
            ) : (
              <Maximize2 aria-hidden="true" />
            )}
          </IconButton>
          {view && (
            <IconButton
              label="More playback actions"
              onClick={(event) =>
                openMenu(event, "Playback", [
                  {
                    label: "Open audio files…",
                    Icon: FolderOpen,
                    onSelect: () => pick("listen"),
                  },
                  {
                    label: "Stop",
                    Icon: Square,
                    disabled: !stoppable && !live,
                    onSelect: () => act({ kind: "stop" }),
                  },
                ])
              }
            >
              <Ellipsis aria-hidden="true" />
            </IconButton>
          )}
        </div>
        {live ? (
          <LiveLine live={live} />
        ) : (
          view && <Timeline key={view.generation} view={view} />
        )}
        <Fault />
      </div>
    </footer>
  );
}
