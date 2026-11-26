import { expect, test, type Page } from "@playwright/test";
import type { PlayerView, Station } from "../../src/generated/contracts";
import {
  installLibraryFixture,
  settle,
  type LibraryFixtureOptions,
} from "./support/library-fixture";

/**
 * Text is measured against the pixels actually behind it. Surfaces here are
 * translucent over a photograph, so no arithmetic on tokens can stand in for
 * that. Covers are the extremes; their palettes are the Rust extractor's.
 */
const covers: Record<
  string,
  NonNullable<LibraryFixtureOptions["covers"]>[number]
> = {
  white: {
    image: "sheet:#fafafa:#dddddd",
    palette: ["#363033", "#63545c", "#eeb4c2"],
  },
  black: {
    image: "sheet:#040404:#222222",
    palette: ["#363033", "#63545c", "#eeb4c2"],
  },
  yellow: {
    image: "sheet:#ffe600:#ff9900",
    palette: ["#454119", "#6f671d", "#f3ea9b"],
  },
  red: {
    image: "sheet:#ff0014:#ff8890",
    palette: ["#4a1c1f", "#91272f", "#f39ba2"],
  },
  blue: {
    image: "sheet:#0a14ff:#5a64ff",
    palette: ["#1c1d4a", "#272b91", "#9b9ff3"],
  },
  cyan: {
    image: "sheet:#00f0ff:#007c84",
    palette: ["#1b4649", "#1f7075", "#9beef3"],
  },
};

interface Sample {
  text: string;
  color: string;
  alpha: number;
  large: boolean;
  box: { x: number; y: number; width: number; height: number };
}

/** Every visible run of text, with the colour it is painted in. */
function collect(page: Page) {
  return page.evaluate((): Sample[] => {
    const out: Sample[] = [];
    const walker = document.createTreeWalker(
      document.body,
      NodeFilter.SHOW_TEXT,
    );
    while (walker.nextNode()) {
      const node = walker.currentNode;
      const text = node.textContent?.trim() ?? "";
      const element = node.parentElement;
      if (!text || !element) continue;
      const style = getComputedStyle(element);
      if (
        style.visibility !== "visible" ||
        element.closest(":disabled, [aria-hidden=true] .sleeve-label")
      )
        continue;
      const range = document.createRange();
      range.selectNodeContents(node);
      const { left, top, width, height } = range.getBoundingClientRect();
      if (width < 4 || height < 6) continue;
      // Text scrolled partly behind the floating record is covered, not faint.
      const record = document
        .querySelector(".dock-sleeve .sleeve")
        ?.getBoundingClientRect();
      if (
        record &&
        !element.closest(".dock") &&
        left < record.right + 24 &&
        left + width > record.left &&
        top + height > record.top - 8
      )
        continue;
      const x = left + width / 2;
      const y = top + height / 2;
      if (x < 0 || y < 0 || x > innerWidth || y > innerHeight) continue;
      // Skip text that something else covers, clips or fades out.
      const hit = document.elementFromPoint(x, y);
      if (!hit || !(hit.contains(element) || element.contains(hit))) continue;
      let alpha = 1;
      let clipped = false;
      for (let at: Element | null = element; at; at = at.parentElement) {
        const own = getComputedStyle(at);
        alpha *= Number(own.opacity);
        if (own.clipPath !== "none") clipped = true;
        if (at.id === "main") {
          // The page is masked under the title line and above the dock.
          const { top: from, bottom } = at.getBoundingClientRect();
          if (y < from + 22 || y > bottom - 120) clipped = true;
        }
      }
      if (clipped || alpha < 0.5) continue;
      const size = parseFloat(style.fontSize);
      out.push({
        text: text.slice(0, 40),
        color: style.color,
        alpha,
        large: size >= 24 || (size >= 18.66 && Number(style.fontWeight) >= 700),
        box: { x: left, y: top, width, height },
      });
    }
    return out;
  });
}

async function audit(page: Page) {
  const samples = await collect(page);
  expect(samples.length).toBeGreaterThan(3);
  // Take the lettering away and photograph what was behind it.
  await page.evaluate(() => {
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(
      "*, *::placeholder { color: transparent !important; } svg { visibility: hidden !important; }",
    );
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
  });
  const shot = (await page.screenshot()).toString("base64");
  const results = await page.evaluate(
    async ([png, list]) => {
      const image = new Image();
      image.src = `data:image/png;base64,${png}`;
      await image.decode();
      const canvas = document.createElement("canvas");
      canvas.width = image.width;
      canvas.height = image.height;
      const c = canvas.getContext("2d", { willReadFrequently: true })!;
      c.drawImage(image, 0, 0);
      const channel = (v: number) =>
        v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
      const luminance = (r: number, g: number, b: number) =>
        0.2126 * channel(r / 255) +
        0.7152 * channel(g / 255) +
        0.0722 * channel(b / 255);
      const swatch = document.createElement("canvas").getContext("2d", {
        willReadFrequently: true,
      })!;
      return (list as Sample[]).map((sample) => {
        // Let the browser resolve oklab and color-mix notations.
        swatch.clearRect(0, 0, 1, 1);
        swatch.fillStyle = sample.color;
        swatch.fillRect(0, 0, 1, 1);
        const [r, g, b, a] = swatch.getImageData(0, 0, 1, 1).data;
        const alpha = (a / 255) * sample.alpha;
        const { data } = c.getImageData(
          Math.max(0, Math.floor(sample.box.x)),
          Math.max(0, Math.floor(sample.box.y)),
          Math.max(1, Math.ceil(sample.box.width)),
          Math.max(1, Math.ceil(sample.box.height)),
        );
        const ratios: number[] = [];
        for (let at = 0; at < data.length; at += 4 * 3) {
          const back = luminance(data[at], data[at + 1], data[at + 2]);
          const mixed = [r, g, b].map(
            (v, n) => v * alpha + data[at + n] * (1 - alpha),
          );
          const front = luminance(mixed[0], mixed[1], mixed[2]);
          ratios.push(
            (Math.max(front, back) + 0.05) / (Math.min(front, back) + 0.05),
          );
        }
        ratios.sort((m, n) => m - n);
        // The worst twentieth of the pixels behind the run.
        return {
          text: sample.text,
          need: sample.large ? 3 : 4.5,
          ratio: ratios[Math.floor(ratios.length * 0.05)],
        };
      });
    },
    [shot, samples] as const,
  );
  return results;
}

const pages: [string, string, ((page: Page) => Promise<void>)?][] = [
  ["home", "/"],
  ["tracks", "/library/tracks"],
  ["album", "/album/3"],
  ["artist", "/artist/2"],
  ["playlist", "/playlists/1"],
  ["folders", "/library/folders"],
  [
    "queue",
    "/album/3",
    (page) => page.getByRole("button", { name: "Queue", exact: true }).click(),
  ],
  ["now playing", "/now-playing"],
  [
    "compact",
    "/",
    (page) => page.getByRole("button", { name: "Compact player" }).click(),
  ],
  [
    "menu",
    "/album/3",
    async (page) => {
      const row = page.getByRole("row").nth(2);
      await row.hover();
      await row.getByRole("button", { name: /^More for/ }).click();
    },
  ],
  [
    "dialog",
    "/playlists",
    (page) => page.getByRole("button", { name: "New playlist" }).last().click(),
  ],
];

const station: Station = {
  id: "contrast-station",
  name: "Radio Contrast",
  country: "PL",
  language: "Polish",
  tags: "Jazz",
  codec: "AAC",
  bitrate: 128,
  reportedOnline: true,
  manual: false,
  favorite: false,
  artwork: true,
};

async function liveRadio(page: Page) {
  await page.evaluate((station) => {
    (
      window as unknown as {
        testLibrary: { updatePlayer: (view: Partial<PlayerView>) => void };
      }
    ).testLibrary.updatePlayer({
      live: { station, status: "playing", title: "A late broadcast" },
      state: "playing",
      current: null,
      positionMs: 0,
      durationMs: null,
      seekable: false,
    });
  }, station);
}

pages.push(
  [
    "radio discovery",
    "/radio",
    async (page) => {
      await page.locator('.radio-feature img[src^="blob:"]').waitFor();
    },
  ],
  ["radio playing", "/now-playing", liveRadio],
);

for (const [name, cover] of Object.entries(covers)) {
  test(`text stays readable under a ${name} cover`, async ({ page }) => {
    test.setTimeout(90_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.emulateMedia({ reducedMotion: "reduce" });
    await installLibraryFixture(page, {
      tracks: 240,
      radio: { stations: [station], enabled: true },
      editorial: true,
      covers: [cover],
      queue: [3, 14, 27, 40],
      history: [1, 13, 25, 37, 49],
      favoriteTracks: [2, 5],
      favoriteAlbums: [3],
      playlists: [
        {
          id: 1,
          name: "Night drives",
          description: "Headlights and rain.",
          trackIds: [3, 14, 27, 40, 62],
          lost: [2],
        },
      ],
    });
    const failures: string[] = [];
    for (const [width, height, views] of [
      [1440, 900, pages],
      [
        1280,
        720,
        pages.filter(([view]) => /album|playlist|playing|radio/.test(view)),
      ],
      [420, 640, pages.filter(([view]) => !/menu|dialog/.test(view))],
    ] as const) {
      await page.setViewportSize({ width, height });
      for (const [view, path, act] of views) {
        await page.goto("about:blank");
        await page.goto(`/#${path}`);
        await page.locator("#main").waitFor();
        await settle(page);
        await act?.(page);
        await settle(page);
        for (const { text, need, ratio } of await audit(page)) {
          if (ratio < need)
            failures.push(
              `${view} ${width}: “${text}” ${ratio.toFixed(2)} < ${need}`,
            );
        }
      }
    }
    expect(failures).toEqual([]);
  });
}

test("text stays readable on every placeholder tone", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  const failures: string[] = [];
  await installLibraryFixture(page, {
    tracks: 240,
    editorial: true,
    noArt: true,
    queue: [3, 14],
  });
  // Ten albums spread over the six tones.
  for (let id = 1; id <= 10; id++) {
    await page.goto("about:blank");
    await page.goto(`/#/album/${id}`);
    await page.locator(".leaf-head").waitFor();
    await settle(page);
    for (const { text, need, ratio } of await audit(page))
      if (ratio < need)
        failures.push(`album ${id}: “${text}” ${ratio.toFixed(2)} < ${need}`);
  }
  expect(failures).toEqual([]);
});
