import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Pencil, Play, Trash2 } from "lucide-react";
import { useState } from "react";
import type { Station, StationEdit } from "../generated/contracts";
import { Sleeve } from "../library/Artwork";
import { usePlayer } from "../player";
import { useLightTokens } from "../shell/Field";
import { Inspector } from "../shell/Inspector";
import { IconButton, useSurface } from "../ui";
import { radioApi, radioFault } from "./api";
import { stationLight, StationFavorite, useStationAvailability } from "./Radio";

const initial: StationEdit = {
  id: null,
  name: "",
  url: "",
  country: "",
  language: "",
  tags: "",
};

export function StationInspector({
  selection,
  onClose,
  onSaved,
}: {
  selection: Station | "new";
  onClose: () => void;
  onSaved: (station: Station) => void;
}) {
  const original = selection === "new" ? null : selection;
  const [editing, setEditing] = useState(!original);
  const [confirm, setConfirm] = useState(false);
  const { playStation } = usePlayer();
  const canPlay = useStationAvailability();
  const client = useQueryClient();
  const { notify } = useSurface();
  const station =
    useQuery({
      queryKey: ["radio-station", original?.id],
      queryFn: () => radioApi.station(original!.id),
      enabled: original != null,
      gcTime: 0,
      retry: false,
    }).data ?? original;
  const edit = useQuery({
    queryKey: ["radio-edit", original?.id],
    queryFn: () => radioApi.edit(original!.id),
    enabled: editing && original != null,
    gcTime: 0,
    retry: false,
  });
  const remove = useMutation({
    mutationFn: () => radioApi.mutate({ kind: "remove", id: original!.id }),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: ["radio"] });
      onClose();
      notify("Station removed");
    },
    onError: (error) => notify(radioFault(error)),
  });
  const light = useLightTokens(
    station ? stationLight(station) : { id: null, seed: "" },
  );
  return (
    <Inspector
      title={editing ? (original ? "Edit station" : "Add station") : "Station"}
      onClose={onClose}
      style={light.style}
    >
      <div className="radio-inspector">
        {editing ? (
          original && !edit.data ? (
            edit.isError ? (
              <p role="alert">{radioFault(edit.error)}</p>
            ) : (
              <p role="status">Opening saved station…</p>
            )
          ) : (
            <StationForm
              initialValues={edit.data ?? initial}
              onCancel={() => (original ? setEditing(false) : onClose())}
              onSaved={(saved) => {
                client.setQueryData(["radio-station", saved.id], saved);
                void client.invalidateQueries({ queryKey: ["radio"] });
                setEditing(false);
                onSaved(saved);
              }}
            />
          )
        ) : (
          station && (
            <>
              <Sleeve
                id={null}
                seed={station.id}
                title={station.name}
                artist={station.country}
                source={light.cover}
              />
              <h3>{station.name}</h3>
              <dl>
                {[
                  ["Country", station.country],
                  ["Language", station.language],
                  ["Genres", station.tags],
                  [
                    "Reported format",
                    [
                      station.codec,
                      station.bitrate && `${station.bitrate} kbps`,
                    ]
                      .filter(Boolean)
                      .join(" · "),
                  ],
                  [
                    "Directory status",
                    station.reportedOnline == null
                      ? ""
                      : station.reportedOnline
                        ? "Listed as online"
                        : "Listed as offline",
                  ],
                ]
                  .filter(([, value]) => value)
                  .map(([label, value]) => (
                    <div key={label}>
                      <dt>{label}</dt>
                      <dd>{value}</dd>
                    </div>
                  ))}
              </dl>
              <div className="actions">
                {canPlay && (
                  <button
                    className="pill"
                    type="button"
                    onClick={() => playStation(station.id)}
                  >
                    <Play aria-hidden="true" /> Play station
                  </button>
                )}
                <StationFavorite station={station} refresh />
                {station.manual && (
                  <>
                    <IconButton
                      label="Edit station"
                      onClick={() => {
                        setEditing(true);
                        setConfirm(false);
                      }}
                    >
                      <Pencil aria-hidden="true" />
                    </IconButton>
                    <IconButton
                      label="Remove station"
                      onClick={() => setConfirm(!confirm)}
                      expanded={confirm}
                    >
                      <Trash2 aria-hidden="true" />
                    </IconButton>
                  </>
                )}
              </div>
              {confirm && (
                <div className="radio-remove">
                  <p>Remove this station from your collection?</p>
                  <button
                    className="pill"
                    type="button"
                    disabled={remove.isPending}
                    onClick={() => remove.mutate()}
                  >
                    Remove station
                  </button>
                  <button
                    className="quiet"
                    type="button"
                    onClick={() => setConfirm(false)}
                  >
                    Cancel
                  </button>
                </div>
              )}
            </>
          )
        )}
      </div>
    </Inspector>
  );
}

function StationForm({
  initialValues,
  onCancel,
  onSaved,
}: {
  initialValues: StationEdit;
  onCancel: () => void;
  onSaved: (station: Station) => void;
}) {
  const [draft, setDraft] = useState(initialValues);
  const save = useMutation({
    mutationFn: () => radioApi.mutate({ kind: "save", station: draft }),
    onSuccess: (station) => {
      if (station) onSaved(station);
    },
  });
  return (
    <form
      className="radio-form"
      onSubmit={(event) => {
        event.preventDefault();
        save.mutate();
      }}
    >
      <label>
        Name
        <input
          required
          maxLength={160}
          value={draft.name}
          onChange={(event) => setDraft({ ...draft, name: event.target.value })}
        />
      </label>
      <label>
        Stream URL
        <input
          type="url"
          required
          maxLength={4096}
          value={draft.url}
          placeholder="https://…"
          spellCheck={false}
          aria-describedby="stream-help"
          onChange={(event) => setDraft({ ...draft, url: event.target.value })}
        />
      </label>
      <p id="stream-help">Use a direct public HTTP or HTTPS stream.</p>
      <label>
        Country
        <input
          maxLength={80}
          value={draft.country}
          onChange={(event) =>
            setDraft({ ...draft, country: event.target.value })
          }
        />
      </label>
      <label>
        Language
        <input
          maxLength={80}
          value={draft.language}
          onChange={(event) =>
            setDraft({ ...draft, language: event.target.value })
          }
        />
      </label>
      <label>
        Genres or tags
        <input
          maxLength={160}
          value={draft.tags}
          onChange={(event) => setDraft({ ...draft, tags: event.target.value })}
        />
      </label>
      {save.isError && <p role="alert">{radioFault(save.error)}</p>}
      <div className="actions">
        <button className="pill" type="submit" disabled={save.isPending}>
          Save station
        </button>
        <button className="quiet" type="button" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
