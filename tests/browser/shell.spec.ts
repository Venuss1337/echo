import { expect, test } from "@playwright/test";
import { installLibraryFixture, settle } from "./support/library-fixture";

const screenshots = {
  animations: "disabled",
  maxDiffPixelRatio: 0.002,
  stylePath: "tests/browser/support/screenshot.css",
} as const;

test.use({ deviceScaleFactor: 1 });

test("keyboard navigation reaches the library at desktop width", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Your library is empty" }),
  ).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("link", { name: "Skip to content" }),
  ).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("main")).toBeFocused();
  const library = page
    .getByRole("navigation", { name: "Workspace" })
    .getByRole("link", { name: "Library" });
  await library.focus();
  await page.keyboard.press("Enter");
  await expect(library).toHaveAttribute("aria-current", "page");
  await expect(
    page.getByRole("heading", { name: "Library", exact: true }),
  ).toBeVisible();
});

test("every narrow page fits and reduced motion removes transitions", async ({
  page,
}) => {
  await page.setViewportSize({ width: 420, height: 540 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  await installLibraryFixture(page, { tracks: 240, queue: [1, 2] });
  for (const path of [
    "/",
    "/search",
    "/library/tracks",
    "/library/albums",
    "/library/artists",
    "/library/folders",
    "/album/1",
    "/artist/1",
    "/playlists",
    "/favorites",
    "/history",
    "/now-playing",
  ]) {
    await page.goto(`/#${path}`);
    await expect(page.getByRole("contentinfo")).toBeInViewport({ ratio: 1 });
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBe(420);
  }
  await expect(page.locator(".icon-button").first()).toHaveCSS(
    "transition-duration",
    "0s",
  );
});

for (const [name, library, width, height] of [
  ["opening", "opening", 1920, 1080],
  ["unavailable-compact", "unavailable", 420, 540],
] as const) {
  test(`${name} state reference`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.setViewportSize({ width, height });
    await page.addInitScript((state) => {
      Object.assign(window, {
        isTauri: true,
        __TAURI_INTERNALS__: {
          transformCallback: () => 1,
          invoke: async (command: string) => {
            if (command !== "app_get_status") throw "unavailable";
            return { library: state };
          },
        },
      });
    }, library);
    await page.goto("/#/library");
    await expect(
      page.getByRole("heading", {
        name:
          library === "opening"
            ? "Opening your library"
            : "Library unavailable",
      }),
    ).toBeVisible();
    await settle(page);
    await expect(page).toHaveScreenshot(`${name}.png`, screenshots);
  });
}
