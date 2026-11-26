import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { Heart, Plus } from "lucide-react";
import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import type { FavoriteTarget, PlaylistAct } from "../generated/contracts";
import { Dialog, IconButton, useSurface } from "../ui";
import { describeLibraryFault, libraryApi } from "./api";
import { plural } from "./format";
import { useLibrary } from "./LibraryProvider";
import { Mosaic } from "./Artwork";

/** What a listener is about to put into a playlist. */
export type Addition =
  | { kind: "tracks"; ids: number[]; label: string }
  | { kind: "album"; id: number; label: string };

interface UserBinding {
  favorites: ReadonlyMap<string, boolean>;
  toggleFavorite: (target: FavoriteTarget, on: boolean, title: string) => void;
  /** Resolves to the affected playlist id, or null after reporting a fault. */
  playlistAct: (action: PlaylistAct) => Promise<number | null>;
  addToPlaylist: (addition: Addition) => void;
  createPlaylist: () => void;
}

const UserContext = createContext<UserBinding | null>(null);
const favoriteKey = (target: FavoriteTarget) => `${target.kind}:${target.id}`;

/**
 * Optimism-free user actions. Favorites keep a small overlay of confirmed
 * changes so every view agrees without refetching library pages.
 */
export function UserProvider({ children }: { children: ReactNode }) {
  const { notify } = useSurface();
  const [favorites, setFavorites] = useState<ReadonlyMap<string, boolean>>(
    new Map(),
  );
  const [adding, setAdding] = useState<Addition | null>(null);
  const [creating, setCreating] = useState(false);
  const toggleFavorite = useCallback(
    (target: FavoriteTarget, on: boolean, title: string) => {
      libraryApi.favorite(target, on).then(
        () => {
          setFavorites((previous) =>
            new Map(previous).set(favoriteKey(target), on),
          );
          notify(
            on
              ? `Added ${title} to favorites`
              : `Removed ${title} from favorites`,
          );
        },
        (error) => notify(describeLibraryFault(error)),
      );
    },
    [notify],
  );
  const playlistAct = useCallback(
    (action: PlaylistAct) =>
      libraryApi.playlistAct(action).then(
        ({ id }) => id,
        (error) => {
          notify(describeLibraryFault(error));
          return null;
        },
      ),
    [notify],
  );
  const value = useMemo<UserBinding>(
    () => ({
      favorites,
      toggleFavorite,
      playlistAct,
      addToPlaylist: setAdding,
      createPlaylist: () => setCreating(true),
    }),
    [favorites, playlistAct, toggleFavorite],
  );
  return (
    <UserContext value={value}>
      {children}
      {adding && (
        <AddDialog addition={adding} onClose={() => setAdding(null)} />
      )}
      {creating && (
        <PlaylistForm
          title="New playlist"
          submit="Create playlist"
          onClose={() => setCreating(false)}
          onSubmit={async (name, description) =>
            (await playlistAct({
              kind: "create",
              name,
              description,
              trackIds: [],
            })) != null
          }
        />
      )}
    </UserContext>
  );
}

export function useUser() {
  const user = useContext(UserContext);
  if (!user) throw new Error("UserProvider is missing");
  return user;
}

/** Playlists, shared by the rail, dialogs and collection page. */
export function usePlaylists() {
  const { summary } = useLibrary();
  return useQuery({
    queryKey: ["user", "playlists", summary?.userRevision],
    queryFn: libraryApi.playlists,
    enabled: summary != null,
    placeholderData: keepPreviousData,
    staleTime: Infinity,
    gcTime: 60_000,
  });
}

export function useFavorite(target: FavoriteTarget | null, initial: boolean) {
  const { favorites } = useUser();
  return target ? (favorites.get(favoriteKey(target)) ?? initial) : initial;
}

export function HeartButton({
  target,
  initial,
  title,
  className = "",
}: {
  target: FavoriteTarget;
  initial: boolean;
  title: string;
  className?: string;
}) {
  const { toggleFavorite } = useUser();
  const on = useFavorite(target, initial);
  return (
    <IconButton
      label={on ? `Remove ${title} from favorites` : `Favorite ${title}`}
      pressed={on}
      className={`heart ${className}`}
      onClick={() => toggleFavorite(target, !on, title)}
    >
      <Heart aria-hidden="true" />
    </IconButton>
  );
}

export function PlaylistForm({
  title,
  submit,
  initial = { name: "", description: "" },
  onSubmit,
  onClose,
}: {
  title: string;
  submit: string;
  initial?: { name: string; description: string };
  onSubmit: (name: string, description: string) => Promise<boolean>;
  onClose: () => void;
}) {
  const [name, setName] = useState(initial.name);
  const [description, setDescription] = useState(initial.description);
  const [busy, setBusy] = useState(false);
  return (
    <Dialog title={title} onClose={onClose}>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          setBusy(true);
          void onSubmit(name, description).then((done) => {
            setBusy(false);
            if (done) onClose();
          });
        }}
      >
        <label className="form-field">
          <span>Name</span>
          <input
            autoFocus
            required
            maxLength={120}
            value={name}
            onChange={(event) => setName(event.currentTarget.value)}
          />
        </label>
        <label className="form-field">
          <span>Description</span>
          <textarea
            rows={3}
            maxLength={500}
            value={description}
            onChange={(event) => setDescription(event.currentTarget.value)}
          />
        </label>
        <div className="dialog-actions">
          <button type="button" className="quiet" onClick={onClose}>
            Cancel
          </button>
          <button
            type="submit"
            className="pill"
            disabled={busy || !name.trim()}
          >
            {submit}
          </button>
        </div>
      </form>
    </Dialog>
  );
}

function AddDialog({
  addition,
  onClose,
}: {
  addition: Addition;
  onClose: () => void;
}) {
  const { playlistAct } = useUser();
  const { notify } = useSurface();
  const playlists = usePlaylists();
  const [name, setName] = useState("");
  async function add(id: number, label: string) {
    const done = await playlistAct(
      addition.kind === "album"
        ? { kind: "addAlbum", id, albumId: addition.id }
        : { kind: "addTracks", id, trackIds: addition.ids },
    );
    if (done != null) {
      notify(`Added ${addition.label} to ${label}`);
      onClose();
    }
  }
  return (
    <Dialog title="Add to playlist" onClose={onClose}>
      <p className="dialog-lede">{addition.label}</p>
      <form
        className="new-playlist"
        onSubmit={(event) => {
          event.preventDefault();
          const label = name.trim();
          void playlistAct({
            kind: "create",
            name: label,
            description: "",
            trackIds: [],
          }).then((id) => {
            if (id != null) void add(id, label);
          });
        }}
      >
        <input
          autoFocus
          aria-label="New playlist name"
          placeholder="New playlist"
          maxLength={120}
          value={name}
          onChange={(event) => setName(event.currentTarget.value)}
        />
        <button type="submit" className="pill" disabled={!name.trim()}>
          <Plus aria-hidden="true" /> Create
        </button>
      </form>
      <ul className="playlist-picker">
        {playlists.data?.map((playlist) => (
          <li key={playlist.id}>
            <button
              type="button"
              onClick={() => void add(playlist.id, playlist.name)}
            >
              <Mosaic ids={playlist.artworkIds} seed={playlist.name} />
              <span>
                <strong>{playlist.name}</strong>
                <small>{plural(playlist.tracks, "track")}</small>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </Dialog>
  );
}
