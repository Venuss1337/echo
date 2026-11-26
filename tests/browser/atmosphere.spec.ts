import { expect, test, type Page } from "@playwright/test";
import { calls, installLibraryFixture } from "./support/library-fixture";

const dock = (page: Page) => page.getByRole("contentinfo", { name: "Player" });
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

test("the title bar searches from anywhere and walks the session's history", async ({
  page,
}) => {
  await installLibraryFixture(page, library);
  await page.goto("/#/library/albums");
  const bar = page.locator(".titlebar");
  await expect(bar.getByRole("button", { name: "Back" })).toBeDisabled();
  await expect(bar.getByRole("button", { name: "Forward" })).toBeDisabled();
  const box = bar.getByRole("searchbox", { name: "Search library" });
  await box.fill("fixture track 00030");
  await expect(page).toHaveURL(/search\?q=fixture/);
  await expect(
    page
      .getByRole("table", { name: "Track results" })
      .getByRole("row", { name: /Fixture track 00030/ }),
  ).toBeVisible();
  // Typing more refines the same page instead of stacking history.
  await box.fill("fixture track 00031");
  await expect(page).toHaveURL(/00031/);
  await bar.getByRole("button", { name: "Back" }).click();
  await expect(page).toHaveURL(/#\/library\/albums$/);
  await expect(box).toHaveValue("");
  await bar.getByRole("button", { name: "Forward" }).click();
  await expect(box).toHaveValue("fixture track 00031");
  await bar.getByRole("button", { name: "Back" }).click();
  // Going somewhere new discards what was ahead.
  await page.getByRole("link", { name: "Favorites" }).click();
  await expect(bar.getByRole("button", { name: "Forward" })).toBeDisabled();
});

test("caption buttons ask for exactly three window actions", async ({
  page,
}) => {
  await installLibraryFixture(page, library);
  await page.goto("/");
  const caption = page.getByRole("group", { name: "Window" });
  await caption.getByRole("button", { name: "Minimize" }).click();
  await caption.getByRole("button", { name: "Maximize" }).click();
  // The label follows the window, not the click.
  await caption.getByRole("button", { name: "Restore" }).click();
  await expect(caption.getByRole("button", { name: "Maximize" })).toBeVisible();
  await caption.getByRole("button", { name: "Close" }).click();
  expect((await calls(page, "window_act")).map(({ args }) => args)).toEqual([
    { intent: "minimize" },
    { intent: "toggleMaximize" },
    { intent: "toggleMaximize" },
    { intent: "close" },
  ]);
  const reported = (await calls(page, "window_caption")).at(-1)?.args
    .bounds as { right: number; top: number; width: number; height: number };
  expect(reported).toEqual({ right: 46, top: 0, width: 46, height: 54 });
});

test("a shell that fails to render still has its window", async ({ page }) => {
  await installLibraryFixture(page, library);
  await page.goto("/#/library/tracks");
  // A view the contract forbids, so rendering it throws.
  await page.evaluate(() => {
    const api = (
      window as unknown as {
        testLibrary: { breakPlayer: () => void };
      }
    ).testLibrary;
    api.breakPlayer();
  });
  await expect(
    page.getByRole("heading", { name: "ECHO could not open" }),
  ).toBeVisible();
  await page
    .getByRole("group", { name: "Window" })
    .getByRole("button", { name: "Close" })
    .click();
  expect((await calls(page, "window_act")).at(-1)?.args).toEqual({
    intent: "close",
  });
});

test("only empty title bar drags the window", async ({ page }) => {
  await installLibraryFixture(page, library);
  for (const path of ["/", "/now-playing"]) {
    await page.goto(`/#${path}`);
    await expect(page.locator(".titlebar")).toHaveCSS("app-region", "drag");
    const draggable = await page.evaluate(() =>
      [...document.querySelectorAll("button, a, input, select, [tabindex]")]
        .filter((element) => {
          const { top, height, width } = element.getBoundingClientRect();
          return width > 0 && height > 0 && top < 54;
        })
        .filter((element) => {
          // The nearest declared region decides what a press there does.
          for (
            let node: Element | null = element;
            node;
            node = node.parentElement
          ) {
            const region =
              getComputedStyle(node).getPropertyValue("app-region");
            if (region === "no-drag") return false;
            if (region === "drag") return true;
          }
          return false;
        })
        .map((element) => element.outerHTML.slice(0, 90)),
    );
    expect(draggable).toEqual([]);
  }
});

test("the sidebar folds to a rail, remembers it and keeps its names", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await installLibraryFixture(page, library);
  await page.goto("/#/library/tracks");
  const aside = page.locator(".sidebar");
  const full = (await aside.boundingBox())!.width;
  const canvas = (await page.locator("#main").boundingBox())!.width;
  await page.getByRole("button", { name: "Collapse sidebar" }).click();
  await expect
    .poll(async () => (await aside.boundingBox())!.width)
    .toBeLessThan(90);
  expect((await page.locator("#main").boundingBox())!.width).toBeGreaterThan(
    canvas + 100,
  );
  const nav = page.getByRole("navigation", { name: "Workspace" });
  for (const name of ["Home", "Library", "Favorites", "Playlists", "Mix"])
    await expect(nav.getByRole("link", { name, exact: true })).toBeVisible();
  await nav.getByRole("link", { name: "Artists" }).click();
  await expect(page).toHaveURL(/artists$/);
  await page.reload();
  await expect
    .poll(async () => (await aside.boundingBox())!.width)
    .toBeLessThan(90);
  await page.getByRole("button", { name: "Expand sidebar" }).click();
  await expect.poll(async () => (await aside.boundingBox())!.width).toBe(full);
});

test("a narrow window opens navigation as a drawer and gives focus back", async ({
  page,
}) => {
  await page.setViewportSize({ width: 480, height: 700 });
  await installLibraryFixture(page, library);
  await page.goto("/#/library/tracks");
  const nav = page.getByRole("navigation", { name: "Workspace" });
  await expect(nav).toHaveCount(0);
  const toggle = page.getByRole("button", { name: "Expand sidebar" });
  await toggle.click();
  await expect(nav.getByRole("link", { name: "Home" })).toBeFocused();
  // Nothing behind the drawer can be reached while it is open.
  for (let step = 0; step < 24; step++) {
    await page.keyboard.press("Tab");
    expect(
      await page.evaluate(
        () =>
          document.activeElement === document.body ||
          document.activeElement?.closest(".sidebar, .caption, .skip") != null,
      ),
    ).toBe(true);
  }
  await nav.getByRole("link", { name: "Home" }).focus();
  await page.keyboard.press("Escape");
  await expect(nav).toHaveCount(0);
  await expect(toggle).toBeFocused();
  await toggle.click();
  await nav.getByRole("link", { name: "Favorites" }).click();
  await expect(page).toHaveURL(/favorites$/);
  await expect(nav).toHaveCount(0);
});

for (const [width, height] of [
  [1440, 900],
  [820, 720],
  [420, 640],
]) {
  test(`the dock never covers the end of a page at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height });
    await page.emulateMedia({ reducedMotion: "reduce" });
    await installLibraryFixture(page, {
      ...library,
      history: [1, 13, 25, 37, 49],
      favoriteTracks: Array.from({ length: 30 }, (_, n) => n + 1),
    });
    for (const [path, last] of [
      ["/", ".shelf:last-of-type"],
      ["/library/tracks", ".virtual-row"],
      ["/library/albums", ".album-row"],
      ["/album/2", ".listing .track-row"],
      ["/artist/1", ".shelf:last-of-type"],
      ["/playlists/1", ".virtual-row"],
      ["/favorites", ".virtual-row"],
      ["/library/folders", ".root-card"],
    ]) {
      await page.goto(`/#${path}`);
      await expect(page.locator(last).first()).toBeVisible();
      const end = async () =>
        page.evaluate(async (selector) => {
          for (const scroller of document.querySelectorAll(
            "#main, .virtual-scroll, .scroll-section",
          ))
            scroller.scrollTop = scroller.scrollHeight;
          await new Promise((done) => setTimeout(done, 120));
          const bottoms = [...document.querySelectorAll(selector)].map(
            (row) => row.getBoundingClientRect().bottom,
          );
          return Math.max(...bottoms);
        }, last);
      await end();
      const bottom = await end();
      // The record rises above the dock, so measure from the record.
      const top = await page.evaluate(
        () =>
          document.querySelector(".dock-sleeve")!.getBoundingClientRect().top,
      );
      expect(
        bottom,
        `${path} ends at ${bottom}, dock at ${top}`,
      ).toBeLessThanOrEqual(top + 1);
    }
  });
}

test("keyboard focus in a long list stays clear of the dock", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await installLibraryFixture(page, library);
  await page.goto("/#/playlists/1");
  await expect(page.locator(".virtual-row").first()).toBeVisible();
  const top = (await page.locator(".dock").boundingBox())!.y;
  for (let step = 0; step < 46; step++) {
    await page.keyboard.press("Tab");
    const bottom = await page.evaluate(() => {
      const focused = document.activeElement;
      return focused?.closest(".virtual-scroll")
        ? focused.getBoundingClientRect().bottom
        : 0;
    });
    expect(bottom).toBeLessThanOrEqual(top);
  }
});

for (const width of [420, 520, 640, 760, 900, 1100, 1280, 1920]) {
  test(`every essential dock control fits at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 720 });
    await page.emulateMedia({ reducedMotion: "reduce" });
    await installLibraryFixture(page, { ...library, editorial: true });
    for (const path of ["/library/tracks", "/now-playing"]) {
      await page.goto(`/#${path}`);
      const player = dock(page);
      const names: (string | RegExp)[] = [
        "Shuffle",
        "Previous track",
        "Play",
        "Next track",
        /^Repeat/,
        "Mute",
        "Queue",
        "Compact player",
        "More playback actions",
      ];
      const boxes = [];
      const frame = (await page.locator(".dock").boundingBox())!;
      for (const name of names) {
        const control = player.getByRole("button", {
          name,
          exact: typeof name === "string",
        });
        await expect(control).toBeVisible();
        const box = (await control.boundingBox())!;
        expect(box.x).toBeGreaterThanOrEqual(frame.x);
        expect(box.x + box.width).toBeLessThanOrEqual(frame.x + frame.width);
        expect(box.y + box.height).toBeLessThanOrEqual(frame.y + frame.height);
        boxes.push({ name: String(name), ...box });
      }
      const seek = player.getByRole("slider", { name: "Seek" });
      await expect(seek).toBeVisible();
      boxes.push({ name: "Seek", ...(await seek.boundingBox())! });
      for (const a of boxes)
        for (const b of boxes)
          if (a.name < b.name) {
            const overlap =
              Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x) > 1 &&
              Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y) > 5;
            expect(overlap, `${a.name} over ${b.name}`).toBe(false);
          }
      const copy = page.locator(".dock-copy");
      if (path !== "/now-playing")
        expect((await copy.boundingBox())!.width).toBeGreaterThan(60);
    }
  });
}

test("the volume opens from its button where there is no room for a slider", async ({
  page,
}) => {
  await page.setViewportSize({ width: 820, height: 720 });
  await installLibraryFixture(page, library);
  await page.goto("/#/library/tracks");
  const slider = dock(page).getByRole("slider", { name: "Volume" });
  await dock(page).getByRole("button", { name: "Mute" }).hover();
  const box = (await slider.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.up();
  await expect(slider).toHaveValue(/^(4\d|5\d)$/);
  await dock(page).getByRole("button", { name: "Mute" }).focus();
  await page.keyboard.press("Tab");
  await expect(slider).toBeFocused();
});

test("the queue takes room from the page and gives it back", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await installLibraryFixture(page, library);
  await page.goto("/#/library/tracks");
  const main = page.locator("#main");
  const before = (await main.boundingBox())!;
  const toggle = dock(page).getByRole("button", { name: "Queue", exact: true });
  await toggle.click();
  const panel = page.getByRole("region", { name: "Queue" });
  await expect(panel).toBeVisible();
  const box = (await panel.boundingBox())!;
  const during = (await main.boundingBox())!;
  expect(during.x + during.width).toBeLessThanOrEqual(box.x);
  expect(box.x + box.width).toBeLessThanOrEqual(1440 - 8);
  await expect(page.locator("html")).not.toHaveAttribute("data-motion", /./);
  await panel.getByRole("button", { name: "Close queue" }).click();
  await expect(panel).toHaveCount(0);
  expect(await main.boundingBox()).toEqual(before);
});

for (const width of [860, 420]) {
  test(`without room the queue floats above the dock at ${width}`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 700 });
    await page.emulateMedia({ reducedMotion: "reduce" });
    await installLibraryFixture(page, library);
    for (const path of ["/library/tracks", "/now-playing"]) {
      await page.goto(`/#${path}`);
      const toggle = dock(page).getByRole("button", {
        name: "Queue",
        exact: true,
      });
      await toggle.click();
      const panel = page.getByRole("region", { name: "Queue" });
      const box = (await panel.boundingBox())!;
      const frame = (await page.locator(".dock").boundingBox())!;
      expect(box.width).toBeGreaterThan(300);
      expect(box.y + box.height).toBeLessThanOrEqual(frame.y);
      expect(box.y).toBeGreaterThanOrEqual(46);
      await panel
        .getByRole("button", { name: /^Remove Fixture track 00005/ })
        .focus();
      await page.keyboard.press("Enter");
      await expect(panel.getByRole("listitem")).toHaveCount(3);
      await page.evaluate(() => location.reload());
      await expect(toggle).toBeVisible();
    }
  });
}

test("escape leaves what is on screen, and a widened window has no drawer", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await installLibraryFixture(page, library);
  await page.goto("/#/library/tracks");
  await dock(page).getByRole("button", { name: "Queue", exact: true }).click();
  await dock(page).getByRole("button", { name: "Compact player" }).click();
  const card = page.getByRole("main", { name: "Compact player" });
  await expect(card).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(card).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Queue" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("region", { name: "Queue" })).toHaveCount(0);
  await page.getByRole("button", { name: "Collapse sidebar" }).click();
  await page.setViewportSize({ width: 900, height: 700 });
  await page.getByRole("button", { name: "Expand sidebar" }).click();
  await expect(page.locator(".drawer-scrim")).toBeVisible();
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(page.locator(".drawer-scrim")).toHaveCount(0);
  await expect(page.locator("#main")).toHaveJSProperty("inert", false);
});

test("menus near the dock open upwards and the caption stays out of the way", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await installLibraryFixture(page, library);
  await page.goto("/#/library/tracks");
  const more = dock(page).getByRole("button", {
    name: "More playback actions",
  });
  await more.click();
  const menu = page.getByRole("menu", { name: "Playback" });
  const box = (await menu.boundingBox())!;
  expect(box.y + box.height).toBeLessThanOrEqual(
    (await more.boundingBox())!.y + 1,
  );
  // The press that dismisses a menu must reach the page, not drag the window.
  const region = await page.evaluate(() => {
    const scrim = document.elementFromPoint(800, 20)!;
    return getComputedStyle(scrim).getPropertyValue("app-region");
  });
  expect(region).toBe("no-drag");
  await page.mouse.click(800, 20);
  await expect(menu).toHaveCount(0);
  await more.click();
  await page.getByRole("menuitem", { name: "Open audio files…" }).click();
  expect(await calls(page, "player_pick")).toHaveLength(1);
  await more.click();
  await page.getByRole("menuitem", { name: "Stop" }).click();
  expect((await calls(page, "player_act")).at(-1)?.args).toEqual({
    intent: { kind: "stop" },
  });
});

test("a modal traps focus, leaves the window controls alone and restores focus", async ({
  page,
}) => {
  await installLibraryFixture(page, library);
  await page.goto("/#/playlists");
  const opener = page.getByRole("button", { name: "New playlist" }).last();
  await opener.focus();
  await page.keyboard.press("Enter");
  const form = page.getByRole("dialog", { name: "New playlist" });
  await expect(form.getByLabel("Name")).toBeFocused();
  for (let step = 0; step < 8; step++) {
    await page.keyboard.press(step % 3 ? "Tab" : "Shift+Tab");
    expect(
      await form.evaluate((node) => node.contains(document.activeElement)),
    ).toBe(true);
  }
  // Shortcuts stay quiet while the modal is open, even from its own padding.
  await form.click({ position: { x: 6, y: 6 } });
  await page.keyboard.press("q");
  await page.keyboard.press("m");
  await expect(page.getByRole("region", { name: "Queue" })).toHaveCount(0);
  expect(await calls(page, "player_act")).toHaveLength(0);
  await expect(page.locator(".shell")).toHaveJSProperty("inert", true);
  await page
    .getByRole("group", { name: "Window" })
    .getByRole("button", { name: "Minimize" })
    .click();
  expect(await calls(page, "window_act")).toHaveLength(1);
  await form.getByRole("button", { name: "Cancel" }).click();
  await expect(form).toHaveCount(0);
  await expect(page.locator(".shell")).toHaveJSProperty("inert", false);
});

test("the room is lit by what plays, a record page lends its own light", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await installLibraryFixture(page, { ...library, editorial: true });
  const ink = () =>
    page.evaluate(() =>
      document.documentElement.style.getPropertyValue("--ink"),
    );
  const dockInk = () =>
    page
      .locator(".dock-slot")
      .evaluate((element) =>
        (element as HTMLElement).style.getPropertyValue("--ink"),
      );
  await page.goto("/#/library/tracks");
  // The fixture's first palette belongs to the loaded track's album.
  await expect.poll(ink).toBe("#3b1a23");
  await page.goto("/#/album/4");
  await expect.poll(ink).toBe("#10303a");
  expect(await dockInk()).toBe("#3b1a23");
  await page.goto("/#/favorites");
  await expect.poll(ink).toBe("#3b1a23");
});

test("the photograph dims as the page is read", async ({ page }) => {
  await installLibraryFixture(page, { ...library, history: [1, 13, 25, 37] });
  await page.goto("/");
  const shell = page.locator(".shell");
  await expect(shell).toHaveAttribute("data-exposure", "lit");
  await page.locator("#main").evaluate((main) => {
    main.scrollTop = 400;
  });
  await expect(shell).toHaveAttribute("data-exposure", "low");
  await page.locator("#main").evaluate((main) => {
    main.scrollTop = 0;
  });
  await expect(shell).toHaveAttribute("data-exposure", "lit");
  await page.getByRole("link", { name: "Favorites" }).click();
  await expect(shell).toHaveAttribute("data-exposure", "low");
});

test("motion settles, and nothing keeps moving while music plays", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await installLibraryFixture(page, {
    ...library,
    player: { state: "playing" },
  });
  await page.goto("/#/library/albums");
  await page.getByRole("link", { name: "Fixture Suite 00002" }).last().click();
  await expect(
    page.getByRole("heading", { level: 1, name: "Fixture Suite 00002" }),
  ).toBeVisible();
  for (const name of ["Queue", "Queue", "Now Playing", "Leave Now Playing"]) {
    await dock(page).getByRole("button", { name, exact: true }).click();
    await expect(page.locator("html")).not.toHaveAttribute("data-motion", /./);
  }
  await page.getByRole("button", { name: "Compact player" }).click();
  await expect(
    page.getByRole("main", { name: "Compact player" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".dock")).toBeVisible();
  await page.waitForTimeout(1400);
  expect(
    await page.evaluate(
      () =>
        document
          .getAnimations()
          .filter(({ playState }) => playState === "running").length,
    ),
  ).toBe(0);
});

test("reduced motion applies every change without a transition", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await installLibraryFixture(page, library);
  await page.addInitScript(() => {
    const started: number[] = [];
    Object.assign(window, { transitions: started });
    const original = document.startViewTransition?.bind(document);
    document.startViewTransition = ((update: () => void) => {
      started.push(1);
      return original(update);
    }) as typeof document.startViewTransition;
  });
  await page.goto("/#/library/albums");
  await dock(page).getByRole("button", { name: "Queue", exact: true }).click();
  await page.getByRole("button", { name: "Collapse sidebar" }).click();
  await dock(page)
    .getByRole("button", { name: "Now Playing", exact: true })
    .click();
  await expect(page.locator(".immersive")).toBeVisible();
  expect(
    await page.evaluate(
      () => (window as unknown as { transitions: number[] }).transitions.length,
    ),
  ).toBe(0);
});

test("the exhibition plays its neighbours from the wall", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await installLibraryFixture(page, {
    tracks: 240,
    queue: [1, 13, 25],
    editorial: true,
  });
  await page.goto("/#/now-playing");
  const wall = page.getByRole("region", { name: "Now Playing" });
  await expect(wall.getByRole("button", { name: /^Previous:/ })).toHaveCount(0);
  await wall.getByRole("button", { name: /^Play next:/ }).click();
  await expect(wall.getByRole("button", { name: /^Previous:/ })).toBeVisible();
  expect((await calls(page, "player_act")).at(-1)?.args).toEqual({
    intent: { kind: "next" },
  });
});
