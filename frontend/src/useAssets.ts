import { useCallback, useEffect, useRef, useState } from 'react';
import { api, errorMessage } from './api';
import type { Asset, AssetPage } from './api';

const PAGE_SIZE = 100;
const MAX_CACHED_PAGES = 8;

/** Keep only visible pages and a small LRU cache, regardless of library size. */
export function useAssets(query: string) {
  const pages = useRef(new Map<number, Asset[]>());
  const inFlight = useRef(new Map<number, Promise<Asset[]>>());
  const controllers = useRef(new Set<AbortController>());
  const protectedPages = useRef(new Set<number>());
  const generation = useRef(0);
  const [version, setVersion] = useState(0);
  const [total, setTotal] = useState<number | null>(null);
  const totalRef = useRef<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const ensurePage = useCallback((page: number, refresh = false): Promise<Asset[]> => {
    if (!refresh && pages.current.has(page)) {
      const cached = pages.current.get(page)!;
      pages.current.delete(page);
      pages.current.set(page, cached);
      return Promise.resolve(cached);
    }
    const pending = inFlight.current.get(page);
    if (pending) return pending;
    const requestGeneration = generation.current;
    const controller = new AbortController();
    controllers.current.add(controller);
    const request = api<AssetPage>(
      `/api/assets?${query}&offset=${page * PAGE_SIZE}&limit=${PAGE_SIZE}`,
      { signal: controller.signal },
    ).then(result => {
      if (requestGeneration !== generation.current) return [];
      totalRef.current = result.total;
      setTotal(result.total);
      setError(null);
      pages.current.delete(page);
      pages.current.set(page, result.assets);
      while (pages.current.size > MAX_CACHED_PAGES) {
        const victim = [...pages.current.keys()].find(key => !protectedPages.current.has(key));
        if (victim === undefined) break;
        pages.current.delete(victim);
      }
      setVersion(value => value + 1);
      return result.assets;
    }).catch((cause: unknown) => {
      if (controller.signal.aborted || requestGeneration !== generation.current) return [];
      setError(errorMessage(cause));
      throw cause;
    }).finally(() => {
      controllers.current.delete(controller);
      if (requestGeneration === generation.current) inFlight.current.delete(page);
    });
    inFlight.current.set(page, request);
    return request;
  }, [query]);

  useEffect(() => {
    generation.current++;
    controllers.current.forEach(controller => controller.abort());
    controllers.current.clear();
    inFlight.current.clear();
    pages.current.clear();
    protectedPages.current.clear();
    totalRef.current = null;
    setTotal(null);
    setError(null);
    setVersion(value => value + 1);
    void ensurePage(0).catch(() => {});
    return () => {
      generation.current++;
      controllers.current.forEach(controller => controller.abort());
    };
  }, [ensurePage]);

  const ensureRange = useCallback((start: number, end: number) => {
    const lastPage = Math.max(0, Math.ceil((totalRef.current ?? 1) / PAGE_SIZE) - 1);
    const first = Math.floor(Math.max(0, start) / PAGE_SIZE);
    const last = Math.min(lastPage, Math.floor(Math.max(start, end) / PAGE_SIZE) + 1);
    const needed = new Set<number>();
    for (let page = first; page <= last; page++) needed.add(page);
    protectedPages.current = needed;
    for (const page of needed) void ensurePage(page).catch(() => {});
  }, [ensurePage]);

  const get = useCallback((index: number): Asset | undefined => {
    return pages.current.get(Math.floor(index / PAGE_SIZE))?.[index % PAGE_SIZE];
  }, []);

  const getAsync = useCallback(async (index: number): Promise<Asset | undefined> => {
    const result = await ensurePage(Math.floor(index / PAGE_SIZE));
    return result[index % PAGE_SIZE];
  }, [ensurePage]);

  const update = useCallback((asset: Asset) => {
    for (const [page, assets] of pages.current) {
      const index = assets.findIndex(item => item.id === asset.id);
      if (index !== -1) {
        const changed = [...assets];
        changed[index] = asset;
        pages.current.set(page, changed);
      }
    }
    setVersion(value => value + 1);
  }, []);

  const refresh = useCallback(() => {
    const active = new Set([0, ...protectedPages.current]);
    for (const page of active) void ensurePage(page, true).catch(() => {});
  }, [ensurePage]);

  return { total, error, version, get, getAsync, update, ensureRange, refresh };
}
