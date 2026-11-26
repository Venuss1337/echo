import { keepPreviousData, useQueries, useQuery } from "@tanstack/react-query";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useCallback, useEffect, useMemo, type RefObject } from "react";
import { pagesCovering } from "./format";
import { useLibrary } from "./LibraryProvider";

export const PAGE = 100;

interface Page<T> {
  total: number;
  rows: T[];
}

/**
 * Virtualized rows backed by paginated queries. Only the pages under the
 * viewport are fetched and kept; the library is never held in JavaScript.
 * `key` must name every query parameter so each view has its own cache.
 */
export function usePagedList<T>({
  scope,
  key,
  fetchPage,
  scroller,
  rowHeight,
  group = 1,
  overscan = 8,
}: {
  scope: string;
  key: readonly unknown[];
  fetchPage: (offset: number, limit: number) => Promise<Page<T>>;
  scroller: RefObject<HTMLElement | null>;
  rowHeight: number;
  /** Items per virtual row, for card grids. */
  group?: number;
  overscan?: number;
}) {
  const { dataKey } = useLibrary();
  const base = useMemo(
    () => ["library", scope, dataKey, ...key],
    [dataKey, key, scope],
  );
  const first = useQuery({
    queryKey: [...base, 0],
    queryFn: () => fetchPage(0, PAGE),
    placeholderData: keepPreviousData,
    staleTime: Infinity,
    gcTime: 60_000,
  });
  const total = first.data?.total ?? 0;
  const virtualizer = useVirtualizer({
    count: Math.ceil(total / group),
    getScrollElement: () => scroller.current,
    estimateSize: () => rowHeight,
    overscan,
  });
  // Row sizes follow the container width for grids, so re-measure on change.
  useEffect(() => {
    virtualizer.measure();
  }, [rowHeight, virtualizer]);
  const items = virtualizer.getVirtualItems();
  const pages = pagesCovering(
    (items[0]?.index ?? 0) * group,
    ((items[items.length - 1]?.index ?? 0) + 1) * group - 1,
    PAGE,
    total,
  );
  const results = useQueries({
    queries: pages.map((page) => ({
      queryKey: [...base, page],
      queryFn: () => fetchPage(page * PAGE, PAGE),
      placeholderData: keepPreviousData,
      staleTime: Infinity,
      gcTime: 60_000,
    })),
  });
  const loaded = useMemo(() => {
    const map = new Map<number, T[]>();
    pages.forEach((page, index) => {
      const rows = results[index]?.data?.rows;
      if (rows) map.set(page, rows);
    });
    return map;
  }, [pages, results]);
  const rowAt = useCallback(
    (index: number): T | undefined =>
      loaded.get(Math.floor(index / PAGE))?.[index % PAGE],
    [loaded],
  );
  return {
    total,
    rowAt,
    virtualizer,
    items,
    loading: first.isPending,
    failed: first.isError,
    refetch: () => void first.refetch(),
  };
}
