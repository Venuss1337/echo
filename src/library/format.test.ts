import { describe, expect, it } from "vitest";
import type { LibrarySummary, ScanReport } from "../generated/contracts";
import {
  count,
  duration,
  hash,
  pagesCovering,
  plural,
  reportSummary,
  toneOf,
  TONES,
} from "./format";
import { dataKeyOf, isSummary, newerSummary } from "./LibraryProvider";

const summary = (revision: string, tracks = 0): LibrarySummary => ({
  revision,
  userRevision: "1",
  roots: [],
  tracks,
  albums: 0,
  artists: 0,
  unavailableTracks: 0,
  scan: null,
});

describe("formatting", () => {
  it("formats counts and durations without locale drift", () => {
    expect(count(20000)).toBe("20,000");
    expect(plural(1, "track")).toBe("1 track");
    expect(plural(0, "album")).toBe("0 albums");
    expect(duration(0)).toBe("–:––");
    expect(duration(61_000)).toBe("1:01");
    expect(duration(3_725_000)).toBe("1:02:05");
  });

  it("derives a stable placeholder tone from the seed alone", () => {
    expect(hash("Spaces\u0001Nils Frahm")).toBe(hash("Spaces\u0001Nils Frahm"));
    expect(toneOf("anything")).toBe(toneOf("anything"));
    const tones = new Set(
      Array.from({ length: 200 }, (_, n) => toneOf(`album ${n}`)),
    );
    expect(tones.size).toBe(TONES);
    expect([...tones].every((tone) => tone >= 0 && tone < TONES)).toBe(true);
  });

  it("covers a visible window with only the pages it touches", () => {
    expect(pagesCovering(0, 19, 100, 20_000)).toEqual([0]);
    expect(pagesCovering(95, 105, 100, 20_000)).toEqual([0, 1]);
    expect(pagesCovering(19_990, 25_000, 100, 20_000)).toEqual([199]);
    expect(pagesCovering(0, 10, 100, 0)).toEqual([]);
    expect(pagesCovering(10, 5, 100, 500)).toEqual([]);
  });

  it("describes every scan outcome honestly", () => {
    const base: ScanReport = {
      status: "complete",
      finishedAt: 1,
      discovered: 10,
      indexed: 4,
      unchanged: 6,
      skipped: 0,
      failed: 1,
      missing: 2,
    };
    expect(reportSummary(base)).toBe(
      "Scan complete: 4 read, 6 unchanged, 2 missing, 1 couldn’t be read",
    );
    expect(reportSummary({ ...base, status: "partial" })).toContain(
      "some folders couldn’t be read",
    );
    expect(reportSummary({ ...base, status: "offline" })).toContain(
      "Your index is unchanged",
    );
    expect(reportSummary({ ...base, status: "interrupted" })).toContain(
      "intact",
    );
  });
});

describe("library summary handling", () => {
  it("rejects malformed summaries at the IPC boundary", () => {
    expect(isSummary(summary("5"))).toBe(true);
    for (const bad of [
      null,
      "text",
      { library: "empty" },
      { ...summary("5"), revision: 5 },
      { ...summary("5"), revision: "-1" },
      { ...summary("5"), roots: "none" },
      { ...summary("5"), tracks: "many" },
    ]) {
      expect(isSummary(bad)).toBe(false);
    }
  });

  it("never lets an older revision replace a newer one", () => {
    const newer = summary("9007199254740994", 5);
    expect(newerSummary(newer, summary("9007199254740993", 1))).toBe(newer);
    expect(newerSummary(undefined, newer)).toBe(newer);
    const replaced = newerSummary(newer, summary("9007199254740995", 6));
    expect(replaced.tracks).toBe(6);
  });

  it("changes the page cache key only when indexed data may differ", () => {
    expect(dataKeyOf(undefined)).toBe("none");
    expect(dataKeyOf(summary("1", 3))).toBe(dataKeyOf(summary("2", 3)));
    expect(dataKeyOf(summary("1", 3))).not.toBe(dataKeyOf(summary("1", 4)));
  });
});
