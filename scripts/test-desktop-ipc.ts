import { chromium, expect } from "@playwright/test";
import type {
  AppStatus,
  PlayerIntent,
  PlayerView,
} from "../src/generated/contracts";

const port = Number(process.argv[2]);
if (!Number.isInteger(port) || port < 1 || port > 65535) {
  throw new Error("An isolated loopback WebView2 debugging port is required.");
}
const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, {
  timeout: 15_000,
});
try {
  const [context] = browser.contexts();
  const page =
    context.pages()[0] ??
    (await context.waitForEvent("page", { timeout: 5000 }));
  const player = page.getByRole("contentinfo", { name: "Player" });
  await expect(
    player.getByRole("button", { name: "More playback actions" }),
  ).toBeVisible();
  const before = await page.evaluate(async () => {
    const ipc = (
      window as Window & {
        __TAURI_INTERNALS__?: {
          invoke: <T>(cmd: string, args?: object) => Promise<T>;
        };
      }
    ).__TAURI_INTERNALS__;
    if (!ipc) throw new Error("Real desktop IPC missing.");
    const invoke = ipc.invoke;
    const library = await invoke<AppStatus>("app_get_status");
    const snapshot = await invoke<PlayerView>("player_snapshot");
    if (library.library !== "empty" || snapshot.state !== "idle") {
      throw new Error("Expected a ready library and an inert audio session.");
    }
    for (const [command, args] of [
      [
        "player_act",
        { intent: { kind: "play", id: "1", path: "C:/media.wav" } },
      ],
      ["player_act", { intent: { kind: "volume", value: 101 } }],
      ["player_execute", { command: "loadfile" }],
    ] as const) {
      await invoke(command, args).then(
        () => {
          throw new Error(
            `Rejected command unexpectedly succeeded: ${command}`,
          );
        },
        () => undefined,
      );
    }
    return snapshot.volume;
  });
  const volume = before === 37 ? 42 : 37;
  const slider = player.getByRole("slider", { name: "Volume" });
  const setVolume = (value: number) =>
    page.evaluate((value) => {
      const { invoke } = (
        window as unknown as {
          __TAURI_INTERNALS__: {
            invoke: <T>(
              cmd: string,
              args: { intent: PlayerIntent },
            ) => Promise<T>;
          };
        }
      ).__TAURI_INTERNALS__;
      return invoke<PlayerView>("player_act", {
        intent: { kind: "volume", value },
      });
    }, value);
  try {
    await setVolume(volume);
    await expect(slider).toHaveValue(String(volume));
    await page.reload();
    await expect(slider).toHaveValue(String(volume));
  } finally {
    await setVolume(before);
  }
  console.log(
    "Installed WebView2: status, scoped command rejection, volume channel and reload passed.",
  );
} finally {
  await browser.close();
}
