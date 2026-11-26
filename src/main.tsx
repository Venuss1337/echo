import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { HashRouter } from "react-router";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import App from "./App";
import { ShellBoundary } from "./shell/ShellBoundary";
import { Caption } from "./shell/TitleBar";
import "./styles/index.css";

const root = document.getElementById("root");
if (!root) throw new Error("Missing ECHO root element");
const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: Infinity } },
});

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <HashRouter>
        <ShellBoundary>
          <App />
        </ShellBoundary>
        {/* Outside the boundary: a window must be closable whatever fails. */}
        <Caption />
      </HashRouter>
    </QueryClientProvider>
  </StrictMode>,
);
