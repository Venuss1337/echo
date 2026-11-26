/**
 * The complete listener journey inside the real WebView2, against the
 * production Rust services and the reviewed decoder. The parent PowerShell
 * harness answers the native folder dialog. Nothing here is mocked.
 */
import { chromium, expect, type Page } from "@playwright/test";
import type {
  HistoryPage,
  LibrarySummary,
  PlayerIntent,
  PlayerView,
  PlaylistCard,
  PlaylistPage,
} from "../src/generated/contracts";

const port = Number(process.argv[2]);
const phase = process.argv[3];
const shots = process.argv[4];
if (!Number.isInteger(port) || port < 1 || port > 65535)
  throw new Error("An isolated loopback WebView2 debugging port is required.");
if (phase !== "journey" && phase !== "journey-restart")
  throw new Error("Unknown journey phase.");

const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, {
  timeout: 15_000,
});
const step = (text: string) => console.log(`ok - ${text}`);

async function invoke<T>(page: Page, command: string, args: object = {}) {
  return page.evaluate(
    ([name, payload]) =>
      (
        window as unknown as {
          __TAURI_INTERNALS__: {
            invoke: (cmd: string, args: unknown) => Promise<unknown>;
          };
        }
      ).__TAURI_INTERNALS__.invoke(name as string, payload) as Promise<never>,
    [command, args] as const,
  ) as Promise<T>;
}

try {
  const [context] = browser.contexts();
  const page =
    context.pages()[0] ??
    (await context.waitForEvent("page", { timeout: 5000 }));
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (/Content Security Policy|Refused to/i.test(message.text()))
      errors.push(message.text());
  });
  const player = page.getByRole("contentinfo", { name: "Player" });
  const main = page.locator("#main");
  const status = player.getByRole("status");
  const rail = page.getByRole("navigation", { name: "Workspace" });
  const snapshot = () => invoke<PlayerView>(page, "player_snapshot");
  const summary = () => invoke<LibrarySummary>(page, "library_summary");
  const capture = async (name: string) => {
    if (shots) await page.screenshot({ path: `${shots}/${name}.png` });
  };
  await expect(player.getByRole("button", { name: "Queue" })).toBeVisible({
    timeout: 15_000,
  });

  if (phase === "journey") {
    const act = (intent: PlayerIntent) =>
      invoke<PlayerView>(page, "player_act", { intent });
    await act({ kind: "volume", value: 0 });
    await act({ kind: "mute", enabled: true });
    await capture("01-empty-home");

    await rail.getByRole("link", { name: "Library" }).click();
    await page.getByRole("button", { name: "Add music folder" }).click();
    await expect
      .poll(
        async () => {
          const s = await summary();
          return s.scan == null ? s.tracks : -1;
        },
        { timeout: 40_000 },
      )
      .toBe(24);
    const scanned = await summary();
    expect(scanned.roots[0].lastScan?.status).toBe("complete");
    expect([scanned.albums, scanned.artists]).toEqual([2, 2]);
    step("native folder picker, production scan and index");

    await rail.getByRole("link", { name: "Albums" }).click();
    await expect(page.getByRole("article")).toHaveCount(2);
    await expect(page.locator(".album-cover img").first()).toBeVisible();
    await capture("02-albums");
    step("browse indexed albums with cached artwork through the strict CSP");

    await rail.getByRole("link", { name: "Search" }).click();
    await page
      .getByRole("searchbox", { name: "Search library" })
      .fill("cinder 07");
    const hit = page
      .getByRole("table", { name: "Track results" })
      .getByRole("row")
      .first();
    await expect(hit).toContainText("Cinder 07");
    await hit.getByRole("link", { name: "Ember Field 音" }).click();
    await expect(
      page.getByRole("heading", { level: 1, name: "Ember Field 音" }),
    ).toBeVisible();
    step("search an indexed track and open its album");

    await main
      .getByRole("button", { name: "Play", exact: true })
      .first()
      .click();
    await expect(status).toHaveText("Playing", { timeout: 10_000 });
    await expect
      .poll(async () => (await snapshot()).positionMs, { timeout: 8000 })
      .toBeGreaterThan(0);
    await capture("03-album");
    step("play the album through the one playback coordinator");

    await rail.getByRole("link", { name: "Library" }).click();
    const track = page.getByRole("row", { name: /Tide 03/ });
    await track.hover();
    await track.getByRole("button", { name: "More for Tide 03" }).click();
    await page.getByRole("menuitem", { name: "Add to queue" }).click();
    await expect
      .poll(async () => (await snapshot()).tracks.at(-1)?.title)
      .toBe("Tide 03");
    step("add an indexed track to the queue");

    await page.locator(".virtual-scroll").hover();
    await page.mouse.wheel(0, -4000);
    for (const n of ["01", "02", "03"])
      await page
        .getByRole("row", { name: new RegExp(`Cinder ${n}`) })
        .click({ modifiers: ["Control"] });
    await page
      .getByRole("group", { name: "Selection" })
      .getByRole("button", { name: /Add to playlist/ })
      .click();
    await page.getByLabel("New playlist name").fill("Journey 音");
    await page.getByRole("button", { name: "Create", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await rail.getByRole("link", { name: "Journey 音" }).click();
    await page.getByRole("button", { name: /^Reorder Cinder 03/ }).focus();
    await page.keyboard.press("Alt+ArrowUp");
    await expect(page.locator(".line-title").nth(1)).toHaveText("Cinder 03");
    await capture("04-playlist");
    step("create a playlist, add a selection and reorder it");

    const favorite = page.getByRole("row", { name: /Cinder 03/ });
    await favorite.hover();
    await favorite.getByRole("button", { name: /^Favorite/ }).click();
    await rail.getByRole("link", { name: "Favorites" }).click();
    await expect(page.getByRole("row", { name: /Cinder 03/ })).toBeVisible();
    step("favorite a track");

    await expect
      .poll(
        async () =>
          (
            await invoke<HistoryPage>(page, "history_page", {
              offset: 0,
              limit: 10,
            })
          ).total,
        { timeout: 20_000 },
      )
      .toBeGreaterThan(0);
    step("confirmed listening reached history");

    await page
      .getByRole("button", { name: "Now Playing", exact: true })
      .click();
    await expect(page.locator(".immersive img").first()).toBeVisible();
    await expect
      .poll(() =>
        page
          .locator(".immersive")
          .evaluate((element) =>
            (element as HTMLElement).style.getPropertyValue("--ink"),
          ),
      )
      .toMatch(/^#[0-9a-f]{6}$/);
    const width = await page
      .locator(".work-current img")
      .evaluate(async (image: HTMLImageElement) => {
        await image.decode();
        return image.naturalWidth;
      });
    expect(width).toBeGreaterThan(320);
    await page.waitForTimeout(700);
    await capture("05-now-playing");
    step("immersive view with the large artwork and an extracted palette");

    await page.getByRole("button", { name: "Compact player" }).click();
    const compact = page.getByRole("main", { name: "Compact player" });
    await compact.getByRole("button", { name: "Pause", exact: true }).click();
    await expect(compact.getByRole("status")).toHaveText("Paused");
    await capture("06-compact");
    await compact
      .getByRole("button", { name: "Return to full player" })
      .click();
    await act({ kind: "repeat", mode: "all" });
    const before = await snapshot();
    await page.reload();
    await expect(status).toHaveText("Paused");
    expect((await snapshot()).generation).toBe(before.generation);
    step("compact presentation, reload and shared playback state");
  } else {
    const restored = await snapshot();
    expect(restored.state).toBe("idle");
    expect(restored.volume).toBe(0);
    expect(restored.muted).toBe(true);
    expect(restored.repeat).toBe("all");
    expect(restored.tracks).toHaveLength(13);
    expect(restored.tracks.at(-1)?.title).toBe("Tide 03");
    expect(restored.positionMs).toBeGreaterThan(0);
    step("playback preferences, queue and position");
    expect((await summary()).tracks).toBe(24);
    const [playlist] = await invoke<PlaylistCard[]>(page, "playlists_list");
    expect(playlist.name).toBe("Journey 音");
    const entries = await invoke<PlaylistPage>(page, "playlist_entries", {
      id: playlist.id,
      offset: 0,
      limit: 10,
    });
    expect(entries.rows.map(({ media }) => media.title)).toEqual([
      "Cinder 01",
      "Cinder 03",
      "Cinder 02",
    ]);
    expect(entries.rows[1].media.favorite).toBe(true);
    const history = await invoke<HistoryPage>(page, "history_page", {
      offset: 0,
      limit: 10,
    });
    expect(history.rows[0].listenedMs).toBeGreaterThan(0);
    await expect(
      page.getByRole("heading", { name: "Listening room" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Just heard" }),
    ).toBeVisible();
    await capture("07-home-restored");
    step("library, playlist order, favorites and history");

    // Removing the folder must not take user data with it.
    await invoke(page, "library_remove_root", {
      id: (await summary()).roots[0].id,
    });
    const orphaned = await invoke<PlaylistPage>(page, "playlist_entries", {
      id: playlist.id,
      offset: 0,
      limit: 10,
    });
    expect(
      orphaned.rows.map(({ media }) => [media.title, media.available]),
    ).toEqual([
      ["Cinder 01", false],
      ["Cinder 03", false],
      ["Cinder 02", false],
    ]);
    await rail.getByRole("link", { name: "Journey 音" }).click();
    await expect(page.getByText("3 tracks unavailable")).toBeVisible();
    await capture("08-unavailable");
    step("root removal leaves readable unavailable entries");
  }
  expect(errors).toEqual([]);
} catch (error) {
  if (shots)
    await browser
      .contexts()[0]
      ?.pages()[0]
      ?.screenshot({ path: `${shots}/failure.png` })
      .catch(() => undefined);
  throw error;
} finally {
  await browser.close();
}
