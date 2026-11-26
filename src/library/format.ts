import type {
  LibraryRootView,
  ScanReport,
  ScanView,
} from "../generated/contracts";

const numbers = new Intl.NumberFormat("en");

export function count(value: number) {
  return numbers.format(value);
}

export function plural(value: number, one: string, many = `${one}s`) {
  return `${count(value)} ${value === 1 ? one : many}`;
}

/** m:ss or h:mm:ss for durations. Unknown (0) durations show a dash. */
export function duration(milliseconds: number) {
  if (milliseconds <= 0) return "–:––";
  const total = Math.round(milliseconds / 1000);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = String(total % 60).padStart(2, "0");
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}`
    : `${minutes}:${seconds}`;
}

/** Stable 32-bit FNV-1a hash, used to choose a placeholder tone. */
export function hash(text: string) {
  let value = 0x811c9dc5;
  for (let index = 0; index < text.length; index++) {
    value ^= text.charCodeAt(index);
    value = Math.imul(value, 0x01000193);
  }
  return value >>> 0;
}

export const TONES = 6;

export function toneOf(seed: string) {
  return hash(seed) % TONES;
}

/** The class that lights a record without a cover; no seed is the house. */
export function toneClass(seed: string) {
  return seed ? `tone-${toneOf(seed)}` : "tone-house";
}

/** Pages needed to cover [start, end], never past the last page. */
export function pagesCovering(
  start: number,
  end: number,
  pageSize: number,
  total: number,
) {
  if (total <= 0 || end < start) return [];
  const last = Math.floor((total - 1) / pageSize);
  const first = Math.min(last, Math.max(0, Math.floor(start / pageSize)));
  const finish = Math.min(last, Math.max(first, Math.floor(end / pageSize)));
  return Array.from({ length: finish - first + 1 }, (_, i) => first + i);
}

export function scanHeadline(scan: ScanView, rootName: string) {
  if (scan.phase === "finalizing") return `Finishing ${rootName}`;
  return scan.totalKnown
    ? `Reading ${rootName}`
    : `Looking through ${rootName}`;
}

export function scanDetail(scan: ScanView) {
  return scan.totalKnown
    ? `${count(scan.processed)} of ${count(scan.discovered)} files`
    : `${plural(scan.discovered, "file")} found so far`;
}

export function reportSummary(report: ScanReport) {
  const parts: string[] = [];
  if (report.indexed > 0) parts.push(`${count(report.indexed)} read`);
  if (report.unchanged > 0) parts.push(`${count(report.unchanged)} unchanged`);
  if (report.missing > 0) parts.push(`${count(report.missing)} missing`);
  if (report.failed > 0) parts.push(`${count(report.failed)} couldn’t be read`);
  if (report.skipped > 0) parts.push(`${count(report.skipped)} skipped`);
  const body = parts.length ? parts.join(", ") : "no audio files";
  switch (report.status) {
    case "complete":
      return `Scan complete: ${body}`;
    case "partial":
      return `Scan finished, but some folders couldn’t be read: ${body}`;
    case "cancelled":
      return `Scan cancelled: ${body}`;
    case "interrupted":
      return "The last scan was interrupted. Your index is intact.";
    case "offline":
      return "The folder was unreachable. Your index is unchanged.";
    default:
      return "The last scan failed. Your index is unchanged.";
  }
}

export function rootStatus(root: LibraryRootView) {
  if (!root.online) return "Offline";
  return root.unavailableTracks > 0
    ? `${plural(root.unavailableTracks, "file")} missing`
    : "Ready";
}

export function when(seconds: number | null) {
  if (seconds == null) return null;
  return new Date(seconds * 1000).toLocaleString("en", {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

/** Rounded running time for a collection, such as "48 min" or "3 hr 12 min". */
export function longDuration(milliseconds: number) {
  const minutes = Math.max(1, Math.round(milliseconds / 60_000));
  const hours = Math.floor(minutes / 60);
  return hours > 0
    ? `${count(hours)} hr${minutes % 60 ? ` ${minutes % 60} min` : ""}`
    : `${minutes} min`;
}

/** Short relative time for listening history. */
export function ago(seconds: number, now = Date.now()) {
  const elapsed = Math.max(0, Math.floor(now / 1000 - seconds));
  if (elapsed < 90) return "now";
  const minutes = Math.floor(elapsed / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  return new Date(seconds * 1000).toLocaleDateString("en", {
    month: "short",
    day: "numeric",
  });
}
