import { expect, test, type Page } from "@playwright/test";
import { calls, installLibraryFixture } from "./support/library-fixture";

const drive = (page: Page, changes: object) =>
  page.evaluate((next) => {
    (
      window as unknown as {
        testLibrary: { updatePlayer: (changes: object) => void };
      }
    ).testLibrary.updatePlayer(next);
  }, changes);
const advance = (page: Page) =>
  page.evaluate(() =>
    (
      window as unknown as { testLibrary: { advance: () => void } }
    ).testLibrary.advance(),
  );
const transport = (page: Page) =>
  page.getByRole("contentinfo", { name: "Player" });
const openQueue = async (page: Page) => {
  await transport(page)
    .getByRole("button", { name: "Queue", exact: true })
    .click();
  // Raw mouse input needs the panel to have arrived where it is measured.
  await expect(page.locator("html")).not.toHaveAttribute("data-motion", /./);
  return page.getByRole("region", { name: "Queue" });
};
const upcoming = (page: Page) =>
  page.locator(".queue-list .queue-copy > span").allTextContents();

test("shows only confirmed state and ignores stale revisions", async ({
  page,
}) => {
  await installLibraryFixture(page, { tracks: 60, queue: [1, 2, 3] });
  await page.goto("/#/library/tracks");
  const player = transport(page);
  await expect(player.getByRole("status")).toHaveText("Paused");
  await player
    .getByRole("button", { name: "Play", exact: true })
    .press("Enter");
  await expect(player.getByRole("status")).toHaveText("Playing");
  await drive(page, { state: "error", failure: "outputUnavailable" });
  await expect(player.getByRole("alert")).toContainText(
    "Windows audio output is unavailable",
  );
  await drive(page, { state: "playing", failure: null });
  await expect(player.getByRole("alert")).toHaveCount(0);
  const order = (await calls(page, "player_subscribe")).length;
  expect(order).toBe(1);
});

test("queue edits name stable ids and tolerate playback advancing mid-drag", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await installLibraryFixture(page, { tracks: 60, queue: [1, 2, 3, 4, 5] });
  await page.goto("/#/library/tracks");
  const queue = await openQueue(page);
  await expect
    .poll(() => upcoming(page))
    .toEqual(
      ["00002", "00003", "00004", "00005"].map((n) => `Fixture track ${n}`),
    );
  // Keyboard reorder.
  await queue
    .getByRole("button", { name: /^Reorder Fixture track 00005/ })
    .focus();
  await page.keyboard.press("Alt+ArrowUp");
  await expect.poll(async () => (await upcoming(page))[2]).toContain("00005");

  // Start dragging the first upcoming row, then playback moves onto it.
  const grip = queue.getByRole("button", {
    name: /^Reorder Fixture track 00002/,
  });
  const box = (await grip.boundingBox())!;
  await page.mouse.move(box.x + 5, box.y + 14);
  await page.mouse.down();
  await page.mouse.move(box.x + 5, box.y + 130, { steps: 6 });
  await advance(page);
  await expect(queue.locator(".queue-current")).toContainText("00002");
  await page.mouse.up();
  // The dragged row became current, so its order is untouched.
  await expect
    .poll(() => upcoming(page))
    .toEqual(["00003", "00005", "00004"].map((n) => `Fixture track ${n}`));

  // Drag an upcoming row to the end.
  const third = queue.getByRole("button", {
    name: /^Reorder Fixture track 00003/,
  });
  const start = (await third.boundingBox())!;
  await page.mouse.move(start.x + 5, start.y + 14);
  await page.mouse.down();
  await page.mouse.move(start.x + 5, start.y + 300, { steps: 8 });
  await page.mouse.up();
  await expect
    .poll(() => upcoming(page))
    .toEqual(["00005", "00004", "00003"].map((n) => `Fixture track ${n}`));
  const row = queue.getByRole("listitem").filter({ hasText: "00004" });
  await row.hover();
  await row.getByRole("button", { name: "Remove Fixture track 00004" }).click();
  await expect.poll(async () => (await upcoming(page)).length).toBe(2);
  await queue.getByRole("button", { name: "Clear upcoming" }).click();
  await expect(queue).toContainText("Nothing follows this track");
  const ids = (await calls(page, "player_act"))
    .map(({ args }) => args.intent as { kind: string; id?: string })
    .filter(({ kind }) => kind === "moveBefore");
  expect(ids.every(({ id }) => /^\d+$/.test(id ?? ""))).toBe(true);
});

test("unavailable queue entries are labelled", async ({ page }) => {
  await installLibraryFixture(page, { tracks: 24, queue: [1, 2] });
  await page.goto("/#/library/tracks");
  const queue = await openQueue(page);
  await page.evaluate(() => {
    const api = (
      window as unknown as {
        testLibrary: { updatePlayer: (c: object) => { tracks: object[] } };
      }
    ).testLibrary;
    const { tracks } = api.updatePlayer({});
    api.updatePlayer({
      tracks: tracks.map((row, n) => ({ ...row, unavailable: n === 1 })),
    });
  });
  await expect(queue.getByRole("listitem").first()).toContainText(
    "Unavailable",
  );
});

test("immersive view carries every control and returns where it came from", async ({
  page,
}) => {
  await installLibraryFixture(page, { tracks: 60, queue: [1, 2, 3] });
  await page.goto("/#/album/2");
  await expect(
    page.getByRole("heading", { level: 1, name: "Fixture Suite 00002" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Now Playing", exact: true }).click();
  const scene = page.getByRole("region", { name: "Now Playing" });
  await expect(
    scene.getByRole("heading", { name: "Fixture track 00001" }),
  ).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Workspace" })).toHaveCount(
    0,
  );
  const player = transport(page);
  for (const name of [
    "Shuffle",
    "Previous track",
    "Next track",
    "Mute",
    "Queue",
  ]) {
    await expect(
      player.getByRole("button", { name, exact: true }),
    ).toBeVisible();
  }
  await expect(player.getByRole("slider", { name: "Seek" })).toBeVisible();
  await expect(player.getByRole("slider", { name: "Volume" })).toBeVisible();
  await player.getByRole("button", { name: "Next track" }).click();
  await expect(
    scene.getByRole("heading", { name: "Fixture track 00002" }),
  ).toBeVisible();
  await scene.getByRole("button", { name: /^Favorite/ }).click();
  await expect(
    player.getByRole("button", { name: /^Remove .* from favorites/ }),
  ).toBeVisible();
  // Atmosphere comes from the cover through contrast-checked tokens.
  const ink = await page
    .locator(".immersive")
    .evaluate((element) => getComputedStyle(element).getPropertyValue("--ink"));
  expect(ink.trim()).toMatch(/^#[0-9a-f]{6}$/);
  await page.keyboard.press("Escape");
  await expect(page).toHaveURL(/#\/album\/2$/);
});

test("compact presentation shares the same playback and returns", async ({
  page,
}) => {
  await page.setViewportSize({ width: 420, height: 640 });
  await installLibraryFixture(page, { tracks: 60, queue: [1, 2, 3] });
  await page.goto("/#/library/tracks");
  await page.getByRole("button", { name: "Compact player" }).click();
  const compact = page.getByRole("main", { name: "Compact player" });
  await expect(compact.getByRole("heading")).toHaveText("Fixture track 00001");
  await compact.getByRole("button", { name: "Play", exact: true }).click();
  await expect(compact.getByRole("status")).toHaveText("Playing");
  await compact.getByRole("button", { name: "Next track" }).click();
  await expect(compact.getByRole("heading")).toHaveText("Fixture track 00002");
  expect(
    await page.evaluate(
      () =>
        document.documentElement.scrollWidth === innerWidth &&
        document.documentElement.scrollHeight === innerHeight,
    ),
  ).toBe(true);
  await compact.getByRole("button", { name: "Return to full player" }).click();
  await expect(transport(page).getByRole("status")).toHaveText("Playing");
  await expect(page).toHaveURL(/#\/library\/tracks$/);
  expect(await calls(page, "player_subscribe")).toHaveLength(1);
});

test("shortcuts act outside fields and stay out of the way inside them", async ({
  page,
}) => {
  await installLibraryFixture(page, { tracks: 60, queue: [1, 2, 3] });
  await page.goto("/#/library/tracks");
  const status = transport(page).getByRole("status");
  await page.locator("body").press(" ");
  await expect(status).toHaveText("Playing");
  await page.locator("body").press("n");
  await expect(transport(page)).toContainText("Fixture track 00002");
  await page.locator("body").press("m");
  await expect(
    transport(page).getByRole("button", { name: "Unmute" }),
  ).toBeVisible();
  await page.locator("body").press("q");
  await expect(page.getByRole("region", { name: "Queue" })).toBeVisible();
  await page.locator("body").press("Escape");
  await expect(page.getByRole("region", { name: "Queue" })).toHaveCount(0);
  await page.locator("body").press("/");
  await page
    .getByRole("searchbox", { name: "Search library" })
    .pressSequentially("n m q ");
  await expect(transport(page)).toContainText("Fixture track 00002");
  await expect(page.getByRole("region", { name: "Queue" })).toHaveCount(0);
});

test("only interpolates confirmed playback and honors reduced motion", async ({
  page,
}) => {
  await page.clock.install();
  await installLibraryFixture(page, { tracks: 24, queue: [1, 2] });
  await page.goto("/#/library/tracks");
  const seek = transport(page).getByRole("slider", { name: "Seek" });
  await expect(seek).toHaveValue("74000");
  await drive(page, { state: "playing" });
  await expect(transport(page).getByRole("status")).toHaveText("Playing");
  await page.clock.runFor(650);
  await expect(seek).not.toHaveValue("74000");
  await drive(page, { state: "paused" });
  await expect(seek).toHaveValue("74000");
  // Nobody can see a minimized window, so nothing is interpolated for it.
  const minimize = (next: boolean) =>
    page.evaluate(
      (value) =>
        (
          window as unknown as {
            testLibrary: { minimize: (value: boolean) => void };
          }
        ).testLibrary.minimize(value),
      next,
    );
  await minimize(true);
  await drive(page, { state: "playing" });
  await page.clock.runFor(650);
  await expect(seek).toHaveValue("74000");
  await minimize(false);
  await page.clock.runFor(650);
  await expect(seek).not.toHaveValue("74000");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await drive(page, { state: "paused" });
  await drive(page, { state: "playing" });
  await page.clock.runFor(650);
  await expect(seek).toHaveValue("74000");
});

test("an idle paused scene schedules no continuous rendering", async ({
  page,
}) => {
  await installLibraryFixture(page, { tracks: 24, queue: [1, 2] });
  await page.goto("/#/now-playing");
  await expect(page.locator(".immersive img").first()).toBeVisible();
  await page.waitForTimeout(1200);
  const idle = await page.evaluate(async () => {
    let mutations = 0;
    const observer = new MutationObserver((list) => {
      mutations += list.length;
    });
    observer.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      characterData: true,
    });
    await new Promise((resolve) => setTimeout(resolve, 1000));
    observer.disconnect();
    return {
      mutations,
      animations: document
        .getAnimations()
        .filter(({ playState }) => playState === "running").length,
    };
  });
  expect(idle).toEqual({ mutations: 0, animations: 0 });
});

test("the complete journey holds together", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await installLibraryFixture(page, { tracks: 240, history: [25] });
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Listening room" }),
  ).toBeVisible();
  await page.getByRole("link", { name: "Search" }).click();
  await page
    .getByRole("searchbox", { name: "Search library" })
    .fill("fixture track 00030");
  const hit = page.getByRole("row", { name: /Fixture track 00030/ });
  await hit.getByRole("link", { name: "Fixture Suite 00003" }).click();
  await page
    .locator("#main")
    .getByRole("button", { name: "Play", exact: true })
    .first()
    .click();
  await expect(transport(page).getByRole("status")).toHaveText("Playing");
  await page
    .getByRole("button", { name: "More for Fixture Suite 00003" })
    .click();
  await page.getByRole("menuitem", { name: "Add to playlist…" }).click();
  await page.getByLabel("New playlist name").fill("Evening");
  await page.getByRole("button", { name: "Create", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page
    .getByRole("navigation", { name: "Workspace" })
    .getByRole("link", { name: "Evening" })
    .click();
  await expect(
    page.getByRole("heading", { level: 1, name: "Evening" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: /^Reorder/ })
    .nth(1)
    .focus();
  await page.keyboard.press("Alt+ArrowUp");
  await expect(page.locator(".line-title").first()).toHaveText(
    "Fixture track 00026",
  );
  await page.getByRole("button", { name: "Now Playing", exact: true }).click();
  await expect(page.locator(".immersive")).toBeVisible();
  await page.getByRole("button", { name: "Compact player" }).click();
  await expect(
    page.getByRole("main", { name: "Compact player" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".immersive")).toBeVisible();
});
