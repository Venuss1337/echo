import { FolderPlus, HardDrive, RefreshCw, Trash2 } from "lucide-react";
import { useState } from "react";
import type { LibraryRootView, ScanView } from "../generated/contracts";
import {
  count,
  plural,
  reportSummary,
  rootStatus,
  scanDetail,
  scanHeadline,
  when,
} from "./format";
import { useLibrary } from "./LibraryProvider";

/** Live counters for the running scan. Counts are announced rarely: only the
 * headline is a status region, because the numbers change several times a second. */
export function ScanProgress({
  scan,
  name,
  onCancel,
  busy,
}: {
  scan: ScanView;
  name: string;
  onCancel: () => void;
  busy: boolean;
}) {
  return (
    <div className="scan-progress">
      <div className="scan-line">
        <strong role="status">{scanHeadline(scan, name)}</strong>
        <span aria-hidden="true">{scanDetail(scan)}</span>
        <button
          className="quiet"
          type="button"
          disabled={busy}
          onClick={onCancel}
        >
          Cancel scan
        </button>
      </div>
      {scan.totalKnown ? (
        <progress
          aria-label={`Scan progress for ${name}`}
          max={Math.max(1, scan.discovered)}
          value={scan.processed}
        />
      ) : (
        <progress aria-label={`Scan progress for ${name}`} />
      )}
      {scan.queued > 0 && (
        <p className="scan-note">{plural(scan.queued, "folder")} waiting.</p>
      )}
    </div>
  );
}

function RootCard({ root }: { root: LibraryRootView }) {
  const { summary, busy, scan, removeRoot, cancelScan } = useLibrary();
  const [confirming, setConfirming] = useState(false);
  const live = summary?.scan?.rootId === root.id ? summary.scan : null;
  const last = when(root.lastScanAt);
  return (
    <li className="root-card">
      <div className="root-head">
        <HardDrive aria-hidden="true" />
        <div className="root-names">
          <h3>{root.name}</h3>
          <p className="root-path" title={root.path}>
            {root.path}
          </p>
        </div>
        <span className={root.online ? "badge" : "badge badge-warn"}>
          {rootStatus(root)}
        </span>
      </div>
      <dl className="root-stats">
        <div>
          <dt>Tracks</dt>
          <dd>{count(root.tracks)}</dd>
        </div>
        <div>
          <dt>Last full scan</dt>
          <dd>{last ?? "Not yet"}</dd>
        </div>
      </dl>
      {root.lastScan && !live && (
        <p className="scan-note">{reportSummary(root.lastScan)}</p>
      )}
      {!root.online && (
        <p className="scan-note">
          This folder can’t be reached, perhaps because a drive is unplugged.
          Its {plural(root.tracks, "track")} stay in your library as unavailable
          and return when the folder does.
        </p>
      )}
      {live && (
        <ScanProgress
          scan={live}
          name={root.name}
          busy={busy === "scan"}
          onCancel={cancelScan}
        />
      )}
      {confirming ? (
        <div
          className="confirm"
          role="group"
          aria-label={`Remove ${root.name}`}
        >
          <p>
            Remove <strong>{root.name}</strong> from ECHO? Your music files stay
            exactly where they are. ECHO only forgets its index of{" "}
            {plural(root.tracks, "track")}.
          </p>
          <div className="root-actions">
            <button
              className="quiet"
              type="button"
              onClick={() => {
                setConfirming(false);
                removeRoot(root.id);
              }}
            >
              <Trash2 aria-hidden="true" /> Remove folder
            </button>
            <button
              className="quiet"
              type="button"
              autoFocus
              onClick={() => setConfirming(false)}
            >
              Keep folder
            </button>
          </div>
        </div>
      ) : (
        <div className="root-actions">
          <button
            className="quiet"
            type="button"
            disabled={!root.online || live != null || busy != null}
            onClick={() => scan(root.id)}
          >
            <RefreshCw aria-hidden="true" /> Rescan
          </button>
          <button
            className="quiet"
            type="button"
            disabled={busy === "remove"}
            onClick={() => setConfirming(true)}
          >
            <Trash2 aria-hidden="true" /> Remove
          </button>
        </div>
      )}
    </li>
  );
}

export function Folders() {
  const { native, summary, fault, busy, addRoot, scan, dismissFault } =
    useLibrary();
  const roots = summary?.roots ?? [];
  return (
    <section
      className="library-section scroll-section"
      aria-labelledby="folders-title"
    >
      <div className="section-bar">
        <h2 id="folders-title" className="section-title">
          <span className="named-by-tab">Music folders </span>
          <span className="section-count">
            {native ? plural(roots.length, "folder") : ""}
          </span>
        </h2>
        <div className="bar-actions">
          {roots.length > 1 && (
            <button
              className="quiet"
              type="button"
              disabled={busy != null || summary?.scan != null}
              onClick={() => scan()}
            >
              <RefreshCw aria-hidden="true" /> Rescan all
            </button>
          )}
          <button
            className="pill"
            type="button"
            disabled={!native || busy != null}
            onClick={addRoot}
          >
            <FolderPlus aria-hidden="true" />{" "}
            {busy === "add" ? "Choosing…" : "Add music folder"}
          </button>
        </div>
      </div>
      <p className="section-lede">
        ECHO reads tags and artwork from these folders and keeps watching for
        changes. It never edits, moves or deletes your music.
      </p>
      {fault && (
        <p className="notice" role="alert">
          {fault}{" "}
          <button className="link-button" type="button" onClick={dismissFault}>
            Dismiss
          </button>
        </p>
      )}
      {!native ? (
        <p className="list-note">
          Folder management is available in the ECHO desktop app.
        </p>
      ) : roots.length === 0 ? (
        <p className="list-note">
          No folders yet. Choose the folder that holds your music to begin.
        </p>
      ) : (
        <ul className="root-list">
          {roots.map((root) => (
            <RootCard key={root.id} root={root} />
          ))}
        </ul>
      )}
    </section>
  );
}
