import { expect, test, type Page } from "@playwright/test";
import type {
  LiveState,
  PlayerView,
  Station,
} from "../../src/generated/contracts";
import { installLibraryFixture, settle } from "./support/library-fixture";

const stations: Station[] = [
  ["Blue Hour Radio", "GB", "English", "jazz, ambient"],
  ["Studio Północ", "PL", "Polish", "jazz, experimental"],
  ["Radio Circuit", "FR", "French", "electronic, ambient"],
  ["Maré Alta", "BR", "Portuguese", "soul"],
  ["Kiso FM", "JP", "Japanese", "community, jazz"],
  ["North Sound Public Radio", "CA", "English", "community, news"],
].map(([name, country, language, tags], index) => ({
  id: `station:${index}`,
  name,
  country,
  language,
  tags,
  codec: index % 2 ? "AAC" : "MP3",
  bitrate: 128,
  reportedOnline: true,
  manual: false,
  favorite: index === 1,
  artwork: index === 0,
}));

const screenshot = {
  animations: "disabled",
  maxDiffPixelRatio: 0.002,
  stylePath: "tests/browser/support/screenshot.css",
} as const;

test.use({ reducedMotion: "reduce", deviceScaleFactor: 1 });

async function open(
  page: Page,
  route = "/radio",
  player?: Partial<PlayerView>,
) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await installLibraryFixture(page, {
    tracks: 24,
    queue: [3, 14],
    radio: { stations, enabled: true },
    player,
  });
  await page.goto(`/#${route}`);
  await expect(page.locator("#main")).toBeVisible();
}

test("radio discovery, filtering, favorites and station inspector", async ({
  page,
}) => {
  await open(page);
  await expect(
    page.getByRole("heading", { name: "Blue Hour Radio" }),
  ).toBeVisible();
  await expect(page.locator('.radio-feature img[src^="blob:"]')).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("radio-overview.png", screenshot);
  await page.getByLabel("Search stations").fill("Circuit");
  await expect(
    page.getByRole("button", { name: /Radio Circuit FR/ }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Blue Hour Radio" }),
  ).toHaveCount(0);
  await settle(page);
  await expect(page).toHaveScreenshot("radio-search.png", screenshot);
  await page.getByLabel("Search stations").clear();
  await page.getByLabel("Country code").fill("PL");
  await expect(
    page.getByRole("button", { name: /Studio Północ PL/ }),
  ).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("radio-country.png", screenshot);
  await page.getByRole("button", { name: "Favorites", exact: true }).click();
  await expect(
    page.getByRole("button", { name: /Studio Północ PL/ }),
  ).toBeVisible();
  await page.getByRole("button", { name: /Studio Północ PL/ }).click();
  await expect(
    page
      .getByRole("region", { name: "Station", exact: true })
      .getByText("Listed as online"),
  ).toBeVisible();
  await settle(page);
  await expect(page).toHaveScreenshot("radio-favorites.png", screenshot);
});

test("manual collection validates, edits and removes a station", async ({
  page,
}) => {
  await open(page);
  await page.getByRole("button", { name: "My stations", exact: true }).click();
  await page.getByRole("button", { name: "Add station", exact: true }).click();
  const inspector = page.getByRole("region", {
    name: "Add station",
    exact: true,
  });
  await inspector.getByLabel("Name", { exact: true }).fill("Open Air");
  await inspector.getByLabel("Stream URL").fill("http://127.0.0.1/live");
  await inspector.getByRole("button", { name: "Save station" }).click();
  await expect(inspector.getByRole("alert")).toContainText("direct public");
  await inspector
    .getByLabel("Stream URL")
    .fill("https://broadcast.invalid/live");
  await inspector.getByRole("button", { name: "Save station" }).click();
  await page.getByRole("button", { name: "Edit station", exact: true }).click();
  await page.getByLabel("Name", { exact: true }).fill("Open Air FM");
  await page.getByRole("button", { name: "Save station" }).click();
  const details = page.getByRole("region", { name: "Station", exact: true });
  await expect(
    details.getByRole("heading", { name: "Open Air FM" }),
  ).toBeVisible();
  await details
    .getByRole("button", { name: "Remove station", exact: true })
    .click();
  await details
    .getByText("Remove this station from your collection?")
    .locator("..")
    .getByRole("button", { name: "Remove station" })
    .click();
  await expect(page.getByText("No stations here yet.")).toBeVisible();
});

test("superseded search cancels, and a directory outage leaves saved stations available", async ({
  page,
}) => {
  await installLibraryFixture(page, {
    tracks: 0,
    radio: { stations, enabled: false, searchDelayMs: 1200, outage: true },
  });
  await page.goto("/#/radio");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (
          window as unknown as { testLibraryLog: { command: string }[] }
        ).testLibraryLog.some(({ command }) => command === "radio_search"),
      ),
    )
    .toBe(true);
  await page.getByLabel("Search stations").fill("signal");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (
          window as unknown as { testLibraryLog: { command: string }[] }
        ).testLibraryLog.some(({ command }) => command === "radio_cancel"),
      ),
    )
    .toBe(true);
  await expect(page.getByRole("alert")).toContainText(
    "Radio Browser is unavailable",
  );
  await page.getByRole("button", { name: "Favorites", exact: true }).click();
  await expect(
    page.getByRole("button", { name: /Studio Północ PL/ }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Play Studio Północ/ }),
  ).toHaveCount(0);
});

const scenes: [LiveState, PlayerView["state"], string][] = [
  ["playing", "playing", "Playing live"],
  ["connecting", "loading", "Connecting…"],
  ["buffering", "buffering", "Buffering…"],
  ["reconnecting", "loading", "Reconnecting…"],
  ["offline", "error", "Station unavailable."],
];
for (const [status, state, label] of scenes) {
  test(`live exhibition · ${status}`, async ({ page }) => {
    await open(page, "/now-playing", {
      live: {
        station: stations[0],
        status,
        title: state === "playing" ? "Night Shift — Soft Signal" : null,
      },
      state,
      current: null,
      durationMs: null,
      positionMs: 0,
      seekable: false,
    });
    const main = page.getByRole("main");
    await expect(
      main.getByRole("heading", { name: "Blue Hour Radio" }),
    ).toBeVisible();
    await expect(main.getByRole("status")).toContainText(label);
    await expect(page.getByRole("slider", { name: "Seek" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Shuffle" })).toHaveCount(0);
    await settle(page);
    await expect(page).toHaveScreenshot(`radio-${status}.png`, screenshot);
    if (status === "playing") {
      await page.getByRole("button", { name: "Compact player" }).click();
      await page.setViewportSize({ width: 420, height: 640 });
      const compact = page.getByRole("main", { name: "Compact player" });
      await expect(
        compact.getByRole("button", { name: "Pause station" }),
      ).toBeVisible();
      await settle(page);
      await expect(page).toHaveScreenshot("radio-compact.png", screenshot);
      await compact.getByRole("button", { name: "Pause station" }).focus();
      await page.keyboard.press("Enter");
      await expect(compact.getByRole("status")).toContainText(
        "resume at the live edge",
      );
      await compact.getByRole("button", { name: "Stop station" }).click();
      await expect(
        compact.getByRole("heading", { name: "Nothing playing" }),
      ).toBeVisible();
    }
  });
}
