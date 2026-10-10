import i18n, { currentLocale } from './i18n';

export interface Scan {
  state: 'idle' | 'scanning' | 'error';
  processed: number;
  total: number | null;
  error: string | null;
}

export interface Library {
  id: number;
  name: string;
  path: string;
  asset_count: number;
  scan: Scan;
}

export interface Asset {
  id: number;
  library_id: number;
  name: string;
  relative_path: string;
  format: string;
  size: number;
  width: number | null;
  height: number | null;
  modified_at: number;
  favorite: boolean;
  tags: string[];
  thumbnail_url: string;
  original_url: string;
}

export interface Stats {
  total_assets: number;
  total_size: number;
  total_favorites: number;
  total_libraries: number;
}

export interface AssetPage {
  assets: Asset[];
  total: number;
  offset: number;
  limit: number;
}

export interface Tag {
  name: string;
  count: number;
}

export interface DirectoryListing {
  path: string | null;
  parent: string | null;
  roots: { name: string; path: string }[];
  directories: { name: string; path: string }[];
  truncated: boolean;
}

export interface LibraryFolder {
  path: string;
  name: string;
  parent: string | null;
  asset_count: number;
  direct_asset_count: number;
  has_children: boolean;
}

export interface LibraryFolders {
  separator: string;
  next_cursor: string | null;
  folders: LibraryFolder[];
  parent: string;
  truncated: boolean;
  direct_asset_count: number;
}

export interface BatchChanges {
  favorite?: boolean;
  add_tags?: string[];
  remove_tags?: string[];
}

export interface DesignScene {
  version: 1;
  name: string;
  canvas: { width: number; height: number; background: string };
  layers: Record<string, unknown>[];
}

export interface RenderJob {
  job_id: string;
  design_id: string;
  revision: number;
  quality: 'preview' | 'final';
  status: 'queued' | 'running' | 'succeeded' | 'failed';
  created_at: number;
  finished_at: number | null;
  error: string | null;
  width: number;
  height: number;
  output_url: string | null;
  preview_url: string | null;
  layout_url: string | null;
}

export interface StoredDesign {
  design_id: string;
  revision: number;
  created_at: number;
  name: string;
  scene: DesignScene;
  asset_sources: { asset_id: number; cache_key: string }[];
  latest_job: RenderJob | null;
}

export interface DesignSummary {
  design_id: string;
  revision: number;
  name: string;
  updated_at: number;
  width: number;
  height: number;
  layer_count: number;
  latest_job: RenderJob | null;
}

export interface DesignFont {
  id: string;
  name: string;
  supports_chinese: boolean;
}

export interface DesignPage {
  designs: DesignSummary[];
  limit: number;
  offset: number;
}

export const AUTH_REQUIRED_EVENT = 'picsoc:authentication-required';
let authGeneration = 0;

export class ApiError extends Error {
  readonly status: number;
  constructor(message: string, status: number) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

export async function api<T>(url: string, options: RequestInit = {}): Promise<T> {
  const requestAuthGeneration = authGeneration;
  const headers = new Headers(options.headers);
  if (!headers.has('Content-Type')) headers.set('Content-Type', 'application/json');
  headers.set('Accept-Language', i18n.language);
  const response = await fetch(url, {
    ...options,
    headers,
  });
  if (!response.ok) {
    let message = i18n.t('common.requestFailed', { status: response.status });
    try {
      const result = await response.json();
      if (typeof result.error === 'string') message = result.error;
    } catch {
      // A proxy may return a plain-text error; retain the HTTP status.
    }
    if (response.status === 401 && !url.startsWith('/api/auth/') && !options.signal?.aborted && requestAuthGeneration === authGeneration) window.dispatchEvent(new Event(AUTH_REQUIRED_EVENT));
    throw new ApiError(message, response.status);
  }
  if (url === '/api/auth/login' || url === '/api/auth/logout') authGeneration++;
  return response.json() as Promise<T>;
}

export function errorMessage(error: unknown): string {
  if (error instanceof TypeError) return i18n.t('common.serviceUnavailable');
  return error instanceof Error ? error.message : i18n.t('common.unknownError');
}

export function formatSize(size: number): string {
  if (size < 1024) return `${size.toLocaleString(currentLocale())} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = size / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toLocaleString(currentLocale(), {
    minimumFractionDigits: value < 10 ? 1 : 0,
    maximumFractionDigits: value < 10 ? 1 : 0,
  })} ${units[unit]}`;
}

export function formatDate(timestamp: number): string {
  return new Intl.DateTimeFormat(currentLocale(), {
    year: 'numeric', month: '2-digit', day: '2-digit',
  }).format(new Date(timestamp * 1000));
}
