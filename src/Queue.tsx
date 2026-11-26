import { ArrowDown, ArrowUp, GripVertical, ListPlus, X } from "lucide-react";
import { memo, useCallback, useRef, useState } from "react";
import type { QueueTrack } from "./generated/contracts";
import { Artwork } from "./library/Artwork";
import { plural } from "./library/format";
import { usePlayer } from "./player";
import { useLightTokens } from "./shell/Field";
import { Inspector } from "./shell/Inspector";
import { nowSeed, useNowTrack } from "./transport";
import { IconButton, useReorder } from "./ui";

const QueueRow = memo(function QueueRow({
  track,
  upcoming,
  first,
  last,
  dropClass,
  onGrip,
  onMove,
  onRemove,
  onPlay,
}: {
  track: QueueTrack;
  upcoming: boolean;
  first: boolean;
  last: boolean;
  dropClass: string;
  onGrip: (event: React.PointerEvent<HTMLElement>) => void;
  onMove: (direction: -1 | 1) => void;
  onRemove: () => void;
  onPlay: () => void;
}) {
  return (
    <li
      className={`queue-row ${upcoming ? "" : "played"} ${track.unavailable ? "unavailable" : ""} ${dropClass}`}
      data-reorder-id={upcoming ? track.id : undefined}
      data-reorder-last={last}
    >
      {upcoming && (
        <button
          type="button"
          className="grip"
          aria-label={`Reorder ${track.title}. Use Alt with the arrow keys.`}
          title="Drag to reorder, or Alt + arrow keys"
          onPointerDown={onGrip}
          onKeyDown={(event) => {
            if (!event.altKey) return;
            if (event.key === "ArrowUp" && !first) onMove(-1);
            else if (event.key === "ArrowDown" && !last) onMove(1);
            else return;
            event.preventDefault();
          }}
        >
          <GripVertical aria-hidden="true" />
        </button>
      )}
      <button
        type="button"
        className="queue-track"
        title={track.title}
        onClick={onPlay}
      >
        <Artwork id={track.artworkId} seed={track.title} />
        <span className="queue-copy">
          <span>{track.title}</span>
          <small>
            {track.unavailable
              ? "Unavailable"
              : (track.subtitle ?? "Local file")}
          </small>
        </span>
      </button>
      <span className="queue-row-actions">
        {upcoming && (
          <>
            <IconButton
              label={`Move ${track.title} up`}
              disabled={first}
              onClick={() => onMove(-1)}
            >
              <ArrowUp aria-hidden="true" />
            </IconButton>
            <IconButton
              label={`Move ${track.title} down`}
              disabled={last}
              onClick={() => onMove(1)}
            >
              <ArrowDown aria-hidden="true" />
            </IconButton>
          </>
        )}
        <IconButton label={`Remove ${track.title}`} onClick={onRemove}>
          <X aria-hidden="true" />
        </IconButton>
      </span>
    </li>
  );
});

/** Rust owns the queue. This panel names entries by id and shows what it says. */
export function QueuePanel({ onClose }: { onClose: () => void }) {
  const { view, act, pick } = usePlayer();
  const scroller = useRef<HTMLDivElement>(null);
  const move = useCallback(
    (id: string, before: string | null) =>
      act({ kind: "moveBefore", id, before }),
    [act],
  );
  const reorder = useReorder(scroller, move);
  const [history, setHistory] = useState(false);
  const now = useNowTrack();
  const { style } = useLightTokens({
    id: now?.artworkId ?? null,
    seed: nowSeed(now),
  });
  const tracks = view?.tracks ?? [];
  const at = tracks.findIndex(({ id }) => id === view?.current);
  const current = at >= 0 ? tracks[at] : undefined;
  const played = at > 0 ? tracks.slice(0, at) : [];
  const upcoming = tracks.slice(at + 1);
  return (
    <Inspector
      title="Queue"
      style={style}
      scroller={scroller}
      onClose={onClose}
      actions={
        <IconButton
          label="Add files to the queue"
          onClick={() => pick("append")}
        >
          <ListPlus aria-hidden="true" />
        </IconButton>
      }
    >
      {current && (
        <>
          <h3 className="visually-hidden">Current track</h3>
          <div className="queue-current" aria-current="true">
            <Artwork id={current.artworkId} seed={current.title} />
            <span className="queue-copy">
              <span>{current.title}</span>
              <small>
                {current.unavailable
                  ? "Unavailable"
                  : (current.subtitle ?? "Local file")}
              </small>
            </span>
          </div>
        </>
      )}
      <div className="inspector-section">
        <h3>
          Up next{upcoming.length > 0 && <small>{upcoming.length}</small>}
        </h3>
        {upcoming.length > 0 && (
          <button
            className="text-button"
            type="button"
            aria-label="Clear upcoming"
            onClick={() => act({ kind: "clearUpcoming" })}
          >
            Clear
          </button>
        )}
      </div>
      {upcoming.length > 0 ? (
        <ol className="queue-list">
          {upcoming.map((track, index) => (
            <QueueRow
              key={track.id}
              track={track}
              upcoming
              first={index === 0}
              last={index === upcoming.length - 1}
              dropClass={reorder.rowClass(
                track.id,
                index === upcoming.length - 1,
              )}
              onGrip={(event) => reorder.start(track.id, event)}
              onMove={(direction) =>
                move(
                  track.id,
                  direction < 0
                    ? upcoming[index - 1].id
                    : (upcoming[index + 2]?.id ?? null),
                )
              }
              onRemove={() => act({ kind: "remove", id: track.id })}
              onPlay={() => act({ kind: "play", id: track.id })}
            />
          ))}
        </ol>
      ) : (
        <p className="queue-empty">
          {tracks.length === 0
            ? "Play an album, or add tracks from any listing."
            : view?.repeat === "all"
              ? "The queue starts over after this track."
              : view?.repeat === "one"
                ? "This track repeats."
                : "Nothing follows this track."}
        </p>
      )}
      {played.length > 0 && (
        <>
          <button
            type="button"
            className="text-button played-toggle"
            aria-expanded={history}
            onClick={() => setHistory(!history)}
          >
            {history ? "Hide" : "Show"} {plural(played.length, "earlier track")}
          </button>
          {history && (
            <ol className="queue-list" aria-label="Earlier in the queue">
              {played.map((track) => (
                <QueueRow
                  key={track.id}
                  track={track}
                  upcoming={false}
                  first
                  last
                  dropClass=""
                  onGrip={() => undefined}
                  onMove={() => undefined}
                  onRemove={() => act({ kind: "remove", id: track.id })}
                  onPlay={() => act({ kind: "play", id: track.id })}
                />
              ))}
            </ol>
          )}
        </>
      )}
    </Inspector>
  );
}
