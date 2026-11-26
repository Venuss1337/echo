import { defineConfig } from "@playwright/test";

const endpoint = "http://localhost:4173";
export default defineConfig({
  testDir: "./tests/browser",
  use: { baseURL: endpoint, browserName: "chromium", headless: true },
  webServer: {
    command: "bun run preview --host localhost --port 4173 --strictPort",
    url: endpoint,
    reuseExistingServer: false,
  },
});
