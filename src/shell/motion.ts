import { flushSync } from "react-dom";

/**
 * The spatial changes ECHO animates. Each has its own choreography in
 * `styles/motion.css`, keyed by `data-motion` on the root element.
 */
export type Motion = "inspector" | "sidebar" | "expand" | "compact" | "record";

/** A paused or hidden window may never produce the frame a capture waits for. */
const CAPTURE_LIMIT_MS = 180;
/** How long a transition may wait for the arriving view to exist. */
const ARRIVAL_STEPS = 25;
const ARRIVAL_STEP_MS = 12;

export function reducedMotion() {
  return (
    typeof matchMedia !== "function" ||
    matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/**
 * Applies a layout change as one spatial movement. The change itself is always
 * immediate and synchronous: only its presentation is animated, so state never
 * depends on an animation finishing.
 *
 * `arrived` holds the capture of the new view until its content is mounted.
 */
export function choreograph(
  motion: Motion,
  update: () => void,
  arrived?: () => boolean,
) {
  const root = document.documentElement;
  if (
    typeof document.startViewTransition !== "function" ||
    reducedMotion() ||
    document.visibilityState === "hidden"
  ) {
    update();
    return;
  }
  let applied = false;
  const apply = () => {
    if (applied) return;
    applied = true;
    flushSync(update);
  };
  root.dataset.motion = motion;
  const transition = document.startViewTransition(async () => {
    apply();
    for (let step = 0; arrived && step < ARRIVAL_STEPS; step++) {
      if (arrived()) return;
      // Rendering is paused here, so wait on the DOM rather than on frames.
      await new Promise((resolve) => setTimeout(resolve, ARRIVAL_STEP_MS));
    }
  });
  const limit = setTimeout(() => {
    if (applied) return;
    apply();
    transition.skipTransition();
  }, CAPTURE_LIMIT_MS);
  const settle = () => {
    clearTimeout(limit);
    if (root.dataset.motion === motion) delete root.dataset.motion;
  };
  // A superseded or skipped transition rejects `ready`; the change still applied.
  transition.ready.catch(() => undefined);
  transition.finished.then(settle, settle);
}
