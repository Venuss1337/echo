import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useEffect, useState, type CSSProperties } from "react";
import type { ArtworkSize } from "../generated/contracts";
import { libraryApi } from "./api";
import { toneClass } from "./format";

/** Idle covers kept per size. Covers in use are never released. */
const IDLE = { thumb: 240, large: 8 } as const;

interface Slot {
  url: Promise<string | null>;
  users: number;
}

const caches: Record<ArtworkSize, Map<number, Slot>> = {
  thumb: new Map(),
  large: new Map(),
};

function acquire(id: number, size: ArtworkSize) {
  const cache = caches[size];
  let slot = cache.get(id);
  if (!slot) {
    slot = {
      users: 0,
      url: libraryApi.artwork(id, size).then(
        (bytes) =>
          URL.createObjectURL(
            new Blob([bytes as Uint8Array<ArrayBuffer>], {
              type: "image/jpeg",
            }),
          ),
        () => {
          // A failure may be temporary, so the next mount asks again.
          cache.delete(id);
          return null;
        },
      ),
    };
  }
  // Re-inserting keeps the map in least-recently-used order.
  cache.delete(id);
  cache.set(id, slot);
  slot.users += 1;
  return slot;
}

function release(size: ArtworkSize, slot: Slot) {
  slot.users -= 1;
  const cache = caches[size];
  let idle = [...cache.values()].filter(({ users }) => users === 0).length;
  for (const [key, candidate] of cache) {
    if (idle <= IDLE[size]) break;
    if (candidate.users > 0) continue;
    cache.delete(key);
    idle -= 1;
    void candidate.url.then((url) => {
      // Something may have asked for it again while this resolved.
      if (url && candidate.users === 0) URL.revokeObjectURL(url);
    });
  }
}

/** A blob URL for cached artwork, or null until it is ready or if absent. */
export function useArtwork(id: number | null | undefined, size: ArtworkSize) {
  const [loaded, setLoaded] = useState<{
    id: number;
    size: ArtworkSize;
    url: string | null;
  }>();
  useEffect(() => {
    if (id == null) return;
    let live = true;
    const slot = acquire(id, size);
    void slot.url.then((url) => {
      if (live) setLoaded({ id, size, url });
    });
    return () => {
      live = false;
      release(size, slot);
    };
  }, [id, size]);
  return loaded && loaded.id === id && loaded.size === size ? loaded.url : null;
}

/** Large art with the already cached thumbnail as an immediate stand-in. */
export function useCover(id: number | null | undefined) {
  const thumb = useArtwork(id, "thumb");
  const large = useArtwork(id, "large");
  return large ?? thumb;
}

export type Atmosphere = CSSProperties & Record<`--${string}`, string>;

/**
 * Semantic lighting for one cover. Rust computes and caches the colors and
 * guarantees their contrast; absent artwork keeps the house palette from CSS.
 */
export function useAtmosphere(id: number | null | undefined): Atmosphere {
  const palette = useQuery({
    queryKey: ["palette", id],
    queryFn: () => libraryApi.palette(id as number),
    enabled: id != null,
    // Hold the last light until the next is known, instead of flashing house.
    placeholderData: keepPreviousData,
    staleTime: Infinity,
    gcTime: 5 * 60_000,
    retry: false,
  }).data;
  return palette && id != null
    ? {
        "--ink": palette.ink,
        "--glow": palette.glow,
        "--accent": palette.accent,
      }
    : {};
}

/** Scenes without a cover take the same light as their placeholder. */
export function sceneTone(id: number | null | undefined, seed: string) {
  return id == null ? toneClass(seed) : "";
}

export function sceneStyle(
  atmosphere: Atmosphere,
  cover: string | null,
): Atmosphere {
  return cover ? { ...atmosphere, "--art": `url("${cover}")` } : atmosphere;
}
