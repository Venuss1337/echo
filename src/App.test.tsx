import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router";
import { afterEach, expect, it, vi } from "vitest";
import App from "./App";
import { getAppStatus } from "./status";

vi.mock("./status");

afterEach(cleanup);

it.each([
  ["empty", "Your library is empty"],
  ["unavailable", "Library unavailable"],
] as const)("presents the %s IPC state", async (library, heading) => {
  vi.mocked(getAppStatus).mockResolvedValue({ library });
  render(
    <QueryClientProvider client={new QueryClient()}>
      <MemoryRouter>
        <App />
      </MemoryRouter>
    </QueryClientProvider>,
  );
  expect(await screen.findByRole("heading", { name: heading })).toBeVisible();
  expect(screen.getByRole("contentinfo")).toHaveTextContent("Nothing playing");
});
