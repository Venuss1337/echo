import { Maximize2 } from "lucide-react";
import { Artwork } from "./library/Artwork";
import { sceneStyle, useCover } from "./library/covers";
import { titleScale } from "./library/Detail";
import { HeartButton } from "./library/UserData";
import { usePlayer } from "./player";
import { stationLight, StationFavorite } from "./radio/Radio";
import { EchoMark } from "./shell/EchoMark";
import { Field, useLightTokens } from "./shell/Field";
import {
  Fault,
  LiveLine,
  Timeline,
  TransportButtons,
  VolumeControl,
  descriptions,
  nowSeed,
  useNowTrack,
} from "./transport";
import { IconButton } from "./ui";

/**
 * The compact edition. The print fills the card from edge to edge and the
 * title is set across the seam where it meets the ink; the seam itself is the
 * groove. In a window larger than the card, the card floats on the field.
 */
export function CompactPlayer({ onExit }: { onExit: () => void }) {
  const { view } = usePlayer();
  const live = view?.live;
  const now = useNowTrack();
  const artwork = now?.artworkId ?? null;
  const localCover = useCover(artwork);
  const seed = live?.station.id ?? nowSeed(now);
  const title = live?.station.name ?? now?.title ?? "Nothing playing";
  const light = live ? stationLight(live.station) : { id: artwork, seed };
  const { style, cover } = useLightTokens(light);
  return (
    <main
      id="main"
      className="compact-stage"
      aria-label="Compact player"
      data-exposure="lit"
    >
      <Field light={light} />
      <section
        className="edition"
        style={sceneStyle(style, cover ?? localCover)}
      >
        <div className="edition-print">
          <Artwork
            key={artwork ?? seed}
            id={artwork}
            seed={seed}
            source={cover}
            large
          />
        </div>
        <header className="edition-head">
          <span className="wordmark">
            <EchoMark /> <span>ECHO</span>
          </span>
          <IconButton label="Return to full player" onClick={onExit}>
            <Maximize2 aria-hidden="true" />
          </IconButton>
        </header>
        <div className="edition-body">
          <h1 className={titleScale(title)} title={now?.title}>
            {title}
          </h1>
          <div className="edition-line">
            <p className="byline">
              {live
                ? (live.title ?? "Internet radio")
                : now?.artist || "Local audio"}
              {!live && (
                <span
                  role="status"
                  className={
                    view?.state === "playing" || view?.state === "paused"
                      ? "visually-hidden"
                      : "edition-state"
                  }
                >
                  {view ? descriptions[view.state] : "Player idle"}
                </span>
              )}
            </p>
            {now?.trackId != null && (
              <HeartButton
                target={{ kind: "track", id: now.trackId }}
                initial={now.favorite}
                title={now.title}
              />
            )}
            {live && <StationFavorite station={live.station} refresh />}
          </div>
          {live ? (
            <LiveLine live={live} />
          ) : (
            view && <Timeline key={view.generation} view={view} />
          )}
          <div className="edition-controls">
            <TransportButtons />
            <VolumeControl />
          </div>
          <Fault />
        </div>
      </section>
    </main>
  );
}
