import { useState, type CSSProperties } from "react";
import { useArtwork, useCover } from "./covers";
import { toneClass } from "./format";

/** The concentric elliptical imprint from the ECHO mark. */
function Imprint() {
  return (
    <svg viewBox="0 0 100 100" aria-hidden="true">
      {[0, 1, 2].map((index) => (
        <ellipse
          key={index}
          cx={38 + index * 12}
          cy="50"
          rx="17"
          ry="33"
          transform={`rotate(22 ${38 + index * 12} 50)`}
        />
      ))}
    </svg>
  );
}

/**
 * Cover art or a deterministic branded placeholder. The tone comes from a
 * stable hash of `seed`; it is never presented as real artwork.
 */
export function Artwork({
  id,
  seed,
  className = "",
  large = false,
  label,
  source,
}: {
  id: number | null;
  seed: string;
  className?: string;
  large?: boolean;
  /** Printed on the placeholder sleeve when there is no cover. */
  label?: { title: string; artist: string };
  source?: string | null;
}) {
  const thumb = useArtwork(large ? null : id, "thumb");
  const cover = useCover(large ? id : null);
  const [broken, setBroken] = useState<string | null>(null);
  const url = source ?? (large ? cover : thumb);
  return (
    <span className={`art ${className}`} aria-hidden="true">
      {url && broken !== url ? (
        <img
          src={url}
          alt=""
          decoding="async"
          draggable={false}
          onError={() => setBroken(url)}
        />
      ) : (
        <span className={`art-placeholder ${toneClass(seed)}`}>
          <Imprint />
          {label && (
            <span className="sleeve-label">
              {label.title}
              <small>{label.artist}</small>
            </span>
          )}
        </span>
      )}
    </span>
  );
}

/**
 * The open sleeve: one print with two offset impressions behind it. The
 * impressions are the same photograph, so a sleeve always matches itself.
 */
export function Sleeve({
  id,
  title,
  artist,
  seed,
  small = false,
  source,
}: {
  id: number | null;
  title: string;
  artist: string;
  seed: string;
  /** Drawn from the thumbnail, without lettering on the placeholder. */
  small?: boolean;
  source?: string | null;
}) {
  const thumb = useArtwork(id, "thumb");
  const large = useArtwork(small ? null : id, "large");
  const cover = source ?? large ?? thumb;
  return (
    <span
      className={`sleeve ${toneClass(seed)} ${cover ? "has-cover" : ""}`}
      style={
        cover ? ({ "--print": `url("${cover}")` } as CSSProperties) : undefined
      }
    >
      <Artwork
        id={id}
        seed={seed}
        large={!small}
        source={source}
        label={small ? undefined : { title, artist }}
      />
    </span>
  );
}

/**
 * A collection without artwork of its own, as the spines of up to four records
 * standing side by side. One record is simply that record.
 */
export function Mosaic({
  ids,
  seed,
  large = false,
}: {
  ids: number[];
  seed: string;
  large?: boolean;
}) {
  if (ids.length < 2) {
    return <Artwork id={ids[0] ?? null} seed={seed} large={large} />;
  }
  return (
    <span className="art mosaic" aria-hidden="true">
      {ids.slice(0, 4).map((id) => (
        <Artwork key={id} id={id} seed={seed} large={large} />
      ))}
    </span>
  );
}
