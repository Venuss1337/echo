/**
 * The page's half of the title bar acceptance test, over an isolated loopback
 * WebView2 debugging port. The PowerShell harness owns the native half and
 * calls one step at a time; each prints a single line of JSON.
 */
import { chromium, expect } from "@playwright/test";
import { createInterface } from "node:readline";
import type { PlayerView } from "../src/generated/contracts";

const port = Number(process.argv[2]);
const step = process.argv[3];
const argument = process.argv[4];
if (!Number.isInteger(port) || port < 1 || port > 65535)
  throw new Error("An isolated loopback WebView2 debugging port is required.");

const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, {
  timeout: 15_000,
});
try {
  const [context] = browser.contexts();
  const page =
    context.pages()[0] ??
    (await context.waitForEvent("page", { timeout: 5000 }));
  // Reuse the connection over the soak. Each new Playwright client can
  // inject browser-side automation helpers, changing the retention sample.
  if (step === "session") {
    const commands = createInterface({ input: process.stdin });
    for await (const line of commands) {
      const { step, argument } = JSON.parse(line);
      await run(step, argument);
    }
  } else {
    await run(step, argument);
  }

  async function run(step: string, argument: string) {
    // Measurement steps run in any mode, including the title-bar-less mini.
    const measuring = step === "metrics" || step === "pressure";
    if (!measuring && (step !== "mini" || argument === "enter")) {
      await page.locator(".titlebar").waitFor({ timeout: 15_000 });
    }
    switch (step) {
      case "layout": {
        // Centres of every control in the band and of the empty parts between.
        const layout = await page.evaluate(() => {
          const box = (selector: string) => {
            const element = document.querySelector(selector);
            if (!element) return null;
            const { left, top, width, height } =
              element.getBoundingClientRect();
            return { x: left + width / 2, y: top + height / 2, width, height };
          };
          const bar = document
            .querySelector(".titlebar")!
            .getBoundingClientRect();
          const search = document
            .querySelector(".search-field")!
            .getBoundingClientRect();
          const caption = document
            .querySelector(".caption")!
            .getBoundingClientRect();
          return {
            scale: devicePixelRatio,
            width: innerWidth,
            height: innerHeight,
            controls: {
              brand: box(".sidebar-head a"),
              sidebar: box(".sidebar-toggle"),
              search: box(".search-field input"),
              minimize: box(".caption button:nth-child(1)"),
              close: box(".caption .close"),
              navigation: box(".sidebar nav a"),
              content: box(".hero-title, .page h1"),
            },
            maximize: box(".caption button:nth-child(2)"),
            empty: [
              { x: (search.right + caption.left) / 2, y: bar.top + 26 },
              { x: search.right + 40, y: bar.top + 12 },
              { x: search.right + 40, y: bar.bottom - 6 },
              { x: 190, y: 24 },
            ],
          };
        });
        console.log(JSON.stringify(layout));
        break;
      }
      case "arm":
        await page.evaluate(() => {
          const seen: string[] = [];
          Object.assign(window, { captionSeen: seen });
          const button = document.querySelector(
            ".caption button:nth-child(2)",
          ) as HTMLElement;
          new MutationObserver(() =>
            seen.push(
              [
                button.dataset.hovered ? "hovered" : "",
                button.dataset.pressed ? "pressed" : "",
                button.getAttribute("aria-label"),
              ].join(" "),
            ),
          ).observe(button, { attributes: true });
        });
        console.log("{}");
        break;
      case "seen":
        console.log(
          JSON.stringify(
            await page.evaluate(
              () =>
                (window as unknown as { captionSeen: string[] }).captionSeen,
            ),
          ),
        );
        break;
      case "label":
        console.log(
          JSON.stringify(
            await page
              .locator(".caption button")
              .nth(1)
              .getAttribute("aria-label"),
          ),
        );
        break;
      case "press":
        // A real DOM click on the drawn button, not a call to the command.
        await page
          .getByRole("group", { name: "Window" })
          .getByRole("button", { name: argument, exact: true })
          .click({ noWaitAfter: true });
        console.log("{}");
        break;
      case "dialog": {
        // A modal must not take the window's own buttons with it.
        await page
          .getByRole("button", { name: "New playlist" })
          .first()
          .click();
        await page.getByRole("dialog", { name: "New playlist" }).waitFor();
        const usable = await page.evaluate(() => {
          const close = document.querySelector(".caption .close")!;
          const { left, top, width, height } = close.getBoundingClientRect();
          const hit = document.elementFromPoint(
            left + width / 2,
            top + height / 2,
          );
          return close.contains(hit) && !close.closest("[inert]");
        });
        await page.keyboard.press("Escape");
        await page.getByRole("dialog").waitFor({ state: "detached" });
        console.log(JSON.stringify(usable));
        break;
      }
      case "refused": {
        // The page may not move, resize or otherwise command the window.
        const refused = await page.evaluate(async () => {
          const { invoke } = (
            window as unknown as {
              __TAURI_INTERNALS__: {
                invoke: (command: string, args?: object) => Promise<PlayerView>;
              };
            }
          ).__TAURI_INTERNALS__;
          const outcomes: Record<string, boolean> = {};
          for (const [command, args] of [
            [
              "plugin:window|set_size",
              { value: { Logical: { width: 1, height: 1 } } },
            ],
            [
              "plugin:window|set_position",
              { value: { Logical: { x: 0, y: 0 } } },
            ],
            ["plugin:window|set_always_on_top", { value: true }],
            ["plugin:window|start_dragging", {}],
            ["plugin:window|close", {}],
            ["plugin:window|set_fullscreen", { value: true }],
            ["window_act", { intent: "setSize" }],
            [
              "window_caption",
              { bounds: { right: 0, top: 0, width: 900, height: 700 } },
            ],
          ] as const) {
            outcomes[command] = await invoke(command, args).then(
              () => false,
              () => true,
            );
          }
          return outcomes;
        });
        console.log(JSON.stringify(refused));
        break;
      }
      case "mini": {
        if (argument === "enter") {
          await page.evaluate(async () => {
            const { invoke } = (
              window as unknown as {
                __TAURI_INTERNALS__: {
                  invoke: (command: string, args: object) => Promise<unknown>;
                };
              }
            ).__TAURI_INTERNALS__;
            await invoke("window_act", { intent: "enterMini" });
          });
        } else if (argument === "leave") {
          await page
            .getByRole("button", { name: "Return to full player" })
            .click();
        } else if (argument === "reload") {
          await page.reload();
        } else {
          throw new Error("Expected a mini-player test action");
        }
        const compact = page.getByRole("main", { name: "Compact player" });
        await compact.waitFor({
          state: argument === "leave" ? "hidden" : "visible",
        });
        console.log(JSON.stringify(context.pages().length === 1));
        break;
      }
      case "metrics": {
        // Renderer counters. By default no garbage collection is forced, so the
        // sample reflects the live process; "gc" collects first and so measures
        // only what is still reachable, which is what a leak check needs.
        const session = await context.newCDPSession(page);
        if (argument === "gc")
          await session.send("HeapProfiler.collectGarbage");
        await session.send("Performance.enable");
        const { metrics } = await session.send("Performance.getMetrics");
        const counters = await session.send("Memory.getDOMCounters");
        const pick = (name: string) =>
          metrics.find((metric) => metric.name === name)?.value ?? 0;
        const images = await page.evaluate(() => {
          let pixels = 0;
          let count = 0;
          for (const image of document.images) {
            if (!image.complete || !image.naturalWidth) continue;
            count += 1;
            pixels += image.naturalWidth * image.naturalHeight;
          }
          return { count, megapixels: Math.round(pixels / 1e4) / 100 };
        });
        // Composited layers own GPU tiles. Their painted area, in device
        // pixels, is what the GPU process has to keep rasterized.
        await session.send("LayerTree.enable");
        const tree = await new Promise<
          { drawsContent: boolean; width: number; height: number }[]
        >((done) => {
          session.once("LayerTree.layerTreeDidChange", (event) =>
            done(event.layers ?? []),
          );
          void session.send("Runtime.evaluate", {
            expression: "requestAnimationFrame(() => {})",
          });
          setTimeout(() => done([]), 1500);
        });
        await session.send("LayerTree.disable");
        await session.detach();
        const painted = tree.filter((layer) => layer.drawsContent);
        const scale = await page.evaluate(() => devicePixelRatio);
        console.log(
          JSON.stringify({
            layers: painted.length,
            layerMegapixels:
              Math.round(
                (painted.reduce(
                  (sum, layer) => sum + layer.width * layer.height,
                  0,
                ) *
                  scale *
                  scale) /
                  1e4,
              ) / 100,
            jsHeapUsedMiB: Math.round(pick("JSHeapUsedSize") / 1048576),
            jsHeapTotalMiB: Math.round(pick("JSHeapTotalSize") / 1048576),
            nodes: counters.nodes,
            documents: counters.documents,
            listeners: counters.jsEventListeners,
            images,
          }),
        );
        break;
      }
      case "pressure": {
        // Asks Chromium to release what it can. Memory that returns after this
        // was cache or garbage, not retained application state.
        const session = await context.newCDPSession(page);
        await session.send("HeapProfiler.collectGarbage");
        await session.send("Memory.simulatePressureNotification", {
          level: "critical",
        });
        await session.detach();
        console.log("{}");
        break;
      }
      case "shot":
        await page.screenshot({ path: argument });
        console.log("{}");
        break;
      case "profile": {
        await page.evaluate(async (mode) => {
          const { invoke } = (
            window as unknown as {
              __TAURI_INTERNALS__: {
                invoke: (command: string, args?: object) => Promise<unknown>;
              };
            }
          ).__TAURI_INTERNALS__;
          let route = "/";
          switch (mode) {
            case "idle":
              const current = await invoke("player_snapshot");
              if (
                current.live ||
                ["loading", "playing", "paused", "buffering"].includes(
                  current.state,
                )
              ) {
                await invoke("player_act", { intent: { kind: "stop" } });
              }
              break;
            case "local":
              await invoke("player_act", { intent: { kind: "play", id: "1" } });
              break;
            case "radio":
              await invoke("radio_play", {
                id: "local:00000000000000000000000000000001",
              });
              break;
            case "library":
              route = "/library";
              break;
            default:
              throw new Error("Unknown profiling mode");
          }
          location.hash = route;
        }, argument);
        if (argument === "local" || argument === "radio") {
          await expect
            .poll(() =>
              page.evaluate(async (mode) => {
                const { invoke } = (
                  window as unknown as {
                    __TAURI_INTERNALS__: {
                      invoke: (command: string) => Promise<PlayerView>;
                    };
                  }
                ).__TAURI_INTERNALS__;
                const view = await invoke("player_snapshot");
                return mode === "radio"
                  ? view.live?.status === "playing" && !view.seekable
                  : view.state === "playing" && !view.live;
              }, argument),
            )
            .toBe(true);
        }
        console.log("{}");
        break;
      }
      default:
        throw new Error(`Unknown step ${step}`);
    }
  }
} finally {
  await browser.close();
}
