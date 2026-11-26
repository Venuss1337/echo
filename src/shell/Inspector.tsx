import { X } from "lucide-react";
import type { CSSProperties, ReactNode, RefObject } from "react";
import { IconButton } from "../ui";

/** The contextual surfaces share a single place alongside the canvas. */
export type InspectorPane = "queue" | "station";

/** The floating right-hand surface: a title, its actions and a scroll region. */
export function Inspector({
  title,
  actions,
  scroller,
  style,
  onClose,
  children,
}: {
  title: string;
  actions?: ReactNode;
  scroller?: RefObject<HTMLDivElement | null>;
  style?: CSSProperties;
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <aside className="inspector" role="region" aria-label={title} style={style}>
      <header className="inspector-head">
        <h2>{title}</h2>
        {actions}
        <IconButton label={`Close ${title.toLowerCase()}`} onClick={onClose}>
          <X aria-hidden="true" />
        </IconButton>
      </header>
      <div className="inspector-scroll" ref={scroller}>
        {children}
      </div>
    </aside>
  );
}
