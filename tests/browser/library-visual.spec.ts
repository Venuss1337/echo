import { expect, test, type Page } from "@playwright/test";
import {
  installLibraryFixture,
  settle,
  type LibraryFixtureOptions,
} from "./support/library-fixture";

const screenshots = {
  animations: "disabled",
  maxDiffPixelRatio: 0.002,
  stylePath: "tests/browser/support/screenshot.css",
} as const;

test.use({ reducedMotion: "reduce", deviceScaleFactor: 1 });

const populated: LibraryFixtureOptions = {
  tracks: 480,
  editorial: true,
  queue: [3, 14, 27, 40, 41, 55],
  history: [1, 13, 25, 37, 49, 61, 73],
  favoriteTracks: [2, 15, 28, 41],
  favoriteAlbums: [3, 4, 7, 9],
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

async function open(
  page: Page,
  path: string,
  options: LibraryFixtureOptions = populated,
  size: [number, number] = [1280, 800],
) {
  await page.setViewportSize({ width: size[0], height: size[1] });
  await installLibraryFixture(page, options);
  await page.goto(`/#${path}`);
  await expect(page.locator("#main")).toBeVisible();
  await settle(page);
}

const pages: [string, string, LibraryFixtureOptions?, [number, number]?][] = [
  ["home", "/", populated, [1440, 900]],
  ["home-wide", "/", populated, [1920, 1080]],
  ["home-short", "/", populated, [1280, 720]],
  [
    "home-first-session",
    "/",
    {
      ...populated,
      history: [],
      favoriteAlbums: [],
      player: { state: "paused" },
    },
    [1440, 900],
  ],
  ["home-medium", "/", populated, [820, 720]],
  ["tracks", "/library/tracks", { ...populated, unavailableUpTo: 2 }],
  ["albums", "/library/albums"],
  ["artists", "/library/artists"],
  ["album", "/album/3"],
  [
    "album-discs-long",
    "/album/1",
    { ...populated, discs: 2, longTitles: true },
  ],
  ["album-no-art", "/album/4", { ...populated, noArt: true }],
  ["artist", "/artist/4"],
  ["playlists", "/playlists"],
  ["playlist", "/playlists/1"],
  ["favorites", "/favorites"],
  ["history", "/history"],
  ["search", "/search?q=gl"],
  ["now-playing", "/now-playing", populated, [1440, 900]],
  [
    "now-playing-wide",
    "/now-playing",
    { ...populated, queue: [40] },
    [1920, 1080],
  ],
  ["now-playing-no-art", "/now-playing", { ...populated, noArt: true }],
  ["narrow-home", "/", populated, [420, 640]],
  ["narrow-tracks", "/library/tracks", populated, [420, 640]],
  ["narrow-album", "/album/3", populated, [420, 640]],
  ["narrow-now-playing", "/now-playing", populated, [420, 640]],
  ["empty-home", "/", { tracks: 0, roots: [] }],
  ["empty-library", "/library", { tracks: 0, roots: [] }],
  [
    "folders",
    "/library/folders",
    {
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
  ],
];

for (const [name, path, options, size] of pages) {
  test(`${name} reference`, async ({ page }) => {
    // Relative times must not drift between runs.
    await page.clock.setFixedTime(new Date("2026-11-24T20:00:00Z"));
    await open(page, path, options, size);
    if (name === "home-wide") {
      // An image-complete check can pass while these asynchronously generated
      // covers are still placeholders or the hero still shows its thumbnail.
      await page.waitForFunction(() => {
        const hero =
          document.querySelector<HTMLImageElement>(".hero-sleeve img");
        const recents = [
          ...document.querySelectorAll<HTMLImageElement>(".ledge-shelf img"),
        ];
        return (
          hero?.naturalWidth === 640 &&
          recents.length === 6 &&
          recents.every((image) => image.complete && image.naturalWidth > 0)
        );
      });
      await settle(page);
    }
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await expect(page).toHaveScreenshot(`${name}.png`, screenshots);
  });
}

test("now playing queue reference", async ({ page }) => {
  await open(page, "/now-playing", populated, [1440, 900]);
  await page.getByRole("button", { name: "Queue", exact: true }).click();
  await settle(page);
  await expect(page).toHaveScreenshot("now-playing-queue.png", screenshots);
});

test("rail reference", async ({ page }) => {
  await open(page, "/library/albums");
  await page.getByRole("button", { name: "Collapse sidebar" }).click();
  await settle(page);
  await expect(page).toHaveScreenshot("rail.png", screenshots);
});

test("drawer reference", async ({ page }) => {
  await open(page, "/library/tracks", populated, [420, 640]);
  await page.getByRole("button", { name: "Expand sidebar" }).click();
  await settle(page);
  await expect(page).toHaveScreenshot("drawer.png", screenshots);
});

test("playback menu reference", async ({ page }) => {
  await open(page, "/playlists", populated, [1280, 720]);
  await page.getByRole("button", { name: "More playback actions" }).click();
  await settle(page);
  await expect(page).toHaveScreenshot("playback-menu.png", screenshots);
});

test("queue reference", async ({ page }) => {
  await open(page, "/album/3");
  await page.getByRole("button", { name: "Queue", exact: true }).click();
  await settle(page);
  await expect(page).toHaveScreenshot("queue.png", screenshots);
});

test("narrow queue reference", async ({ page }) => {
  await open(page, "/", populated, [420, 640]);
  await page.getByRole("button", { name: "Queue", exact: true }).click();
  await settle(page);
  await expect(page).toHaveScreenshot("narrow-queue.png", screenshots);
});

for (const [name, size] of [
  ["compact", [420, 640]],
  ["compact-stage", [1280, 800]],
  ["compact-landscape", [900, 560]],
] as [string, [number, number]][]) {
  test(`${name} reference`, async ({ page }) => {
    await open(page, "/", populated, size);
    await page.getByRole("button", { name: "Compact player" }).click();
    await settle(page);
    await expect(page).toHaveScreenshot(`${name}.png`, screenshots);
  });
}

test("menu and dialog reference", async ({ page }) => {
  await open(page, "/album/3");
  await page.getByRole("row").nth(3).hover();
  await page
    .getByRole("row")
    .nth(3)
    .getByRole("button", { name: /^More for/ })
    .click();
  await settle(page);
  await expect(page).toHaveScreenshot("menu.png", screenshots);
  await page.getByRole("menuitem", { name: "Add to playlist…" }).click();
  await settle(page);
  await expect(page).toHaveScreenshot("add-to-playlist.png", screenshots);
});
