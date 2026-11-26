/** Real WebView2 and production IPC, attached only to the private test desktop. */
import { chromium, expect } from "@playwright/test";
import type {
  PlayerView,
  Station,
  StationPage,
} from "../src/generated/contracts";

const port = Number(process.argv[2]);
if (!Number.isInteger(port) || port < 1 || port > 65535)
  throw new Error("An isolated loopback WebView2 debugging port is required.");
const liveCompatibility = process.argv[3] === "public";
const fixedFixtures = process.argv[3] === "fixture";
const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, {
  timeout: 15_000,
});
try {
  const [context] = browser.contexts();
  const page =
    context.pages()[0] ??
    (await context.waitForEvent("page", { timeout: 5000 }));
  await page.emulateMedia({ reducedMotion: "reduce" });
  const ipc = <T>(command: string, args: object = {}) =>
    page.evaluate(
      ({ command, args }) =>
        (
          window as unknown as {
            __TAURI_INTERNALS__: {
              invoke: (command: string, args: object) => Promise<unknown>;
            };
          }
        ).__TAURI_INTERNALS__.invoke(command, args),
      { command, args },
    ) as Promise<T>;
  const snapshot = () => ipc<PlayerView>("player_snapshot");
  const act = (intent: object) => ipc<PlayerView>("player_act", { intent });
  const edit = {
    id: null,
    name: "Test station",
    country: "",
    language: "",
    tags: "",
  };
  const save = (url: string, name = edit.name) =>
    ipc<Station>("radio_collection_act", {
      action: { kind: "save", station: { ...edit, name, url } },
    });
  await expect
    .poll(async () => {
      try {
        return (await snapshot()).state;
      } catch {
        return undefined;
      }
    })
    .toBe("idle");
  expect((await snapshot()).muted).toBe(true);
  await expect(save("http://127.0.0.1/live")).rejects.toThrow(/invalidSource/);
  await expect(
    ipc("radio_fetch", { url: "https://example.invalid" }),
  ).rejects.toBeDefined();
  const saved = await save("https://station.example.invalid/live");
  await ipc("radio_collection_act", {
    action: { kind: "favorite", id: saved.id, enabled: true },
  });
  await page.reload();
  await expect
    .poll(() =>
      ipc<StationPage>("radio_collection", { shelf: "favorites", offset: 0 }),
    )
    .toMatchObject({
      stations: expect.arrayContaining([
        expect.objectContaining({ id: saved.id, favorite: true }),
      ]),
    });
  await ipc("radio_collection_act", {
    action: { kind: "remove", id: saved.id },
  });

  if (liveCompatibility || fixedFixtures) {
    const directoryAt = performance.now();
    const query = {
      name: liveCompatibility ? "Radio Paradise" : "",
      country: "",
      language: "",
      tag: "",
      sort: "popular",
      offset: 0,
    };
    const result = await ipc<StationPage>("radio_search", {
      request: 900,
      query,
    });
    if (fixedFixtures)
      expect(result.stations.map(({ name }) => name)).toEqual([
        "Fixture MP3",
        "Fixture AAC",
      ]);
    else expect(result.stations.length).toBeGreaterThan(0);
    console.log(
      `${fixedFixtures ? "Seeded" : "Radio Browser"} search returned ${result.stations.length} rows in ${Math.round(performance.now() - directoryAt)} ms`,
    );
    const cachedAt = performance.now();
    const cached = await ipc<StationPage>("radio_search", {
      request: 901,
      query,
    });
    expect(cached).toEqual(result);
    console.log(
      `Cached directory search returned in ${Math.round(performance.now() - cachedAt)} ms`,
    );
    await page.evaluate(() => {
      location.hash = "/radio";
    });
    await expect(
      page.getByRole("heading", { name: "Radio", exact: true }),
    ).toBeVisible();
    await expect(page.getByText("Directory by Radio Browser")).toBeVisible();
    const local = await act({ kind: "play", id: "1" });
    expect(local.current).toBe("1");
    await expect.poll(async () => (await snapshot()).state).toBe("playing");
    for (const codec of ["mp3", "aac"]) {
      const station = fixedFixtures
        ? result.stations.find(
            ({ name }) => name === `Fixture ${codec.toUpperCase()}`,
          )!
        : await save(
            `https://stream.radioparadise.com/${codec}-128`,
            `Radio Paradise ${codec.toUpperCase()}`,
          );
      const start = performance.now();
      await ipc("radio_play", { id: station.id });
      await expect
        .poll(async () => (await snapshot()).live?.status, { timeout: 25_000 })
        .toBe("playing");
      await expect
        .poll(() =>
          ipc<StationPage>("radio_collection", { shelf: "recent", offset: 0 }),
        )
        .toMatchObject({
          stations: expect.arrayContaining([
            expect.objectContaining({ id: station.id }),
          ]),
        });
      console.log(
        `${codec.toUpperCase()} native live start after ${Math.round(performance.now() - start)} ms`,
      );
      expect((await snapshot()).muted).toBe(true);
      expect((await snapshot()).seekable).toBe(false);
      expect((await snapshot()).tracks.map(({ id }) => id)).toEqual(["1"]);
      await page.reload();
      await expect(
        page
          .getByRole("contentinfo", { name: "Player" })
          .getByText(station.name),
      ).toBeVisible();
      expect((await snapshot()).live?.station.id).toBe(station.id);
      await act({ kind: "play", id: "1" });
      await expect.poll(async () => (await snapshot()).live).toBeUndefined();
      await expect.poll(async () => (await snapshot()).volume).toBe(34);
      if (liveCompatibility) {
        await ipc("radio_collection_act", {
          action: { kind: "remove", id: station.id },
        });
      }
    }
    await act({ kind: "stop" });
  }
  console.log("Private WebView2 radio IPC checks passed.");
} finally {
  await browser.close();
}
