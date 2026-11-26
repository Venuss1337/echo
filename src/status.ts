import { invoke, isTauri } from "@tauri-apps/api/core";
import type { AppStatus } from "./generated/contracts";

/** Reads the one allowed desktop command or returns the browser preview fixture. */
export async function getAppStatus(): Promise<AppStatus> {
  if (!isTauri()) return { library: "empty" };

  const status = await invoke<AppStatus>("app_get_status");
  if (
    status?.library !== "opening" &&
    status?.library !== "empty" &&
    status?.library !== "unavailable"
  ) {
    throw new Error("Unexpected desktop status");
  }
  return status;
}
