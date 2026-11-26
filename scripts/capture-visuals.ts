/**
 * Deterministic review captures of every major view, from a built `dist`.
 *
 *   bun scripts/capture-visuals.ts <dist> <out> [filter]
 *
 * ECHO_REVIEW_COVERS may name a folder of cover0.webp … cover5.webp (the
 * Design Lab's assets) to judge the atmosphere against photographs. Without it
 * the drawn fixture motifs are used. Fixture data never reaches production.
 */
import { chromium, type Page } from "@playwright/test";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, resolve } from "node:path";
import {
  installLibraryFixture,
  settle,
  type LibraryFixtureOptions,
} from "../tests/browser/support/library-fixture";

const dist = resolve(process.argv[2] ?? "dist");
const out = resolve(process.argv[3] ?? "captures");
const filter = process.argv[4] ? new RegExp(process.argv[4]) : null;
mkdirSync(out, { recursive: true });

type Cover = NonNullable<LibraryFixtureOptions["covers"]>[number];

/** Palettes the Rust extractor produces for the Design Lab covers. */
const PALETTES: Cover["palette"][] = [
  ["#412528", "#7d3b41", "#deb0b5"],
  ["#38342e", "#695e4f", "#d8c9b6"],
  ["#4a2d1c", "#914d27", "#f3bb9b"],
  ["#214145", "#2d6f79", "#aadde4"],
  ["#414125", "#686831", "#dedeb0"],
  ["#38352e", "#69604f", "#d8cdb6"],
];
const folder = process.env.ECHO_REVIEW_COVERS;
const photographs: Cover[] = folder
  ? PALETTES.flatMap((palette, index) => {
      const path = join(folder, `cover${index}.webp`);
      return existsSync(path)
        ? [
            {
              image: `data:image/webp;base64,${readFileSync(path).toString("base64")}`,
              palette,
            },
          ]
        : [];
    })
  : [];
const covers = photographs.length > 0 ? photographs : undefined;

/** Covers at the extremes, drawn flat, with the extractor's output. */
const sheet = (
  fill: string,
  mark: string,
  palette: Cover["palette"],
): Cover => ({ image: `sheet:${fill}:${mark}`, palette });
const extremes: Record<string, Cover> = {
  white: sheet("#fafafa", "#dddddd", ["#363033", "#63545c", "#eeb4c2"]),
  black: sheet("#040404", "#222222", ["#363033", "#63545c", "#eeb4c2"]),
  yellow: sheet("#ffe600", "#ff9900", ["#454119", "#6f671d", "#f3ea9b"]),
  red: sheet("#ff0014", "#ff8890", ["#4a1c1f", "#91272f", "#f39ba2"]),
  blue: sheet("#0a14ff", "#5a64ff", ["#1c1d4a", "#272b91", "#9b9ff3"]),
  cyan: sheet("#00f0ff", "#007c84", ["#1b4649", "#1f7075", "#9beef3"]),
};

const populated: LibraryFixtureOptions = {
  tracks: 480,
  editorial: true,
  covers,
  queue: [3, 14, 27, 40, 41, 55],
  history: [1, 13, 25, 37, 49, 61, 73],
  favoriteTracks: [2, 15, 28, 41],
  favoriteAlbums: [3, 4, 7, 9],
  player: { state: "playing" },
  playlists: [
    {
      id: 1,
      name: "Night drives",
      description: "Headlights, rain, and nothing that raises its voice.",
      trackIds: [3, 14, 27, 40, 3, 62, 75, 88],
      lost: [2],
    },
    { id: 2, name: "Soft focus", trackIds: [51, 64, 78, 90] },
    { id: 3, name: "Sunday slow", trackIds: [100, 115] },
    { id: 4, name: "Unsorted", trackIds: [] },
  ],
};

interface View {
  name: string;
  path: string;
  options?: LibraryFixtureOptions;
  act?: (page: Page) => Promise<void>;
  sizes?: [number, number][];
}

const DESKTOP: [number, number][] = [
  [1920, 1080],
  [1440, 900],
  [1280, 720],
];
const NARROW: [number, number][] = [
  [820, 720],
  [420, 640],
];
const openQueue = async (page: Page) => {
  await page.getByRole("button", { name: "Queue", exact: true }).click();
};
const compact = async (page: Page) => {
  await page.getByRole("button", { name: "Compact player" }).first().click();
};
const views: View[] = [
  { name: "home", path: "/", sizes: [...DESKTOP, ...NARROW] },
  { name: "tracks", path: "/library/tracks", sizes: [...DESKTOP, ...NARROW] },
  { name: "albums", path: "/library/albums" },
  { name: "artists", path: "/library/artists" },
  { name: "album", path: "/album/3", sizes: [...DESKTOP, ...NARROW] },
  { name: "artist", path: "/artist/4", sizes: [...DESKTOP, ...NARROW] },
  { name: "playlists", path: "/playlists" },
  { name: "playlist", path: "/playlists/1" },
  { name: "favorites", path: "/favorites" },
  { name: "history", path: "/history" },
  { name: "search", path: "/search?q=gl" },
  {
    name: "folders",
    path: "/library/folders",
    options: {
      ...populated,
      roots: [
        { id: 1, name: "Music", path: "D:\\Music", online: true, tracks: 400 },
        {
          id: 2,
          name: "Archive",
          path: "F:\\Archive",
          online: false,
          tracks: 80,
        },
      ],
      scan: {
        rootId: 1,
        phase: "reading",
        discovered: 1000,
        processed: 250,
        totalKnown: true,
      },
    },
  },
  { name: "now-playing", path: "/now-playing", sizes: [...DESKTOP, ...NARROW] },
  {
    name: "now-playing-queue",
    path: "/now-playing",
    act: openQueue,
    sizes: [[1440, 900]],
  },
  { name: "queue", path: "/album/3", act: openQueue, sizes: DESKTOP },
  {
    name: "queue-tracks",
    path: "/library/tracks",
    act: openQueue,
    sizes: [
      [1440, 900],
      [420, 640],
    ],
  },
  {
    name: "compact",
    path: "/",
    act: compact,
    sizes: [
      [1280, 720],
      [420, 640],
    ],
  },
  {
    name: "menu",
    path: "/album/3",
    act: async (page) => {
      const row = page.getByRole("row").nth(3);
      await row.hover();
      await row.getByRole("button", { name: /^More for/ }).click();
    },
    sizes: [[1440, 900]],
  },
  {
    name: "dialog",
    path: "/playlists",
    act: async (page) => {
      await page.getByRole("button", { name: "New playlist" }).last().click();
    },
    sizes: [[1440, 900]],
  },
  {
    name: "empty-home",
    path: "/",
    options: { tracks: 0, roots: [] },
    sizes: [[1440, 900]],
  },
  {
    name: "empty-library",
    path: "/library",
    options: { tracks: 0, roots: [] },
    sizes: [[1440, 900]],
  },
  {
    // A first session: something loaded and kept, nothing listened to yet.
    name: "first-home",
    path: "/",
    options: {
      ...populated,
      history: [],
      favoriteAlbums: [],
      player: { state: "paused" },
    },
    sizes: [[1440, 900]],
  },
  {
    name: "missing-art-home",
    path: "/",
    options: { ...populated, noArt: true },
    sizes: [[1440, 900]],
  },
  {
    name: "missing-art-now-playing",
    path: "/now-playing",
    options: { ...populated, noArt: true },
    sizes: [[1440, 900]],
  },
  {
    name: "long-metadata",
    path: "/album/1",
    options: { ...populated, discs: 2, longTitles: true, queue: [1, 2] },
    sizes: [
      [1440, 900],
      [820, 720],
    ],
  },
  ...Object.entries(extremes).flatMap(([name, cover]): View[] =>
    [
      ["home", "/"],
      ["now-playing", "/now-playing"],
    ].map(([view, path]) => ({
      name: `palette-${name}-${view}`,
      path,
      options: { ...populated, covers: [cover] },
      sizes: [[1440, 900]],
    })),
  ),
];

const types: Record<string, string> = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".ttf": "font/ttf",
  ".png": "image/png",
};
const server = createServer((request, response) => {
  const path = new URL(request.url ?? "/", "http://x").pathname;
  const file = join(dist, path === "/" ? "index.html" : path);
  if (!file.startsWith(dist) || !existsSync(file)) {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, {
    "content-type": types[extname(file)] ?? "application/octet-stream",
  });
  response.end(readFileSync(file));
});
await new Promise<void>((done) => server.listen(0, "127.0.0.1", done));
const address = server.address();
if (!address || typeof address === "string") throw new Error("no port");
const base = `http://127.0.0.1:${address.port}`;

const browser = await chromium.launch();
const jobs = views
  .flatMap((view) => (view.sizes ?? DESKTOP).map((size) => ({ view, size })))
  .filter(
    ({ view, size }) => !filter || filter.test(`${view.name}-${size[0]}`),
  );
let failed = 0;
async function capture({ view, size: [width, height] }: (typeof jobs)[number]) {
  const page = await browser.newPage({
    viewport: { width, height },
    reducedMotion: "reduce",
  });
  try {
    await page.clock.setFixedTime(new Date("2026-11-24T20:00:00Z"));
    await installLibraryFixture(page, view.options ?? populated);
    await page.goto(`${base}/#${view.path}`);
    await page.locator("#main").waitFor();
    await settle(page);
    await view.act?.(page);
    await settle(page);
    await page.mouse.move(2, 400);
    await page.waitForTimeout(350);
    await page.screenshot({ path: join(out, `${view.name}-${width}.png`) });
  } catch (error) {
    failed += 1;
    console.error(view.name, width, String(error).split("\n")[0]);
  } finally {
    await page.close();
  }
}
const pending = [...jobs];
await Promise.all(
  Array.from({ length: 5 }, async () => {
    for (let job = pending.shift(); job; job = pending.shift())
      await capture(job);
  }),
);
await browser.close();
server.close();
console.log(`${jobs.length - failed} captures in ${out}, ${failed} failed`);
process.exit(failed ? 1 : 0);
