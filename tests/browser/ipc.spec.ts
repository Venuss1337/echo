import { expect, test } from "@playwright/test";
import type { AppStatus } from "../../src/generated/contracts";

test("the desktop endpoint handles invalid status and retries without exposing errors", async ({
  page,
}) => {
  const status = { library: "empty" } satisfies AppStatus;
  await page.addInitScript((next) => {
    let calls = 0;
    Object.assign(window, {
      isTauri: true,
      __TAURI_INTERNALS__: {
        transformCallback: () => 1,
        invoke: async (command: string, args: Record<string, unknown>) => {
          if (command.startsWith("player_")) throw "unavailable";
          if (command !== "app_get_status" || Object.keys(args).length !== 0) {
            throw new Error("Unexpected IPC command");
          }
          calls++;
          return calls === 1 ? { library: "unknown" } : next;
        },
      },
    });
  }, status);
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Library unavailable" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Retry connection" }).click();
  await expect(
    page.getByRole("heading", { name: "Your library is empty" }),
  ).toBeVisible();
});

test("desktop startup status progresses without leaving the route", async ({
  page,
}) => {
  await page.addInitScript(() => {
    let library = "opening";
    window.addEventListener("echo-test-storage-ready", () => {
      library = "empty";
    });
    Object.assign(window, {
      isTauri: true,
      __TAURI_INTERNALS__: {
        transformCallback: () => 1,
        invoke: async (command: string) => {
          if (command.startsWith("player_")) throw "unavailable";
          return { library };
        },
      },
    });
  });
  await page.goto("/#/library");
  await expect(
    page.getByRole("heading", { name: "Opening your library" }),
  ).toBeVisible();
  await page.evaluate(() =>
    window.dispatchEvent(new Event("echo-test-storage-ready")),
  );
  await expect(
    page.getByRole("heading", { name: "Your library is empty" }),
  ).toBeVisible();
  await expect(page).toHaveURL(/#\/library$/);
});
