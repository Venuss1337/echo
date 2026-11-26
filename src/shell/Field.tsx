import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import {
  sceneStyle,
  sceneTone,
  useAtmosphere,
  useCover,
} from "../library/covers";
import { useStationArtwork } from "../radio/api";

/** One record's light: its cover, or the tone its placeholder would have. */
export interface Light {
  id: number | null;
  seed: string;
  stationArt?: string | null;
}

const HOUSE: Light = { id: null, seed: "" };
const lightKey = (light: Light) => light.id ?? `seed:${light.seed}`;

const PageLight = createContext<{
  light: Light | null;
  set: (light: Light | null) => void;
} | null>(null);

/**
 * The room is lit by what is playing, unless the page is about one record.
 * A page that is lends its own light for as long as it is open.
 */
export function FieldProvider({ children }: { children: ReactNode }) {
  const [light, set] = useState<Light | null>(null);
  const value = useMemo(() => ({ light, set }), [light]);
  return <PageLight value={value}>{children}</PageLight>;
}

function usePageLight() {
  const context = useContext(PageLight);
  if (!context) throw new Error("FieldProvider is missing");
  return context;
}

/** Lights the room with this page's record. Pass null to leave it alone. */
export function useField(light: Light | null) {
  const { set } = usePageLight();
  const id = light?.id ?? null;
  const seed = light?.seed ?? null;
  const stationArt = light?.stationArt;
  useEffect(() => {
    if (seed == null) return;
    set({ id, seed, stationArt });
    return () => set(null);
  }, [id, seed, set, stationArt]);
}

/** The light in effect, given whatever is loaded in the player. */
export function useLight(playing: Light | null) {
  return usePageLight().light ?? playing ?? HOUSE;
}

/** Surface tokens for a light, for the shell or for one surface. */
export function useLightTokens(light: Light) {
  const atmosphere = useAtmosphere(light.id);
  const station = useStationArtwork(light.stationArt);
  return {
    style: station.colors
      ? {
          ...atmosphere,
          "--ink": station.colors.ink,
          "--glow": station.colors.glow,
          "--accent": station.colors.accent,
        }
      : atmosphere,
    tone: light.seed ? sceneTone(light.id, light.seed) : "",
    cover: station.cover,
  };
}

/** Puts the room's light on the document, where every surface inherits it. */
export function useRoomLight(light: Light) {
  const { style, tone } = useLightTokens(light);
  const ink = style["--ink"];
  const glow = style["--glow"];
  const accent = style["--accent"];
  useEffect(() => {
    const root = document.documentElement;
    const tokens = { "--ink": ink, "--glow": glow, "--accent": accent };
    for (const [name, value] of Object.entries(tokens)) {
      if (value) root.style.setProperty(name, value);
    }
    if (tone) root.classList.add(tone);
    return () => {
      for (const name of Object.keys(tokens)) root.style.removeProperty(name);
      if (tone) root.classList.remove(tone);
    };
  }, [accent, glow, ink, tone]);
}

function Exposure({
  light,
  onSettled,
}: {
  light: Light;
  onSettled?: () => void;
}) {
  const { style, tone, cover: stationCover } = useLightTokens(light);
  const localCover = useCover(light.id);
  const cover = stationCover ?? localCover;
  useEffect(() => {
    if (!onSettled) return;
    // Matches the exposure in motion.css. The layer below is hidden by then.
    const timer = setTimeout(onSettled, 1100);
    return () => clearTimeout(timer);
  }, [onSettled]);
  return (
    <div
      className={`exposure ${tone} ${cover ? "has-art" : ""}`}
      style={sceneStyle(style, cover)}
    >
      <div className="exposure-art" />
      <div className="exposure-light" />
    </div>
  );
}

/**
 * The atmosphere: one continuous lit field behind every surface. It is still
 * unless the light changes, and then it cross-exposes once.
 */
export function Field({ light }: { light: Light }) {
  const [layers, setLayers] = useState([light]);
  const top = layers[layers.length - 1];
  if (lightKey(top) !== lightKey(light)) setLayers([top, light]);
  const settled = useMemo(
    () => () => setLayers((current) => current.slice(-1)),
    [],
  );
  return (
    <div className="field" aria-hidden="true">
      {layers.map((layer, index) => (
        <Exposure
          key={lightKey(layer)}
          light={layer}
          onSettled={index > 0 ? settled : undefined}
        />
      ))}
      <svg className="field-rings" viewBox="0 0 1440 900" fill="none">
        <g transform="translate(1040 260) rotate(22)">
          <ellipse cx="-190" rx="330" ry="610" />
          <ellipse rx="330" ry="610" />
          <ellipse cx="190" rx="330" ry="610" />
        </g>
      </svg>
      <div className="field-grain" />
    </div>
  );
}
