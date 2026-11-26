import { chromium, expect } from "@playwright/test";
import type { PlayerIntent, PlayerView } from "../src/generated/contracts";

const port = Number(process.argv[2]);
const phase = process.argv[3];
const title = "ECHO selection 音.wav";
if (!Number.isInteger(port) || port < 1 || port > 65535) {
  throw new Error("An isolated loopback WebView2 debugging port is required.");
}
if (phase !== "play" && phase !== "restart")
  throw new Error("Unknown smoke phase.");

const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, {
  timeout: 15_000,
});
try {
  const [context] = browser.contexts();
  const page =
    context.pages()[0] ??
    (await context.waitForEvent("page", { timeout: 5000 }));
  const transport = page.getByRole("contentinfo", { name: "Player" });
  const more = transport.getByRole("button", { name: "More playback actions" });
  const status = transport.getByRole("status");
  await expect(more).toBeVisible();
  const snapshot = () =>
    page.evaluate(() => {
      const { invoke } = (
        window as Window & {
          __TAURI_INTERNALS__?: { invoke: <T>(cmd: string) => Promise<T> };
        }
      ).__TAURI_INTERNALS__!;
      return invoke<PlayerView>("player_snapshot");
    });
  if (phase === "restart") {
    const restored = await snapshot();
    expect(restored.state).toBe("idle");
    expect(
      restored.tracks.find((row) => row.id === restored.current)?.title,
    ).toBe(title);
    expect(restored.positionMs).toBeGreaterThan(0);
    expect(restored.volume).toBe(0);
    expect(restored.muted).toBe(true);
    console.log(
      "Desktop restart restored the selected track, position and muted preferences in Idle.",
    );
  } else {
    await page.evaluate(async () => {
      const { invoke } = (
        window as Window & {
          __TAURI_INTERNALS__?: {
            invoke: (
              cmd: string,
              args: { intent: PlayerIntent },
            ) => Promise<unknown>;
          };
        }
      ).__TAURI_INTERNALS__!;
      await invoke("player_act", { intent: { kind: "volume", value: 0 } });
      await invoke("player_act", { intent: { kind: "mute", enabled: true } });
    });
    await expect(
      transport.getByRole("button", { name: "Unmute" }),
    ).toBeVisible();
    await more.click();
    await page.getByRole("menuitem", { name: "Open audio files…" }).click();
    // The parent test selects an original WAV via the real Windows modal.
    await expect(status).toHaveText("Playing", { timeout: 8000 });
    await expect(transport.getByText(title, { exact: true })).toBeVisible();
    await expect
      .poll(async () => (await snapshot()).positionMs)
      .toBeGreaterThan(0);
    const before = await snapshot();
    await page.reload();
    await expect(status).toHaveText("Playing");
    expect((await snapshot()).generation).toBe(before.generation);
    await transport.getByRole("button", { name: "Pause", exact: true }).click();
    await expect(status).toHaveText("Paused");
    console.log(
      "Desktop picker, real muted WASAPI playback, WebView reload and pause passed.",
    );
  }
} finally {
  await browser.close();
}
