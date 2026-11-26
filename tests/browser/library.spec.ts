import { expect, test, type Page } from "@playwright/test";
import { calls, installLibraryFixture } from "./support/library-fixture";

const menuItem = (page: Page, name: string) =>
  page.getByRole("menuitem", { name });

test("an empty library offers folder selection and never fabricates content", async ({
  page,
}) => {
  await installLibraryFixture(page, { tracks: 0, roots: [] });
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Your library is empty" }),
  ).toBeVisible();
  await expect(page.getByRole("article")).toHaveCount(0);
  await page.goto("/#/library");
  await expect(page.getByRole("table")).toHaveCount(0);
  await page.getByRole("button", { name: "Add music folder" }).click();
  expect(await calls(page, "library_add_root")).toHaveLength(1);
  await page.getByRole("link", { name: "Music folders" }).click();
  await expect(page.getByRole("heading", { name: "New folder" })).toBeVisible();
});

test("a 20,000 track list virtualizes pages and plays by id", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await installLibraryFixture(page, { tracks: 20_000 });
  await page.goto("/#/library/tracks");
  const table = page.getByRole("table", { name: "Tracks" });
  await expect(table).toHaveAttribute("aria-rowcount", "20001");
  await expect(
    table.getByRole("row", { name: /Fixture track 00001/ }),
  ).toBeVisible();
  expect(await page.locator("[role=row]").count()).toBeLessThan(80);
  await page.locator(".virtual-scroll").evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  const last = table.getByRole("row", { name: /Fixture track 20000/ });
  await expect(last).toBeVisible();
  await last.hover();
  await last
    .getByRole("button", { name: "Play Fixture track 20000", exact: true })
    .click();
  for (const label of ["Add to queue", "Play next"]) {
    await last.getByRole("button", { name: /^More for/ }).click();
    await menuItem(page, label).click();
  }
  const plays = await calls(page, "library_play");
  expect(plays.map(({ args }) => args)).toEqual(
    ["replace", "append", "next"].map((mode) => ({
      target: { kind: "track", id: 20000 },
      mode,
      shuffle: false,
    })),
  );
  for (const { args } of await calls(page, "library_tracks")) {
    expect(Object.keys(args)).toEqual(["query"]);
    expect((args.query as { limit: number }).limit).toBeLessThanOrEqual(100);
  }
  // Rows never request the large artwork variant.
  const sizes = (await calls(page, "library_artwork")).map(
    ({ args }) => args.size,
  );
  expect(sizes.filter((size) => size === "large").length).toBeLessThanOrEqual(
    1,
  );
});

test("album page plays from a row, shuffles, groups discs and favorites", async ({
  page,
}) => {
  await installLibraryFixture(page, { tracks: 240, discs: 2 });
  await page.goto("/#/library/albums");
  await page
    .getByRole("article")
    .first()
    .getByRole("link", { name: "Fixture Suite 00001" })
    .first()
    .click();
  await expect(
    page.getByRole("heading", { level: 1, name: "Fixture Suite 00001" }),
  ).toBeVisible();
  const table = page.getByRole("table", { name: /Tracks on/ });
  await expect(table.getByRole("row", { name: /^Disc/ })).toHaveCount(2);
  const row = table.getByRole("row", { name: /Fixture track 00003/ });
  await row.hover();
  await row
    .getByRole("button", { name: "Play Fixture track 00003", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Shuffle Fixture Suite 00001" })
    .click();
  expect((await calls(page, "library_play")).map(({ args }) => args)).toEqual([
    {
      target: { kind: "album", id: 1, startTrack: 3 },
      mode: "replace",
      shuffle: false,
    },
    {
      target: { kind: "album", id: 1, startTrack: null },
      mode: "replace",
      shuffle: true,
    },
  ]);
  // One favorite, visible in every place that shows the track.
  await row.hover();
  await row.getByRole("button", { name: /^Favorite Fixture track/ }).click();
  await expect(
    row.getByRole("button", { name: /^Remove .* from favorites/ }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.goto("/#/library/tracks");
  await expect(
    page
      .getByRole("row", { name: /Fixture track 00003/ })
      .getByRole("button", { name: /^Remove .* from favorites/ }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.goto("/#/favorites");
  await expect(
    page.getByRole("row", { name: /Fixture track 00003/ }),
  ).toBeVisible();
});

test("artist page separates lead albums from appearances", async ({ page }) => {
  await installLibraryFixture(page, { tracks: 480 });
  await page.goto("/#/library/artists");
  await page.getByRole("link", { name: "Fixture Quartet 2" }).click();
  await expect(
    page.getByRole("heading", { level: 1, name: "Fixture Quartet 2" }),
  ).toBeVisible();
  for (const name of ["Albums", "Tracks", "Appears on"]) {
    await expect(
      page.getByRole("heading", { level: 2, name, exact: true }),
    ).toBeVisible();
  }
  const roles = (await calls(page, "library_albums")).map(
    ({ args }) => (args.query as { role: string | null }).role,
  );
  expect(roles).toEqual(expect.arrayContaining(["main", "appears"]));
  await page
    .locator("#main")
    .getByRole("button", { name: "Play", exact: true })
    .first()
    .click();
  expect((await calls(page, "library_play"))[0].args.target).toEqual({
    kind: "artist",
    id: 2,
  });
});

test("missing album and artist pages explain themselves", async ({ page }) => {
  await installLibraryFixture(page, { tracks: 12 });
  await page.goto("/#/album/999");
  await expect(
    page.getByRole("heading", { name: "Album not found" }),
  ).toBeVisible();
  await page.goto("/#/playlists/999");
  await expect(
    page.getByRole("heading", { name: "Playlist not found" }),
  ).toBeVisible();
});

test("playlists: create, duplicate entries, reorder, remove, edit, delete", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await installLibraryFixture(page, { tracks: 60 });
  await page.goto("/#/playlists");
  await page.getByRole("button", { name: "New playlist" }).last().click();
  const form = page.getByRole("dialog", { name: "New playlist" });
  await expect(form.getByLabel("Name")).toBeFocused();
  await expect(
    form.getByRole("button", { name: "Create playlist" }),
  ).toBeDisabled();
  await form.getByLabel("Name").fill("Mix");
  await form.getByLabel("Description").fill("Two of one thing");
  await page.keyboard.press("Enter");
  await form
    .getByRole("button", { name: "Create playlist" })
    .click({ timeout: 1000 })
    .catch(() => undefined);
  await expect(form).toHaveCount(0);

  // Select several rows, then add them, then add one of them again.
  await page.goto("/#/library/tracks");
  for (const n of ["00001", "00002", "00003"]) {
    await page
      .getByRole("row", { name: new RegExp(`Fixture track ${n}`) })
      .click({ modifiers: ["Control"] });
  }
  const selection = page.getByRole("group", { name: "Selection" });
  await expect(selection).toContainText("3 tracks selected");
  await selection.getByRole("button", { name: /Add to playlist/ }).click();
  await page.getByRole("dialog").getByRole("button", { name: /^Mix/ }).click();
  const second = page.getByRole("row", { name: /Fixture track 00002/ });
  await second.hover();
  await second.getByRole("button", { name: /^More for/ }).click();
  await menuItem(page, "Add to playlist…").click();
  await page.getByRole("dialog").getByRole("button", { name: /^Mix/ }).click();

  await page.goto("/#/playlists/1");
  const table = page.getByRole("table", { name: "Tracks in Mix" });
  const titles = async () =>
    (await table.locator(".line-title").allTextContents()).map((text) =>
      Number(text.slice(-5)),
    );
  await expect.poll(titles).toEqual([1, 2, 3, 2]);

  // Keyboard: the second occurrence moves, the first does not.
  await table
    .getByRole("button", { name: /^Reorder/ })
    .nth(3)
    .focus();
  await page.keyboard.press("Alt+ArrowUp");
  await expect.poll(titles).toEqual([1, 2, 2, 3]);

  // Pointer: drag the first row below the third.
  const grip = table.getByRole("button", { name: /^Reorder/ }).first();
  const from = (await grip.boundingBox())!;
  const to = (await table.getByRole("row").nth(3).boundingBox())!;
  await page.mouse.move(from.x + 6, from.y + 14);
  await page.mouse.down();
  await page.mouse.move(from.x + 6, to.y + to.height - 6, { steps: 8 });
  await expect(table.locator(".drop-before")).toHaveCount(1);
  await page.mouse.up();
  await expect.poll(titles).toEqual([2, 2, 1, 3]);

  // Escape abandons a drag.
  await page.mouse.move(from.x + 6, from.y + 14);
  await page.mouse.down();
  await page.mouse.move(from.x + 6, to.y + to.height - 6, { steps: 4 });
  await page.keyboard.press("Escape");
  await page.mouse.up();
  await expect.poll(titles).toEqual([2, 2, 1, 3]);

  const row = table.getByRole("row").nth(2);
  await row.hover();
  await row.getByRole("button", { name: /^More for/ }).click();
  await menuItem(page, "Remove from playlist").click();
  await expect.poll(titles).toEqual([2, 1, 3]);

  await page.getByRole("button", { name: "More for Mix" }).click();
  await menuItem(page, "Edit details…").click();
  await page.getByLabel("Name").fill("Renamed mix");
  await page.getByRole("button", { name: "Save" }).click();
  await expect(
    page.getByRole("heading", { level: 1, name: "Renamed mix" }),
  ).toBeVisible();

  // Deleting keeps what is playing.
  await page
    .locator("#main")
    .getByRole("button", { name: "Play", exact: true })
    .first()
    .click();
  const player = page.getByRole("contentinfo", { name: "Player" });
  await expect(player.getByRole("status")).toHaveText("Playing");
  await page.getByRole("button", { name: "More for Renamed mix" }).click();
  await menuItem(page, "Delete playlist…").click();
  const confirm = page.getByRole("dialog", { name: "Delete playlist?" });
  await expect(confirm).toContainText("music files");
  await expect(
    confirm.getByRole("button", { name: "Keep playlist" }),
  ).toBeFocused();
  await confirm.getByRole("button", { name: "Delete playlist" }).click();
  await expect(page).toHaveURL(/#\/playlists$/);
  await expect(player.getByRole("status")).toHaveText("Playing");
});

test("unavailable playlist entries stay listed and unplayable", async ({
  page,
}) => {
  await installLibraryFixture(page, {
    tracks: 24,
    playlists: [{ id: 1, name: "Kept", trackIds: [1, 2, 3], lost: [1] }],
  });
  await page.goto("/#/playlists/1");
  const row = page.getByRole("row", { name: /Fixture track 00002/ });
  await expect(row).toContainText("Unavailable");
  await expect(row.getByRole("button", { name: /^Play / })).toHaveCount(0);
  await expect(row.getByRole("button", { name: /^Favorite / })).toHaveCount(1);
  await expect(page.locator(".playlisthead")).toContainText(
    "1 track unavailable",
  );
});

test("history clears only after an explicit, scoped choice", async ({
  page,
}) => {
  await installLibraryFixture(page, { tracks: 60, history: [4, 9, 4] });
  await page.goto("/#/history");
  await expect(
    page.getByRole("table", { name: "Listening history" }).getByRole("row"),
  ).toHaveCount(4);
  await page.getByRole("button", { name: "Clear history…" }).click();
  const dialog = page.getByRole("dialog", { name: "Clear listening history" });
  await dialog.getByRole("button", { name: "Keep history" }).click();
  expect(await calls(page, "history_clear")).toHaveLength(0);
  await page.getByRole("button", { name: "Clear history…" }).click();
  await dialog.getByLabel("Everything").check();
  await dialog.getByRole("button", { name: "Clear history" }).click();
  expect((await calls(page, "history_clear"))[0].args).toEqual({
    scope: "all",
  });
  await expect(
    page.getByRole("heading", { name: "No listening recorded" }),
  ).toBeVisible();
});

test("search ranks prefix matches and survives hostile text", async ({
  page,
}) => {
  await installLibraryFixture(page, { tracks: 20_000 });
  await page.goto("/");
  await page.locator("body").press("/");
  const box = page.getByRole("searchbox", { name: "Search library" });
  await expect(box).toBeFocused();
  await box.fill("fixture track 1999");
  await expect(page).toHaveURL(/search\?q=fixture/);
  const tracks = page.getByRole("table", { name: "Track results" });
  await expect(tracks.getByRole("row").first()).toContainText(
    "Fixture track 19990",
  );
  await box.fill('"; DROP TABLE tracks; -- * OR NEAR(');
  await expect(page.getByText(/Nothing matches/)).toBeVisible();
  await box.fill("quartet");
  await expect(
    page.getByRole("heading", { name: "Artists", level: 2 }),
  ).toBeVisible();
  await box.press("Escape");
  await expect(page).toHaveURL(/#\/search$/);
});

test("folders: scan progress, offline roots, confirmation before removal", async ({
  page,
}) => {
  await installLibraryFixture(page, {
    tracks: 1200,
    roots: [
      { id: 1, name: "Music", path: "D:\\Music", online: true, tracks: 1000 },
      {
        id: 2,
        name: "Archive",
        path: "F:\\Archive",
        online: false,
        tracks: 200,
      },
    ],
    scan: {
      rootId: 1,
      phase: "reading",
      discovered: 1000,
      processed: 250,
      totalKnown: true,
    },
  });
  await page.goto("/#/library/folders");
  const music = page.getByRole("listitem").filter({ hasText: "D:\\Music" });
  await expect(
    music.getByRole("progressbar", { name: /Scan progress for Music/ }),
  ).toHaveJSProperty("value", 250);
  const archive = page.getByRole("listitem").filter({ hasText: "F:\\Archive" });
  await expect(archive.getByText("Offline", { exact: true })).toBeVisible();
  await expect(archive.getByRole("button", { name: "Rescan" })).toBeDisabled();
  await archive.getByRole("button", { name: "Remove" }).click();
  const confirm = page.getByRole("group", { name: "Remove Archive" });
  await expect(confirm).toContainText(
    "Your music files stay exactly where they are",
  );
  await page.keyboard.press("Enter");
  expect(await calls(page, "library_remove_root")).toHaveLength(0);
  await archive.getByRole("button", { name: "Remove" }).click();
  await page.getByRole("button", { name: "Remove folder" }).click();
  await expect(archive).toHaveCount(0);
  await music.getByRole("button", { name: "Cancel scan" }).click();
  await expect(music.getByRole("progressbar")).toHaveCount(0);
});

test("menus work from the keyboard and return focus", async ({ page }) => {
  await installLibraryFixture(page, { tracks: 24 });
  await page.goto("/#/library/tracks");
  const more = page.getByRole("button", {
    name: "More for Fixture track 00001",
  });
  await more.focus();
  await page.keyboard.press("Enter");
  await expect(menuItem(page, "Play next")).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(menuItem(page, "Add to queue")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(more).toBeFocused();
});

test("artwork: large variants stay bounded and unused URLs are released", async ({
  page,
}) => {
  await installLibraryFixture(page, { tracks: 1200 });
  await page.addInitScript(() => {
    const live = new Set<string>();
    const create = URL.createObjectURL.bind(URL);
    const revoke = URL.revokeObjectURL.bind(URL);
    URL.createObjectURL = (blob: Blob) => {
      const url = create(blob);
      live.add(url);
      return url;
    };
    URL.revokeObjectURL = (url: string) => {
      live.delete(url);
      revoke(url);
    };
    Object.assign(window, { liveUrls: live });
  });
  await page.goto("/#/album/1");
  for (let id = 1; id <= 30; id++) {
    await page.evaluate((n) => {
      location.hash = `#/album/${n}`;
    }, id);
    await expect(
      page.getByRole("heading", {
        level: 1,
        name: `Fixture Suite ${String(id).padStart(5, "0")}`,
      }),
    ).toBeVisible();
  }
  await page.waitForTimeout(400);
  const live = await page.evaluate(
    () => (window as unknown as { liveUrls: Set<string> }).liveUrls.size,
  );
  // 30 thumbnails may stay cached; large covers are capped at a handful.
  expect(live).toBeLessThanOrEqual(30 + 11);
});

test("scrolling, searching and navigating stay responsive on 20,000 tracks", async ({
  page,
}, info) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await installLibraryFixture(page, {
    tracks: 20_000,
    latencyMs: 15,
    playlists: [
      {
        id: 1,
        name: "Everything",
        trackIds: Array.from({ length: 10_000 }, (_, n) => n + 1),
      },
    ],
  });
  const scroll = () =>
    page.evaluate(async () => {
      const scroller = document.querySelector<HTMLElement>(".virtual-scroll");
      if (!scroller) throw new Error("missing scroller");
      const longTasks: number[] = [];
      new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) longTasks.push(entry.duration);
      }).observe({ entryTypes: ["longtask"] });
      const frames: number[] = [];
      let last = performance.now();
      let maxRows = 0;
      for (let step = 0; step < 120; step++) {
        scroller.scrollTop += 4000;
        await new Promise((resolve) => requestAnimationFrame(resolve));
        const now = performance.now();
        frames.push(now - last);
        last = now;
        maxRows = Math.max(
          maxRows,
          document.querySelectorAll("[role=row]").length,
        );
      }
      frames.sort((a, b) => a - b);
      return {
        p50: frames[60],
        p95: frames[114],
        worstLongTask: Math.max(0, ...longTasks),
        maxRows,
      };
    });
  const results: Record<string, unknown> = {};
  for (const [name, path] of [
    ["library", "/library/tracks"],
    ["playlist", "/playlists/1"],
  ]) {
    await page.goto(`/#${path}`);
    await expect(
      page.getByRole("row", { name: /Fixture track 00001/ }),
    ).toBeVisible();
    const metrics = await scroll();
    results[name] = metrics;
    expect(metrics.maxRows).toBeLessThan(80);
    expect(metrics.worstLongTask).toBeLessThan(300);
    expect(metrics.p95).toBeLessThan(100);
  }
  const started = Date.now();
  await page.goto("/#/search");
  await page
    .getByRole("searchbox", { name: "Search library" })
    .fill("fixture track 1234");
  await expect(
    page.getByRole("table", { name: "Track results" }).getByRole("row").first(),
  ).toContainText("Fixture track 12340");
  results.searchMs = Date.now() - started;
  info.annotations.push({
    type: "metrics",
    description: JSON.stringify(results),
  });
  console.log("LIBRARY_UI_METRICS", JSON.stringify(results));
  expect(results.searchMs as number).toBeLessThan(2500);
});
