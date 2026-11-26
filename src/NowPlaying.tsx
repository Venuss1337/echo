import { FolderOpen } from "lucide-react";
import { useState, type CSSProperties } from "react";
import { Link } from "react-router";
import { Sleeve } from "./library/Artwork";
import {
  sceneStyle,
  sceneTone,
  useAtmosphere,
  useCover,
} from "./library/covers";
import { titleScale } from "./library/Detail";
import { albumPath, artistPath } from "./library/rows";
import { HeartButton } from "./library/UserData";
import { usePlayer } from "./player";
import type { LiveRadio } from "./generated/contracts";
import { stationDetails, stationLight, StationFavorite } from "./radio/Radio";
import { useLightTokens } from "./shell/Field";
import { descriptions, LiveLine, nowSeed, useNowTrack } from "./transport";

/** The nearest record in the queue that does not look like this one. */
function neighbour(
  view: NonNullable<ReturnType<typeof usePlayer>["view"]>,
  direction: 1 | -1,
) {
  const at = view.tracks.findIndex(({ id }) => id === view.current);
  const track = at < 0 ? undefined : view.tracks[at + direction];
  return track && track.artworkId !== view.tracks[at].artworkId ? track : null;
}

/**
 * The exhibition. The record hangs on a wall lit by its own cover, with a
 * museum label beside it. The works before and after it in the queue hang
 * further along the same wall, cropped by the window.
 */
export function NowPlaying() {
  const { view, act, pick } = usePlayer();
  const now = useNowTrack();
  const artwork = now?.artworkId ?? null;
  const cover = useCover(artwork);
  const atmosphere = useAtmosphere(artwork);
  const seed = nowSeed(now);
  const title = now?.title ?? "Nothing playing";
  const at = view?.tracks.findIndex(({ id }) => id === view.current) ?? -1;
  const next = at >= 0 ? (view?.tracks[at + 1] ?? null) : null;
  const before = view && neighbour(view, -1);
  const after = view && neighbour(view, 1);
  // Which way the wall moves when the work changes.
  const [hung, setHung] = useState({ at, from: 1, moved: false });
  if (hung.at !== at)
    setHung({ at, from: at < hung.at ? -1 : 1, moved: hung.at >= 0 });
  if (view?.live) return <RadioExhibition live={view.live} />;
  return (
    <section
      className={`immersive ${now ? sceneTone(artwork, seed) : ""} ${view?.state === "playing" ? "" : "at-rest"}`}
      aria-label="Now Playing"
      style={
        {
          ...sceneStyle(atmosphere, cover),
          "--from": hung.from,
        } as CSSProperties
      }
    >
      {before && (
        <button
          type="button"
          className="work work-before"
          aria-label={`Previous: ${before.title}`}
          title={`Previous: ${before.title}`}
          onClick={() => act({ kind: "previous" })}
        >
          <Sleeve
            id={before.artworkId}
            title=""
            artist=""
            seed={before.title}
            small
          />
        </button>
      )}
      <div
        className={hung.moved ? "work-current is-rehung" : "work-current"}
        key={`${artwork ?? seed}`}
      >
        <Sleeve
          id={artwork}
          title={now ? now.album || now.title : ""}
          artist={now?.artist ?? ""}
          seed={seed}
        />
      </div>
      <div className="plaque">
        <h1 className={titleScale(title)}>{title}</h1>
        {now ? (
          <p className="byline">
            {now.artistId != null ? (
              <Link to={artistPath(now.artistId)}>{now.artist}</Link>
            ) : (
              now.artist || "Local audio"
            )}
          </p>
        ) : (
          <p className="lede">Choose something and it opens here.</p>
        )}
        {now && (
          <dl className="plaque-facts">
            {now.album && (
              <div>
                <dt>From</dt>
                <dd>
                  {now.albumId != null ? (
                    <Link to={albumPath(now.albumId)}>{now.album}</Link>
                  ) : (
                    now.album
                  )}
                  {now.year && `, ${now.year}`}
                </dd>
              </div>
            )}
            {now.codec && (
              <div>
                <dt>Source</dt>
                <dd>{now.codec}</dd>
              </div>
            )}
            {next && (
              <div>
                <dt>Next</dt>
                <dd>
                  {next.title}
                  {next.subtitle && <small>{next.subtitle}</small>}
                </dd>
              </div>
            )}
            {view && view.state !== "playing" && view.state !== "paused" && (
              <div>
                <dt>State</dt>
                <dd>{descriptions[view.state]}</dd>
              </div>
            )}
          </dl>
        )}
        <div className="actions">
          {now?.trackId != null && (
            <HeartButton
              className="ringed"
              target={{ kind: "track", id: now.trackId }}
              initial={now.favorite}
              title={now.title}
            />
          )}
          {!now && view && (
            <button
              className="pill"
              type="button"
              onClick={() => pick("listen")}
            >
              <FolderOpen aria-hidden="true" /> Open audio files
            </button>
          )}
        </div>
      </div>
      {after && (
        <button
          type="button"
          className="work work-after"
          aria-label={`Play next: ${after.title}`}
          title={`Next: ${after.title}`}
          onClick={() => act({ kind: "next" })}
        >
          <Sleeve
            id={after.artworkId}
            title=""
            artist=""
            seed={after.title}
            small
          />
        </button>
      )}
    </section>
  );
}

function RadioExhibition({ live }: { live: LiveRadio }) {
  const { station } = live;
  const light = useLightTokens(stationLight(station));
  return (
    <section
      className={`immersive live-exhibition ${light.tone}`}
      aria-label="Now Playing"
      style={sceneStyle(light.style, light.cover)}
    >
      <div className="work-current">
        <Sleeve
          id={null}
          seed={station.id}
          title={station.name}
          artist={station.country}
          source={light.cover}
        />
      </div>
      <div className="plaque">
        <h1 className={titleScale(station.name)}>{station.name}</h1>
        <p className="byline">
          {live.title ?? (stationDetails(station) || "Internet radio")}
        </p>
        <LiveLine live={live} />
        <div className="actions">
          <StationFavorite station={station} refresh />
          <Link className="quiet" to="/radio">
            Browse radio
          </Link>
        </div>
      </div>
    </section>
  );
}
