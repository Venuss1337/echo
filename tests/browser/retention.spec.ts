import { expect, test, type Page } from "@playwright/test";
import { installLibraryFixture } from "./support/library-fixture";

/**
 * Long sessions revisit the same views many times. Whatever a view registers
 * (event listeners, observers, DOM) must be released when it unmounts, or the
 * renderer grows for as long as ECHO stays open. Counts are taken after a
 * forced garbage collection, so only retained objects are measured.
 */
async function retained(page: Page) {
  const session = await page.context().newCDPSession(page);
  await session.send("HeapProfiler.collectGarbage");
  const counters = await session.send("Memory.getDOMCounters");
  await session.detach();
  return { listeners: counters.jsEventListeners, nodes: counters.nodes };
}

const library = {
  tracks: 240,
  queue: [1, 2, 3, 4, 5],
  playlists: [
    {
      id: 1,
      name: "Mix",
      trackIds: Array.from({ length: 40 }, (_, n) => n + 1),
    },
  ],
};

test("revisiting views does not accumulate listeners or nodes", async ({
  page,
}) => {
  await installLibraryFixture(page, library);
  await page.goto("/");
  await page.locator(".titlebar").waitFor();
  const views = [
    "/",
    "/library/tracks",
    "/library/albums",
    "/album/1",
    "/playlists/1",
    "/now-playing",
    "/",
  ];
  const dock = page.getByRole("contentinfo", { name: "Player" });
  const tour = async () => {
    for (const view of views) {
      await page.evaluate((hash) => (location.hash = hash), view);
      await page.waitForTimeout(400);
    }
    // Playback state changes and the compact edition, as a listener does.
    await dock.getByRole("button", { name: /^(Play|Pause)$/ }).click();
    await page.waitForTimeout(400);
    await dock.getByRole("button", { name: /^(Play|Pause)$/ }).click();
    await dock.getByRole("button", { name: "Compact player" }).click();
    await page
      .getByRole("main", { name: "Compact player" })
      .getByRole("button", { name: "Return to full player" })
      .click();
    await page.waitForTimeout(400);
  };
  // The first tour fills caches that legitimately live for the session.
  await tour();
  const before = await retained(page);
  for (let round = 0; round < 4; round++) await tour();
  const after = await retained(page);
  console.log(JSON.stringify({ before, after }));
  // Exact equality is too strict for React's own bookkeeping; growth that
  // scales with the number of tours is not.
  expect(after.listeners - before.listeners).toBeLessThan(20);
  expect(after.nodes - before.nodes).toBeLessThan(200);
});
