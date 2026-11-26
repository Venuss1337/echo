import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowLeft,
  ArrowRight,
  Heart,
  Play,
  Plus,
  Radio as RadioIcon,
  Search,
} from "lucide-react";
import { useEffect, useState } from "react";
import type {
  RadioSort,
  Station,
  StationQuery,
  StationShelf,
} from "../generated/contracts";
import { Artwork, Sleeve } from "../library/Artwork";
import { usePlayer } from "../player";
import { useField, useLightTokens } from "../shell/Field";
import { IconButton, useSurface } from "../ui";
import { radioApi, radioFault } from "./api";

type Shelf = "discover" | StationShelf;
const shelves: [Shelf, string][] = [
  ["discover", "Discover"],
  ["favorites", "Favorites"],
  ["recent", "Recently played"],
  ["manual", "My stations"],
];
const empty: StationQuery = {
  name: "",
  country: "",
  language: "",
  tag: "",
  sort: "popular",
  offset: 0,
};

export function stationLight(station: Station) {
  return {
    id: null,
    seed: station.id,
    stationArt: station.artwork ? station.id : null,
  };
}

export function stationDetails(station: Station) {
  return [station.country, station.language, station.codec]
    .filter(Boolean)
    .join(" · ");
}

export function useStationAvailability() {
  return (
    useQuery({
      queryKey: ["radio-playback-available"],
      queryFn: radioApi.available,
      staleTime: Infinity,
      retry: false,
    }).data === true
  );
}

export function StationFavorite({
  station,
  refresh = false,
}: {
  station: Station;
  refresh?: boolean;
}) {
  const client = useQueryClient();
  const { notify } = useSurface();
  const current =
    useQuery({
      queryKey: ["radio-station", station.id],
      queryFn: () => radioApi.station(station.id),
      enabled: refresh,
      gcTime: 0,
      retry: false,
    }).data ?? station;
  const change = useMutation({
    mutationFn: () =>
      radioApi.mutate({
        kind: "favorite",
        id: station.id,
        enabled: !current.favorite,
      }),
    onSuccess: (saved) => {
      if (saved && refresh)
        client.setQueryData(["radio-station", station.id], saved);
      void client.invalidateQueries({ queryKey: ["radio"] });
      void client.invalidateQueries({
        queryKey: ["radio-station", station.id],
      });
    },
    onError: (error) => notify(radioFault(error)),
  });
  return (
    <IconButton
      label={
        current.favorite
          ? `Remove ${station.name} from favorites`
          : `Favorite ${station.name}`
      }
      pressed={current.favorite}
      disabled={change.isPending}
      onClick={() => change.mutate()}
    >
      <Heart
        aria-hidden="true"
        className={current.favorite ? "solid" : undefined}
      />
    </IconButton>
  );
}

export function RadioPage({
  onDetails,
}: {
  onDetails: (station: Station | "new") => void;
}) {
  const [shelf, setShelf] = useState<Shelf>("discover");
  const [draft, setDraft] = useState(empty);
  const [query, setQuery] = useState(empty);
  const [offset, setOffset] = useState(0);
  const [sortOverride, setSort] = useState<RadioSort>();
  const [visited, setVisited] = useState<number[]>([]);
  const client = useQueryClient();
  const { notify } = useSurface();
  const { playStation, view } = usePlayer();
  const enabled = useStationAvailability();
  const savedSort = useQuery({
    queryKey: ["radio-preferences"],
    queryFn: () => radioApi.preferences(null),
    staleTime: Infinity,
    retry: false,
  });
  const sort = sortOverride ?? savedSort.data ?? "popular";
  const settled = draft === query;
  const discover = shelf === "discover";
  useEffect(() => {
    const timer = setTimeout(() => setQuery(draft), 320);
    return () => clearTimeout(timer);
  }, [draft]);
  const page = useQuery({
    queryKey: [
      "radio",
      shelf,
      discover ? query : null,
      discover ? sort : null,
      offset,
      settled,
    ],
    queryFn: ({ signal }) =>
      discover
        ? radioApi.search({ ...query, sort, offset }, signal)
        : radioApi.shelf(shelf, offset),
    enabled: !discover || (settled && !savedSort.isPending),
    staleTime: 20_000,
    gcTime: 0,
    retry: false,
  });
  const stations = page.data?.stations ?? [];
  const filtered = Boolean(
    query.name || query.country || query.language || query.tag,
  );
  const featured =
    discover && !filtered && offset === 0 && sort === "popular"
      ? stations[0]
      : undefined;
  useField(featured ? stationLight(featured) : null);
  const featuredLight = useLightTokens(
    featured ? stationLight(featured) : { id: null, seed: "" },
  );
  const recentStation =
    view?.live?.status === "playing" ? view.live.station.id : null;
  useEffect(() => {
    if (recentStation)
      void client.invalidateQueries({ queryKey: ["radio", "recent"] });
  }, [client, recentStation]);

  function resetPage() {
    setOffset(0);
    setVisited([]);
  }

  function field(name: "name" | "country" | "language" | "tag", value: string) {
    resetPage();
    setDraft((previous) => ({ ...previous, [name]: value }));
  }

  return (
    <section className="page radio-page" aria-labelledby="radio-title">
      <header className="radio-header">
        <div>
          <h1 id="radio-title">Radio</h1>
          <p className="lede">
            Tune in to stations around the world and keep the ones you come back
            to.
          </p>
        </div>
        <button
          type="button"
          className="pill quiet"
          onClick={() => onDetails("new")}
        >
          <Plus aria-hidden="true" /> Add station
        </button>
      </header>
      <nav className="radio-shelves" aria-label="Radio sections">
        {shelves.map(([key, label]) => (
          <button
            type="button"
            className="quiet"
            key={key}
            aria-pressed={shelf === key}
            onClick={() => {
              setShelf(key);
              resetPage();
            }}
          >
            {label}
          </button>
        ))}
      </nav>
      {!enabled && (
        <p className="radio-note">
          Station playback is unavailable in this build.
        </p>
      )}
      {discover && (
        <div className="radio-filters" aria-label="Station filters">
          <label className="radio-search">
            <span>Search stations</span>
            <span className="radio-search-input">
              <Search aria-hidden="true" />
              <input
                type="search"
                maxLength={120}
                value={draft.name}
                onChange={(event) => field("name", event.target.value)}
              />
            </span>
          </label>
          <label>
            Country code
            <input
              placeholder="Any"
              list="radio-countries"
              maxLength={2}
              value={draft.country}
              onChange={(event) =>
                field(
                  "country",
                  event.target.value.replace(/[^a-z]/gi, "").toUpperCase(),
                )
              }
            />
          </label>
          <datalist id="radio-countries">
            {Object.entries({
              AU: "Australia",
              BR: "Brazil",
              CA: "Canada",
              DE: "Germany",
              FR: "France",
              GB: "United Kingdom",
              JP: "Japan",
              PL: "Poland",
              US: "United States",
            }).map(([code, country]) => (
              <option key={code} value={code}>
                {country}
              </option>
            ))}
          </datalist>
          <label>
            Language
            <input
              placeholder="Any"
              maxLength={64}
              value={draft.language}
              onChange={(event) => field("language", event.target.value)}
            />
          </label>
          <label>
            Genre or tag
            <input
              placeholder="Any"
              maxLength={64}
              value={draft.tag}
              onChange={(event) => field("tag", event.target.value)}
            />
          </label>
          <label>
            Sort by
            <select
              value={sort}
              onChange={(event) => {
                const next = event.target.value as RadioSort;
                setSort(next);
                resetPage();
                void radioApi.preferences(next).then(
                  (preference) =>
                    client.setQueryData(["radio-preferences"], preference),
                  (error) => notify(radioFault(error)),
                );
              }}
            >
              <option value="popular">Most popular</option>
              <option value="name">Name</option>
              <option value="recent">Recent activity</option>
            </select>
          </label>
        </div>
      )}
      {featured && (
        <article className="radio-feature" style={featuredLight.style}>
          <Sleeve
            id={null}
            seed={featured.id}
            title={featured.name}
            artist={featured.country}
            source={featuredLight.cover}
          />
          <div className="radio-feature-copy">
            <h2>{featured.name}</h2>
            <p>{stationDetails(featured) || "Internet radio"}</p>
            <div className="actions">
              <button
                type="button"
                className="pill"
                onClick={() => onDetails(featured)}
              >
                Station details
              </button>
              {enabled && (
                <IconButton
                  primary
                  label={`Play ${featured.name}`}
                  onClick={() => playStation(featured.id)}
                >
                  <Play aria-hidden="true" className="solid" />
                </IconButton>
              )}
              <StationFavorite station={featured} />
            </div>
          </div>
        </article>
      )}
      <section
        className="radio-list"
        aria-label={
          discover
            ? "Directory results"
            : shelves.find(([key]) => key === shelf)?.[1]
        }
      >
        <header>
          <h2>
            {discover
              ? filtered
                ? "Search results"
                : sort === "popular"
                  ? "Popular stations"
                  : "Stations"
              : shelves.find(([key]) => key === shelf)?.[1]}
          </h2>
          {discover && <span>Directory by Radio Browser</span>}
        </header>
        {page.isPending && (
          <p role="status" className="radio-note">
            Finding stations…
          </p>
        )}
        {page.isError && (
          <p role="alert" className="radio-note">
            {radioFault(page.error)}{" "}
            <button
              className="quiet"
              type="button"
              onClick={() => void page.refetch()}
            >
              Try again
            </button>
          </p>
        )}
        {page.data?.stale && (
          <p role="status" className="radio-note">
            Showing cached results while the directory is unavailable.
          </p>
        )}
        {page.data && stations.length === 0 && (
          <p className="radio-note">
            {discover
              ? "No stations match these filters."
              : "No stations here yet."}
          </p>
        )}
        <ul>
          {stations
            .filter((station) => station !== featured)
            .map((station) => (
              <li className="radio-row" key={station.id}>
                <button
                  className="radio-row-name"
                  type="button"
                  onClick={() => onDetails(station)}
                >
                  <Artwork id={null} seed={station.id} />
                  <span>
                    <strong>{station.name}</strong>
                    <small>{stationDetails(station)}</small>
                  </span>
                </button>
                <StationFavorite station={station} />
                {enabled && (
                  <IconButton
                    label={`Play ${station.name}`}
                    onClick={() => playStation(station.id)}
                  >
                    <Play aria-hidden="true" className="solid" />
                  </IconButton>
                )}
              </li>
            ))}
        </ul>
        {(offset > 0 || page.data?.next != null) && (
          <nav className="radio-pages" aria-label="Station pages">
            <IconButton
              label="Previous station page"
              disabled={offset === 0}
              onClick={() => {
                setOffset(visited[visited.length - 1] ?? 0);
                setVisited(visited.slice(0, -1));
              }}
            >
              <ArrowLeft aria-hidden="true" />
            </IconButton>
            <span>Page {visited.length + 1}</span>
            <IconButton
              label="Next station page"
              disabled={page.data?.next == null}
              onClick={() => {
                if (page.data?.next != null) {
                  setVisited([...visited, offset]);
                  setOffset(page.data.next);
                }
              }}
            >
              <ArrowRight aria-hidden="true" />
            </IconButton>
          </nav>
        )}
      </section>
      {!discover &&
        stations.length === 0 &&
        !page.isPending &&
        !page.isError && (
          <button
            className="quiet"
            type="button"
            onClick={() =>
              shelf === "manual" ? onDetails("new") : setShelf("discover")
            }
          >
            <RadioIcon aria-hidden="true" />{" "}
            {shelf === "manual" ? "Add a stream URL" : "Discover stations"}
          </button>
        )}
    </section>
  );
}
