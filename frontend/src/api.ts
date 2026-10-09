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
}

export interface LibraryFolders {
  folders: LibraryFolder[];
  parent: string;
  truncated: boolean;
}

export interface BatchChanges {
  favorite?: boolean;
  add_tags?: string[];
  remove_tags?: string[];
}

export async function api<T>(url: string, options: RequestInit = {}): Promise<T> {
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
    throw new Error(message);
  }
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
