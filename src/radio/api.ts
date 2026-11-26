import { invoke } from "@tauri-apps/api/core";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import type {
  LiveState,
  Palette,
  RadioCollectionAct,
  RadioSort,
  Station,
  StationEdit,
  StationPage,
  StationQuery,
  StationShelf,
} from "../generated/contracts";

let nextRequest = 0;

export const radioApi = {
  search: async (query: StationQuery, signal: AbortSignal) => {
    const request = ++nextRequest;
    const cancel = () =>
      void invoke("radio_cancel", { request }).catch(() => undefined);
    signal.throwIfAborted();
    signal.addEventListener("abort", cancel, { once: true });
    try {
      return await invoke<StationPage>("radio_search", { request, query });
    } finally {
      signal.removeEventListener("abort", cancel);
    }
  },
  shelf: (shelf: StationShelf, offset: number) =>
    invoke<StationPage>("radio_collection", { shelf, offset }),
  station: (id: string) => invoke<Station>("radio_station", { id }),
  edit: (id: string) => invoke<StationEdit>("radio_edit", { id }),
  mutate: (action: RadioCollectionAct) =>
    invoke<Station | null>("radio_collection_act", { action }),
  preferences: (sort: RadioSort | null) =>
    invoke<RadioSort>("radio_preferences", { sort }),
  available: () => invoke<boolean>("radio_playback_available"),
};

export const liveLabels: Record<LiveState, string> = {
  connecting: "Connecting…",
  buffering: "Buffering…",
  playing: "Playing live",
  reconnecting: "Reconnecting…",
  paused: "Paused · resume at the live edge",
  offline: "Station unavailable. Try again in a moment.",
  unsupported: "This station doesn’t provide a supported MP3 or AAC stream.",
  rejected: "ECHO blocked this station’s connection for security.",
  outputUnavailable: "Connect a Windows audio output, then try again.",
};

export function radioFault(error: unknown) {
  switch (error) {
    case "invalidSource":
      return "Enter a direct public HTTP or HTTPS stream URL without credentials.";
    case "invalid":
      return "Check the station name and the values you entered.";
    case "directoryUnavailable":
      return "Radio Browser is unavailable. Your saved stations are still here.";
    case "notFound":
      return "This station reference is no longer available. Search again.";
    case "playbackDisabled":
      return "Station playback is unavailable in this build.";
    default:
      return "ECHO couldn’t complete the radio request. Try again.";
  }
}

async function getArtwork(id: string) {
  const bytes = new Uint8Array(
    await invoke<ArrayBuffer>("radio_artwork", { id }),
  );
  const colors = new TextDecoder().decode(bytes.subarray(0, 21));
  if (bytes.length <= 21 || !/^(#[\da-f]{6}){3}$/i.test(colors)) return null;
  const palette: Palette = {
    ink: colors.slice(0, 7),
    glow: colors.slice(7, 14),
    accent: colors.slice(14, 21),
  };
  return { palette, blob: new Blob([bytes.slice(21)], { type: "image/jpeg" }) };
}

/** Only featured, inspected and playing stations request artwork. */
export function useStationArtwork(id: string | null | undefined) {
  const item = useQuery({
    queryKey: ["radio-artwork", id],
    queryFn: () => getArtwork(id!),
    enabled: id != null,
    staleTime: 30_000,
    gcTime: 0,
    retry: false,
  }).data;
  const [loaded, setLoaded] = useState<{ blob: Blob; url: string }>();
  const blob = item?.blob;
  useEffect(() => {
    if (!blob) return;
    const url = URL.createObjectURL(blob);
    // URL lifetime follows the mounted presentation, including cross-exposure.
    const timer = setTimeout(() => setLoaded({ blob, url }), 0);
    return () => {
      clearTimeout(timer);
      URL.revokeObjectURL(url);
    };
  }, [blob]);
  return {
    cover: loaded && loaded.blob === blob ? loaded.url : null,
    colors: item?.palette,
  };
}
