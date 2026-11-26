import { Component, type ReactNode } from "react";

/** Keeps an actionable screen visible when the React shell fails to render. */
export class ShellBoundary extends Component<
  { children: ReactNode },
  { failed: boolean }
> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <main className="shell-failure">
        <h1>ECHO could not open</h1>
        <p>Your library and your music are not affected.</p>
        <button
          type="button"
          className="pill"
          onClick={() => window.location.reload()}
        >
          Reload workspace
        </button>
      </main>
    );
  }
}
