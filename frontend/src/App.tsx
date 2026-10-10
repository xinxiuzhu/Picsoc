import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useTranslation } from 'react-i18next';
import {
  AlertCircle, ArrowUp, Check, ChevronDown, ChevronRight, Folder, Grid2X2, Grid3X3,
  HardDrive, Images, LayoutGrid, ListChecks, LoaderCircle, LogOut, Menu, PanelLeftClose, PanelLeftOpen, Plus,
  RefreshCw, Search, SlidersHorizontal, Star, Tag, Trash2, X,
} from 'lucide-react';
import { api, errorMessage, formatSize } from './api';
import type { Asset, BatchChanges, Library, Stats, Tag as AssetTag } from './api';
import { AddLibraryDialog, AssetCard, AssetPreview, BatchTagsDialog, Dialog, EmptyState } from './components';
import { useAssets } from './useAssets';
import { LibraryTree, folderBreadcrumbs } from './LibraryTree';
import { FilterPanel, EMPTY_IMAGE_FILTERS, appendImageFilters, parseExcludedNames, excludedNamesError, SORT_OPTIONS, SORT_KEYS } from './FilterPanel';
import type { ImageFilters } from './FilterPanel';
import { LanguageMenu } from './LanguageMenu';
import { DisplaySettings, getSavedDisplay, saveDisplay } from './DisplaySettings';
import type { DisplayPreferences } from './DisplaySettings';

type Category = 'all' | 'favorites';
const EMPTY_STATS: Stats = { total_assets: 0, total_size: 0, total_favorites: 0, total_libraries: 0 };
const MAX_SELECTION = 500;
const EXCLUDED_NAMES_KEY = 'picsoc-excluded-names';

function getSavedExcludedNames(): string {
  try {
    const value = localStorage.getItem(EXCLUDED_NAMES_KEY) ?? '';
    return excludedNamesError(value) ? '' : parseExcludedNames(value).join('\n');
  } catch { return ''; }
}

function getSavedSidebarCollapsed(): boolean {
  try { return localStorage.getItem('picsoc-sidebar-collapsed') === 'true'; }
  catch { return false; }
}

export default function App({ onLogout }: { onLogout?: () => Promise<void> }) {
  const { t, i18n } = useTranslation();
  const number = (value: number) => value.toLocaleString(i18n.resolvedLanguage?.startsWith('en') ? 'en-US' : 'zh-CN');
  const [libraries, setLibraries] = useState<Library[]>([]);
  const [stats, setStats] = useState<Stats>(EMPTY_STATS);
  const [tags, setTags] = useState<AssetTag[]>([]);
  const [metadataLoaded, setMetadataLoaded] = useState(false);
  const [serviceError, setServiceError] = useState<string | null>(null);
  const [category, setCategory] = useState<Category>('all');
  const [libraryId, setLibraryId] = useState<number | null>(null);
  const [search, setSearch] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  const [format, setFormat] = useState('');
  const [sort, setSort] = useState('modified');
  const [selectedTag, setSelectedTag] = useState('');
  const [favoriteOnly, setFavoriteOnly] = useState(false);
  const [excludedNames, setExcludedNames] = useState(getSavedExcludedNames);
  const excludedKeywords = useMemo(() => parseExcludedNames(excludedNames), [excludedNames]);
  const [imageFilters, setImageFilters] = useState<ImageFilters>({ ...EMPTY_IMAGE_FILTERS });
  const [showFilters, setShowFilters] = useState(false);
  const [logoutBusy, setLogoutBusy] = useState(false);
  const [display, setDisplay] = useState(getSavedDisplay);
  const [showAdd, setShowAdd] = useState(false);
  const [managedLibrary, setManagedLibrary] = useState<Library | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [libraryBusy, setLibraryBusy] = useState(false);
  const [libraryError, setLibraryError] = useState<string | null>(null);
  const [cancelingLibraryId, setCancelingLibraryId] = useState<number | null>(null);
  const [selected, setSelected] = useState<{ asset: Asset; index: number } | null>(null);
  const [toast, setToast] = useState<{ text: string; error?: boolean } | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(getSavedSidebarCollapsed);
  const [mobileViewport, setMobileViewport] = useState(() => window.matchMedia('(max-width: 700px)').matches);
  const [showAllTags, setShowAllTags] = useState(false);
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<number>>(() => new Set());
  const [batchBusy, setBatchBusy] = useState(false);
  const [showBatchTags, setShowBatchTags] = useState(false);
  const [folder, setFolder] = useState('');
  const [folderRecursive, setFolderRecursive] = useState(true);
  const [folderSeparator, setFolderSeparator] = useState('/');
  const [folderDirectCount, setFolderDirectCount] = useState<number | null>(null);
  const [foldersRevision, setFoldersRevision] = useState(0);
  const folderTrail = useMemo(() => folderBreadcrumbs(folder, folderSeparator), [folder, folderSeparator]);
  const scrollElement = useRef<HTMLDivElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);
  const sidebarVisible = mobileViewport ? sidebarOpen : !sidebarCollapsed;
  const sidebarLabel = t(mobileViewport ? sidebarOpen ? 'app.closeNavigation' : 'app.openNavigation' : sidebarCollapsed ? 'app.expandSidebar' : 'app.collapseSidebar');
  const toggleSidebar = useCallback(() => {
    if (mobileViewport) setSidebarOpen(previous => !previous);
    else setSidebarCollapsed(previous => !previous);
  }, [mobileViewport]);
  useEffect(() => {
    const viewport = window.matchMedia('(max-width: 700px)');
    const changed = () => { setMobileViewport(viewport.matches); setSidebarOpen(false); };
    viewport.addEventListener('change', changed);
    return () => viewport.removeEventListener('change', changed);
  }, []);
  useEffect(() => {
    try { localStorage.setItem('picsoc-sidebar-collapsed', String(sidebarCollapsed)); }
    catch { /* Navigation also works when browser storage is unavailable. */ }
  }, [sidebarCollapsed]);
  useEffect(() => { saveDisplay(display); }, [display]);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.isComposing || document.querySelector('[role="dialog"]')) return;
      const target = event.target;
      const editing = target instanceof HTMLElement && (target.matches('input, textarea, select') || target.isContentEditable);
      if ((event.metaKey || event.ctrlKey) && event.key === '\\' && !event.altKey) {
        event.preventDefault(); toggleSidebar();
      }
      if (event.key === 'Escape' && mobileViewport && sidebarOpen) setSidebarOpen(false);
      if (((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') || (!editing && !event.metaKey && !event.ctrlKey && !event.altKey && event.key === '/')) {
        event.preventDefault();
        searchInput.current?.focus();
        searchInput.current?.select();
      }
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [toggleSidebar, mobileViewport, sidebarOpen]);
  const [gridWidth, setGridWidth] = useState(900);
  const favoritePending = useRef(new Set<number>());
  const navigatePending = useRef(false);
  const toastTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const scanning = libraries.some(library => library.scan.state === 'scanning');
  const activeLibrary = libraries.find(library => library.id === libraryId);
  const title = activeLibrary?.name ?? (category === 'favorites' ? t('app.favorites') : t('app.allAssets'));

  const query = useMemo(() => {
    const parameters = new URLSearchParams({ sort });
    if (debouncedSearch.trim()) parameters.set('q', debouncedSearch.trim());
    if (libraryId !== null) parameters.set('library_id', String(libraryId));
    if (libraryId !== null) {
      parameters.set('folder', folder);
      parameters.set('folder_recursive', String(folderRecursive));
    }
    if (category === 'favorites' || favoriteOnly) parameters.set('favorite', 'true');
    if (format) parameters.set('format', format);
    if (selectedTag) parameters.set('tag', selectedTag);
    if (excludedNames) parameters.set('exclude_names', excludedNames);
    appendImageFilters(parameters, imageFilters);
    return parameters.toString();
  }, [debouncedSearch, libraryId, category, format, sort, selectedTag, folder, folderRecursive, imageFilters, favoriteOnly, excludedNames]);
  const assets = useAssets(query);
  const refreshAssets = useRef(assets.refresh);
  refreshAssets.current = assets.refresh;
  useEffect(() => {
    try {
      if (excludedNames) localStorage.setItem(EXCLUDED_NAMES_KEY, excludedNames);
      else localStorage.removeItem(EXCLUDED_NAMES_KEY);
    } catch { /* Filtering also works when browser storage is unavailable. */ }
  }, [excludedNames]);

  const notify = useCallback((text: string, error = false) => {
    setToast({ text, error });
    if (toastTimer.current) clearTimeout(toastTimer.current);
    toastTimer.current = setTimeout(() => setToast(null), error ? 6000 : 3000);
  }, []);

  const loadMetadata = useCallback(async (signal?: AbortSignal) => {
    const results = await Promise.allSettled([
      api<{ libraries: Library[] }>('/api/libraries', { signal }),
      api<Stats>('/api/stats', { signal }),
      api<{ tags: AssetTag[] }>('/api/tags', { signal }),
    ]);
    if (signal?.aborted) return;
    const [libraryResult, statsResult, tagResult] = results;
    if (libraryResult.status === 'fulfilled') {
      setLibraries(libraryResult.value.libraries);
      setMetadataLoaded(true);
      setServiceError(null);
      setManagedLibrary(previous => previous ? libraryResult.value.libraries.find(item => item.id === previous.id) ?? null : null);
    } else { setServiceError(errorMessage(libraryResult.reason)); }
    if (statsResult.status === 'fulfilled') setStats(statsResult.value);
    if (tagResult.status === 'fulfilled') setTags(tagResult.value.tags);
  }, []);

  const loadFolders = useCallback(async () => { setFoldersRevision(previous => previous + 1); }, []);
  const refreshFolders = useRef(loadFolders);
  refreshFolders.current = loadFolders;

  useEffect(() => {
    const controller = new AbortController();
    void loadMetadata(controller.signal);
    refreshAssets.current();
    return () => controller.abort();
  }, [loadMetadata, i18n.resolvedLanguage]);

  useEffect(() => {
    const controller = new AbortController();
    const interval = setInterval(() => {
      if (document.hidden) return;
      void loadMetadata(controller.signal);
      refreshAssets.current();
      void refreshFolders.current();
    }, scanning ? 4000 : 15000);
    return () => { clearInterval(interval); controller.abort(); };
  }, [loadMetadata, scanning]);

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedSearch(search), 250);
    return () => clearTimeout(timer);
  }, [search]);

  useEffect(() => {
    const element = scrollElement.current;
    if (!element) return;
    const observer = new ResizeObserver(entries => setGridWidth(entries[0].contentRect.width));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    scrollElement.current?.scrollTo({ top: 0 });
    setSelected(null);
    setSelectedIds(new Set());
    setShowBatchTags(false);
  }, [query]);

  useEffect(() => {
    if (metadataLoaded && libraryId !== null && !libraries.some(item => item.id === libraryId)) {
      setLibraryId(null); setFolder(''); setFolderDirectCount(null);
    }
    if (cancelingLibraryId !== null && !libraries.some(item => item.id === cancelingLibraryId && item.scan.state === 'scanning')) {
      setCancelingLibraryId(null);
    }
  }, [metadataLoaded, libraries, libraryId, cancelingLibraryId]);

  useEffect(() => () => {
    if (toastTimer.current) clearTimeout(toastTimer.current);
  }, []);

  const density = display.size;
  const automaticColumns = Math.max(1, Math.floor((gridWidth + 16) / (density + 16)));
  const maximumColumns = Math.max(1, Math.floor((gridWidth + 16) / (120 + 16)));
  const columns = display.columns === null ? automaticColumns : Math.min(display.columns, maximumColumns);
  const cardWidth = (gridWidth - Math.max(0, columns - 1) * 16) / columns;
  const imageHeight = Math.round(Math.max(0, cardWidth - 2) * 0.75);
  const rowHeight = imageHeight + 80;
  const previousLayout = useRef({ columns, rowHeight });
  const scrollAnchor = useRef({ assetIndex: 0, rowFraction: 0 });
  const rememberScroll = () => {
    const top = scrollElement.current?.scrollTop ?? 0;
    const layout = previousLayout.current;
    scrollAnchor.current = { assetIndex: Math.floor(top / layout.rowHeight) * layout.columns, rowFraction: (top % layout.rowHeight) / layout.rowHeight };
  };
  const virtualizer = useVirtualizer({
    count: Math.ceil((assets.total ?? 0) / columns),
    getScrollElement: () => scrollElement.current,
    estimateSize: () => rowHeight,
    overscan: 3,
  });
  const rows = virtualizer.getVirtualItems();
  const firstVisible = rows[0]?.index ?? 0;
  const lastVisible = rows[rows.length - 1]?.index ?? 0;

  useLayoutEffect(() => {
    const previous = previousLayout.current;
    if (previous.columns === columns && previous.rowHeight === rowHeight) return;
    previousLayout.current = { columns, rowHeight };
    virtualizer.measure();
    const anchor = scrollAnchor.current;
    // Uniform rows let the new grid include the same first visible asset after resizing.
    virtualizer.scrollToOffset(Math.floor(anchor.assetIndex / columns) * rowHeight + anchor.rowFraction * rowHeight);
    rememberScroll();
  }, [columns, rowHeight, virtualizer]);
  useEffect(() => {
    if (assets.total !== null && assets.total > 0) assets.ensureRange(firstVisible * columns, (lastVisible + 1) * columns - 1);
  }, [firstVisible, lastVisible, columns, assets.total, assets.ensureRange, assets.version]);

  const navigate = async (direction: number) => {
    if (!selected || navigatePending.current) return;
    const index = selected.index + direction;
    if (index < 0 || index >= (assets.total ?? 0)) return;
    navigatePending.current = true;
    try {
      const asset = await assets.getAsync(index);
      if (asset) setSelected({ asset, index });
    } catch (cause) { notify(errorMessage(cause), true); }
    finally { navigatePending.current = false; }
  };

  const assetUpdated = (asset: Asset) => {
    assets.update(asset);
    const noLongerMatches = ((category === 'favorites' || favoriteOnly) && !asset.favorite) || (Boolean(selectedTag) && !asset.tags.includes(selectedTag));
    setSelected(previous => previous?.asset.id === asset.id ? noLongerMatches ? null : { ...previous, asset } : previous);
    void loadMetadata();
    if (category === 'favorites' || favoriteOnly || selectedTag) assets.refresh();
  };

  const favorite = async (asset: Asset) => {
    if (favoritePending.current.has(asset.id)) return;
    favoritePending.current.add(asset.id);
    try {
      const updated = await api<Asset>(`/api/assets/${asset.id}`, { method: 'PATCH', body: JSON.stringify({ favorite: !asset.favorite }) });
      assetUpdated(updated);
      notify(t(updated.favorite ? 'app.favoriteAdded' : 'app.favoriteRemoved'));
    } catch (cause) { notify(errorMessage(cause), true); }
    finally { favoritePending.current.delete(asset.id); }
  };

  const setScope = (nextCategory: Category, nextLibrary: number | null) => {
    setCategory(nextCategory); setLibraryId(nextLibrary); setSidebarOpen(false);
    setFolder(''); setFolderDirectCount(null);
  };
  const setGridDensity = (value: number) => {
    setDisplay(previous => ({ ...previous, size: value, columns: null }));
  };
  const updateDisplay = (preferences: DisplayPreferences) => setDisplay(preferences);
  const clearFilters = () => {
    setSearch(''); setDebouncedSearch(''); setFormat(''); setSelectedTag('');
    setFavoriteOnly(false);
    setExcludedNames('');
    setImageFilters({ ...EMPTY_IMAGE_FILTERS });
  };
  const resetFilters = () => {
    clearFilters(); setSort('modified');
    setCategory('all'); setLibraryId(null);
    setFolder(''); setFolderDirectCount(null);
  };
  const manage = (library: Library) => {
    setManagedLibrary(library); setConfirmRemove(false); setLibraryError(null);
  };
  const scanLibrary = async (library: Library) => {
    setLibraryBusy(true); setLibraryError(null);
    try {
      await api(`/api/libraries/${library.id}/scan`, { method: 'POST' });
      await loadMetadata();
      assets.refresh();
      void loadFolders();
      notify(t('app.scanStarted'));
    } catch (cause) { setLibraryError(errorMessage(cause)); }
    finally { setLibraryBusy(false); }
  };
  const removeLibrary = async (library: Library) => {
    setLibraryBusy(true); setLibraryError(null);
    try {
      await api(`/api/libraries/${library.id}`, { method: 'DELETE' });
      if (libraryId === library.id) { setLibraryId(null); setFolder(''); setFolderDirectCount(null); }
      setManagedLibrary(null); setConfirmRemove(false);
      await loadMetadata(); assets.refresh();
      notify(t('app.libraryRemoved'));
    } catch (cause) { setLibraryError(errorMessage(cause)); }
    finally { setLibraryBusy(false); }
  };
  const cancelScan = async (library: Library) => {
    if (cancelingLibraryId !== null) return;
    setCancelingLibraryId(library.id);
    try {
      await api<{ ok: boolean }>(`/api/libraries/${library.id}/scan/cancel`, { method: 'POST' });
      await loadMetadata(); assets.refresh(); void loadFolders();
      notify(t('app.scanCancelRequested'));
    } catch (cause) {
      setCancelingLibraryId(null);
      notify(errorMessage(cause), true);
    }
  };

  const filterCount = Number(Boolean(search.trim())) + Number(Boolean(format)) + Number(Boolean(selectedTag)) + Number(favoriteOnly) + excludedKeywords.length
    + Number(Boolean(imageFilters.orientation)) + Number(Boolean(imageFilters.aspect_ratio))
    + Number(Boolean(imageFilters.min_width || imageFilters.max_width))
    + Number(Boolean(imageFilters.min_height || imageFilters.max_height))
    + Number(Boolean(imageFilters.min_size_mb || imageFilters.max_size_mb));
  const hasFilters = filterCount > 0 || category === 'favorites';
  const progressLibrary = activeLibrary?.scan.state === 'scanning' ? activeLibrary : libraries.find(item => item.scan.state === 'scanning');
  const scanErrors = libraries.filter(item => item.scan.state === 'error');

  const toggleSelection = (id: number) => {
    if (batchBusy) return;
    setSelectedIds(previous => {
      const next = new Set(previous);
      if (next.has(id)) next.delete(id);
      else if (next.size < MAX_SELECTION) next.add(id);
      return next;
    });
  };
  const selectVisible = () => {
    if (batchBusy) return;
    const top = scrollElement.current?.scrollTop ?? 0;
    const bottom = top + (scrollElement.current?.clientHeight ?? 0);
    const visibleIds: number[] = [];
    for (const row of rows) {
      if (row.end - 16 <= top || row.start >= bottom) continue;
      for (let column = 0; column < columns; column++) {
        const asset = assets.get(row.index * columns + column);
        if (asset) visibleIds.push(asset.id);
      }
    }
    if (!visibleIds.length) { notify(t('app.batch.noVisibleAssets')); return; }
    const next = new Set(selectedIds);
    for (const id of visibleIds) {
      if (next.size >= MAX_SELECTION) break;
      next.add(id);
    }
    if (visibleIds.some(id => !next.has(id))) notify(t('app.batch.limit', { count: MAX_SELECTION }));
    setSelectedIds(next);
  };
  const applyBatch = async (changes: BatchChanges) => {
    if (batchBusy || !selectedIds.size) throw new Error(t('app.batch.chooseFirst'));
    setBatchBusy(true);
    try {
      const result = await api<{ updated: number }>('/api/assets/batch', { method: 'POST', body: JSON.stringify({ ids: [...selectedIds], ...changes }) });
      setSelectedIds(new Set());
      assets.refresh();
      await loadMetadata();
      notify(t('app.batch.updated', { count: result.updated, formattedCount: number(result.updated) }));
    } finally { setBatchBusy(false); }
  };
  const navigateFolder = (targetIndex: number) => {
    setFolder(folderTrail[targetIndex]?.path ?? ''); setFolderDirectCount(null);
  };
  const selectFolder = (nextLibrary: number, nextFolder: string, separator: string) => {
    setCategory('all'); setLibraryId(nextLibrary); setFolder(nextFolder);
    setFolderSeparator(separator); setFolderDirectCount(null); setSidebarOpen(false);
  };
  const selectedFolderInfo = useCallback((directCount: number, separator: string) => {
    setFolderDirectCount(directCount); setFolderSeparator(separator);
  }, []);
  const missingFolder = useCallback(() => { setFolder(''); setFolderDirectCount(null); }, []);
  const clearImageFilter = (...keys: (keyof ImageFilters)[]) => setImageFilters(previous => {
    const next = { ...previous }; keys.forEach(key => { next[key] = ''; }); return next;
  });
  const rangeLabel = (minimum: string, maximum: string, unit: string) => `${minimum || '0'}–${maximum || '∞'} ${unit}`;

  return <div className={`app-shell ${!mobileViewport && sidebarCollapsed ? 'sidebar-collapsed' : ''}`}>
    {sidebarOpen && <button className="sidebar-backdrop" aria-label={t('app.closeNavigation')} onClick={() => setSidebarOpen(false)} />}
    <aside id="picsoc-sidebar" className={`sidebar ${sidebarOpen ? 'open' : ''}`} aria-hidden={!sidebarVisible} inert={!sidebarVisible}>
      <button className="icon-button sidebar-dismiss" aria-label={t('app.closeNavigation')} onClick={() => setSidebarOpen(false)}><X size={18} /></button>
      <a className="brand" href="#" onClick={event => { event.preventDefault(); resetFilters(); setSidebarOpen(false); }} aria-label={t('app.brandHome')}><span className="brand-mark"><Images size={23} strokeWidth={1.8} /></span><span>Picsoc<span className="brand-subtitle">{t('app.brandSubtitle')}</span></span></a>
      <div className="sidebar-scroll">
        <nav className="primary-navigation" aria-label={t('app.mainNavigation')}>
          <button className={`navigation-item ${category === 'all' && libraryId === null ? 'active' : ''}`} aria-current={category === 'all' && libraryId === null ? 'page' : undefined} onClick={() => setScope('all', null)}><LayoutGrid size={18} /><span>{t('app.allAssets')}</span><span className="nav-count">{number(stats.total_assets)}</span></button>
          <button className={`navigation-item ${category === 'favorites' ? 'active' : ''}`} aria-current={category === 'favorites' ? 'page' : undefined} onClick={() => setScope('favorites', null)}><Star size={18} /><span>{t('app.favorites')}</span><span className="nav-count">{number(stats.total_favorites)}</span></button>
        </nav>
        <section className="sidebar-section"><div className="sidebar-section-heading"><h2>{t('app.libraries')}</h2><button className="icon-button compact" onClick={() => setShowAdd(true)} aria-label={t('app.addLibrary')}><Plus size={16} /></button></div>
          <div className="library-list">{libraries.map(library => <LibraryTree key={library.id} library={library} active={libraryId === library.id} selectedFolder={libraryId === library.id ? folder : ''} revision={foldersRevision} onSelect={selectFolder} onManage={manage} onSelectedInfo={selectedFolderInfo} onMissingFolder={missingFolder} />)}</div>
          {!libraries.length && <p className="sidebar-placeholder">{t('app.noLibraries')}</p>}
          <button className="add-library-link" onClick={() => setShowAdd(true)}><Plus size={15} />{t('app.addLibrary')}</button>
        </section>
        <section className="sidebar-section tag-section"><div className="sidebar-section-heading"><h2>{t('app.tags')}</h2><Tag size={14} /></div><div className="sidebar-tags">{(showAllTags ? tags : tags.slice(0, 16)).map(tag => <button className={`sidebar-tag ${selectedTag === tag.name ? 'active' : ''}`} key={tag.name} onClick={() => { setSelectedTag(previous => previous === tag.name ? '' : tag.name); setSidebarOpen(false); }}><span className="tag-dot" /><span>{tag.name}</span><span className="nav-count">{number(tag.count)}</span></button>)}</div>{!tags.length && <p className="sidebar-placeholder">{t('app.noTags')}</p>}{tags.length > 16 && <button className="show-tags" onClick={() => setShowAllTags(value => !value)}>{showAllTags ? t('app.collapseTags') : t('app.viewAllTags', { count: tags.length, formattedCount: number(tags.length) })}<ChevronDown size={13} className={showAllTags ? 'rotate' : ''} /></button>}</section>
      </div>
      <div className="sidebar-footer"><span className="storage-icon"><HardDrive size={17} /></span><div><strong>{formatSize(stats.total_size)}</strong><span>{t('app.storageSummary', { count: libraries.length, formattedCount: number(libraries.length) })}</span></div><span className={`connection-dot ${serviceError ? 'disconnected' : ''}`} title={t(serviceError ? 'app.serviceDisconnected' : 'app.serviceConnected')} /></div>
    </aside>

    <main className="workspace">
      <header className="topbar"><div className="topbar-leading"><button className="icon-button sidebar-toggle" onClick={toggleSidebar} aria-label={sidebarLabel} title={sidebarLabel} aria-expanded={sidebarVisible} aria-controls="picsoc-sidebar">{mobileViewport ? <Menu size={21} /> : sidebarCollapsed ? <PanelLeftOpen size={20} /> : <PanelLeftClose size={20} />}</button><span className="topbar-location"><Images size={18} />{t('app.assetSpace')}</span></div><label className="search-field"><Search size={18} /><input ref={searchInput} type="search" value={search} onChange={event => setSearch(event.target.value)} placeholder={t('app.searchPlaceholder')} aria-label={t('app.searchAssets')} />{!search && <kbd aria-hidden="true">/</kbd>}{search && <button className="icon-button compact" onClick={() => setSearch('')} aria-label={t('app.clearSearch')}><X size={15} /></button>}</label><div className="topbar-actions"><LanguageMenu />{onLogout && <button className="icon-button logout-button" aria-label={t('app.auth.signOut')} title={t('app.auth.signOut')} disabled={logoutBusy} onClick={() => { if (logoutBusy) return; setLogoutBusy(true); void onLogout().catch(cause => notify(errorMessage(cause), true)).finally(() => setLogoutBusy(false)); }}>{logoutBusy ? <LoaderCircle size={17} className="spin" /> : <LogOut size={17} />}</button>}<button className="button primary top-add" onClick={() => setShowAdd(true)} aria-label={t('app.addLibrary')}><Plus size={16} /><span>{t('app.addLibrary')}</span></button></div></header>
      <section className="workspace-heading"><div><h1>{title}<span className="heading-count">{assets.total === null ? '…' : number(assets.total)}</span></h1><p>{activeLibrary ? activeLibrary.path : t(category === 'favorites' ? 'app.favoritesDescription' : 'app.allAssetsDescription')}</p></div><div className="workspace-tools"><button className={`button selection-toggle ${selectionMode ? 'active' : 'secondary'}`} aria-pressed={selectionMode} disabled={batchBusy || (!selectionMode && !assets.total)} onClick={() => { setSelectionMode(value => !value); setSelectedIds(new Set()); setSelected(null); }}><ListChecks size={16} /><span>{t(selectionMode ? 'app.batch.done' : 'app.batch.start')}</span></button></div></section>
      <div className="toolbar"><div className="toolbar-controls"><button className={`button filter-toggle ${filterCount ? 'active' : 'secondary'}`} onClick={() => setShowFilters(true)} aria-haspopup="dialog"><SlidersHorizontal size={15} />{t('app.filters.title')}{filterCount > 0 && <span className="filter-count">{filterCount}</span>}</button><label className="select-control sort-select"><select value={sort} onChange={event => setSort(event.target.value)} aria-label={t('app.sortBy')}>{SORT_OPTIONS.map(value => <option key={value} value={value}>{t(SORT_KEYS[value])}</option>)}</select><ChevronDown size={12} /></label><div className="density-controls" role="group" aria-label={t('app.gridDensity')}><button className={display.columns === null && density === 280 ? 'active' : ''} onClick={() => setGridDensity(280)} aria-label={t('app.largeGrid')} aria-pressed={display.columns === null && density === 280} title={t('app.large')}><Grid2X2 size={17} /></button><button className={display.columns === null && density === 220 ? 'active' : ''} onClick={() => setGridDensity(220)} aria-label={t('app.mediumGrid')} aria-pressed={display.columns === null && density === 220} title={t('app.medium')}><LayoutGrid size={17} /></button><button className={display.columns === null && density === 160 ? 'active' : ''} onClick={() => setGridDensity(160)} aria-label={t('app.compactGrid')} aria-pressed={display.columns === null && density === 160} title={t('app.compact')}><Grid3X3 size={17} /></button></div><DisplaySettings preferences={display} columns={columns} onChange={updateDisplay} /><button className="icon-button refresh-button" onClick={() => { assets.refresh(); void loadMetadata(); void loadFolders(); }} aria-label={t('app.refreshList')} title={t('app.refreshList')}><RefreshCw size={16} /></button></div></div>
      {activeLibrary && <section className="folder-navigation" aria-label={t('app.folders.navigation')}>
        <div className="folder-navigation-header"><nav className="folder-breadcrumb" aria-label={t('app.folders.breadcrumb')}><Folder size={14} /><button onClick={() => navigateFolder(-1)} aria-current={!folder ? 'page' : undefined} title={t('app.folders.allInLibrary')}>{activeLibrary.name}</button>{folderTrail.map((item, index) => <span key={item.path}><ChevronRight size={11} /><button onClick={() => navigateFolder(index)} aria-current={index === folderTrail.length - 1 ? 'page' : undefined} title={item.path}>{item.name}</button></span>)}</nav>{folder && <button className="folder-up" onClick={() => navigateFolder(folderTrail.length - 2)}><ArrowUp size={13} />{t('app.folders.up')}</button>}</div>
        <div className="folder-scope"><label className="folder-recursive"><input type="checkbox" checked={folderRecursive} onChange={event => setFolderRecursive(event.target.checked)} />{t('app.folders.includeChildren')}</label>{folderDirectCount !== null && <span>{t('app.folders.directCount', { count: folderDirectCount, formattedCount: number(folderDirectCount) })}</span>}</div>
      </section>}
      {selectionMode && <div className="batch-toolbar" aria-label={t('app.batch.toolbar')}><div className="batch-summary"><span className="batch-selection-count"><Check size={13} />{t('app.batch.selected', { count: selectedIds.size, formattedCount: number(selectedIds.size) })}</span><span className="batch-limit">{t('app.batch.limitShort', { count: MAX_SELECTION })}</span></div><div className="batch-selection-actions"><button onClick={selectVisible} disabled={batchBusy}>{t('app.batch.selectVisible')}</button><button onClick={() => setSelectedIds(new Set())} disabled={batchBusy || !selectedIds.size}>{t('app.batch.clear')}</button></div><div className="batch-actions"><button onClick={() => { void applyBatch({ favorite: true }).catch(cause => notify(errorMessage(cause), true)); }} disabled={batchBusy || !selectedIds.size}><Star size={13} />{t('app.batch.favorite')}</button><button onClick={() => { void applyBatch({ favorite: false }).catch(cause => notify(errorMessage(cause), true)); }} disabled={batchBusy || !selectedIds.size}><Star size={13} />{t('app.batch.unfavorite')}</button><button onClick={() => setShowBatchTags(true)} disabled={batchBusy || !selectedIds.size}><Tag size={13} />{t('app.batch.tags')}</button>{batchBusy && <LoaderCircle size={15} className="spin" aria-label={t('app.batch.applying')} />}</div></div>}
      {filterCount > 0 && <div className="active-filters"><span>{t('app.currentFilters')}</span>
        {excludedKeywords.map(keyword => <button key={keyword} onClick={() => setExcludedNames(previous => parseExcludedNames(previous).filter(item => item !== keyword).join('\n'))} title={t('app.filters.excludedKeyword', { keyword })} aria-label={t('app.filters.remove', { name: t('app.filters.excludedKeyword', { keyword }) })}><span className="excluded-keyword">{t('app.filters.excludedKeyword', { keyword })}</span><X size={12} /></button>)}
        {favoriteOnly && <button onClick={() => setFavoriteOnly(false)} aria-label={t('app.filters.remove', { name: t('app.filters.favoriteOnly') })}><Star size={12} />{t('app.filters.favoriteOnly')}<X size={12} /></button>}
        {selectedTag && <button onClick={() => setSelectedTag('')} aria-label={t('app.filters.remove', { name: selectedTag })}><Tag size={12} />{selectedTag}<X size={12} /></button>}
        {format && <button onClick={() => setFormat('')} aria-label={t('app.filters.remove', { name: format.toUpperCase() })}>{format.toUpperCase()}<X size={12} /></button>}
        {search.trim() && <button onClick={() => { setSearch(''); setDebouncedSearch(''); }} aria-label={t('app.clearSearch')}>{t('app.searchFilter', { query: search.trim() })}<X size={12} /></button>}
        {imageFilters.orientation && <button onClick={() => clearImageFilter('orientation')}>{t(`app.filters.${imageFilters.orientation}`)}<X size={12} /></button>}
        {imageFilters.aspect_ratio && <button onClick={() => clearImageFilter('aspect_ratio')}>{t('app.filters.aspectRatio')} {imageFilters.aspect_ratio}<X size={12} /></button>}
        {(imageFilters.min_width || imageFilters.max_width) && <button onClick={() => clearImageFilter('min_width', 'max_width')}>{t('app.filters.width')} {rangeLabel(imageFilters.min_width, imageFilters.max_width, 'px')}<X size={12} /></button>}
        {(imageFilters.min_height || imageFilters.max_height) && <button onClick={() => clearImageFilter('min_height', 'max_height')}>{t('app.filters.height')} {rangeLabel(imageFilters.min_height, imageFilters.max_height, 'px')}<X size={12} /></button>}
        {(imageFilters.min_size_mb || imageFilters.max_size_mb) && <button onClick={() => clearImageFilter('min_size_mb', 'max_size_mb')}>{rangeLabel(imageFilters.min_size_mb, imageFilters.max_size_mb, 'MB')}<X size={12} /></button>}
        <button className="clear-filters" onClick={clearFilters}>{t('app.clearFilters')}</button>
      </div>}
      {serviceError && <div className="notice error-notice" role="alert"><AlertCircle size={17} /><span>{serviceError}</span><button onClick={() => { void loadMetadata(); assets.refresh(); }}>{t('app.retry')}</button></div>}
      {progressLibrary && <div className="notice scan-notice" role="status"><LoaderCircle size={16} className="spin" /><span>{t('app.scanning')} <strong>{progressLibrary.name}</strong><span className="scan-count">{t('app.scanProcessed', { count: progressLibrary.scan.processed, formattedCount: number(progressLibrary.scan.processed) })}{progressLibrary.scan.total !== null ? ` / ${number(progressLibrary.scan.total)}` : ''}</span></span><span className="scan-availability">{t('app.keepBrowsing')}</span><button className="scan-cancel" disabled={cancelingLibraryId !== null} onClick={() => { void cancelScan(progressLibrary); }}>{t(cancelingLibraryId === progressLibrary.id ? 'app.scanCanceling' : 'app.cancelScan')}</button>{progressLibrary.scan.total !== null && progressLibrary.scan.total > 0 && <div className="scan-progress" style={{ width: `${Math.min(100, progressLibrary.scan.processed / progressLibrary.scan.total * 100)}%` }} />}</div>}
      {!progressLibrary && scanErrors.length > 0 && <div className="notice error-notice"><AlertCircle size={16} /><span>{t('app.scanFailed', { name: scanErrors[0].name })}</span><button onClick={() => manage(scanErrors[0])}>{t('app.viewDetails')}</button></div>}

      <div className="asset-scroll" ref={scrollElement} onScroll={rememberScroll}>
        {assets.error && <div className="notice error-notice asset-error" role="alert"><AlertCircle size={17} /><span>{assets.error}</span><button onClick={assets.refresh}>{t('app.reload')}</button></div>}
        {assets.total === null && !assets.error && <div className="loading-state"><LoaderCircle size={26} className="spin" /><span>{t('app.loadingAssets')}</span></div>}
        {assets.total === 0 && !assets.error && (activeLibrary && !hasFilters ? <div className="empty-state folder-empty-state"><Folder size={44} strokeWidth={1.3} /><h2>{t('app.folders.emptyTitle')}</h2><p>{t(folderRecursive ? 'app.folders.emptyDescription' : 'app.folders.emptyDirectDescription')}</p>{!folderRecursive && <button className="button secondary" onClick={() => setFolderRecursive(true)}>{t('app.folders.includeChildren')}</button>}<button className="button secondary mobile-folder-browse" onClick={() => setSidebarOpen(true)}>{t('app.folders.browseTree')}</button></div> : <EmptyState kind={metadataLoaded && libraries.length === 0 ? 'welcome' : hasFilters ? 'filtered' : 'empty'} onAdd={() => setShowAdd(true)} onReset={() => { clearFilters(); if (category === 'favorites') setCategory('all'); }} />)}
        {assets.total !== null && assets.total > 0 && <div className="virtual-grid" style={{ height: Math.ceil(assets.total / columns) * rowHeight }} aria-label={t('app.assetList')}>
          {rows.map(row => <div className="asset-row" key={row.key} style={{ transform: `translateY(${row.start}px)`, height: rowHeight, gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`, '--image-height': `${imageHeight}px` } as React.CSSProperties}>
            {Array.from({ length: columns }, (_, column) => {
              const index = row.index * columns + column;
              if (index >= (assets.total ?? 0)) return null;
              const asset = assets.get(index);
              return <AssetCard key={asset?.id ?? `placeholder-${index}`} asset={asset} selectionMode={selectionMode} selected={asset ? selectedIds.has(asset.id) : false} selectionDisabled={batchBusy || Boolean(asset && !selectedIds.has(asset.id) && selectedIds.size >= MAX_SELECTION)} onSelect={() => { if (asset) toggleSelection(asset.id); }} onFavorite={item => void favorite(item)} onOpen={() => { if (asset) setSelected({ asset, index }); }} />;
            })}
          </div>)}
        </div>}
      </div>
      <footer className="workspace-footer"><span>{assets.total === null ? t('app.readingAssets') : t('app.assetCount', { count: assets.total, formattedCount: number(assets.total) })}{selectedTag ? t('app.filteredTag', { tag: selectedTag }) : ''}</span><span className="footer-message"><HardDrive size={12} />{t('app.footerMotto')}</span></footer>
    </main>

    {showFilters && <FilterPanel filters={imageFilters} format={format} tag={selectedTag} sort={sort} favoriteOnly={favoriteOnly} excludedNames={excludedNames} tags={tags.map(item => item.name)} onClose={() => setShowFilters(false)} onApply={(nextFilters, nextFormat, nextTag, nextSort, nextFavoriteOnly, nextExcludedNames) => { setImageFilters(nextFilters); setFormat(nextFormat); setSelectedTag(nextTag); setSort(nextSort); setFavoriteOnly(nextFavoriteOnly); setExcludedNames(nextExcludedNames); }} />}
    {showAdd && <AddLibraryDialog onClose={() => setShowAdd(false)} onAdded={library => { setLibraries(previous => [...previous, library]); setScope('all', library.id); void loadMetadata(); assets.refresh(); notify(t('app.libraryAdded')); }} />}
    {showBatchTags && <BatchTagsDialog count={selectedIds.size} existingTags={tags.map(item => item.name)} onClose={() => { if (!batchBusy) setShowBatchTags(false); }} onApply={async (mode, tags) => { await applyBatch(mode === 'add' ? { add_tags: tags } : { remove_tags: tags }); }} />}
    {selected && <AssetPreview asset={selected.asset} index={selected.index} total={assets.total ?? 0} library={libraries.find(item => item.id === selected.asset.library_id)} onClose={() => setSelected(null)} onNavigate={direction => void navigate(direction)} onUpdated={assetUpdated} />}
    {managedLibrary && <Dialog onClose={() => { if (!libraryBusy) setManagedLibrary(null); }} labelledBy="manage-library-title" className="manage-dialog"><button className="icon-button dialog-close" onClick={() => setManagedLibrary(null)} aria-label={t('app.close')} disabled={libraryBusy}><X size={20} /></button><div className="dialog-symbol"><Folder size={25} /></div><h2 id="manage-library-title">{managedLibrary.name}</h2><p className="manage-path">{managedLibrary.path}</p><div className="library-details"><span><strong>{number(managedLibrary.asset_count)}</strong> {t('app.imageNoun', { count: managedLibrary.asset_count })}</span><span className={`library-state ${managedLibrary.scan.state}`}><span />{t(`app.scanState.${managedLibrary.scan.state}`)}</span></div>{managedLibrary.scan.error && <div className="inline-error">{managedLibrary.scan.error}</div>}{libraryError && <div className="inline-error" role="alert">{libraryError}</div>}{confirmRemove ? <div className="remove-confirm"><h3>{t('app.removeLibraryQuestion')}</h3><p>{t('app.removeLibraryDescription')}</p><div className="dialog-actions"><button className="button secondary" onClick={() => setConfirmRemove(false)} disabled={libraryBusy}>{t('app.cancel')}</button><button className="button danger" onClick={() => void removeLibrary(managedLibrary)} disabled={libraryBusy}>{libraryBusy ? <LoaderCircle size={15} className="spin" /> : <Trash2 size={15} />}{t('app.removeIndex')}</button></div></div> : <><p className="quiet-note">{t('app.rescanHint')}</p><div className="dialog-actions library-actions"><button className="button danger-ghost" onClick={() => setConfirmRemove(true)} disabled={libraryBusy || managedLibrary.scan.state === 'scanning'}><Trash2 size={15} />{t('app.removeLibrary')}</button>{managedLibrary.scan.state === 'scanning' ? <button className="button secondary" onClick={() => { void cancelScan(managedLibrary); }} disabled={cancelingLibraryId !== null}>{cancelingLibraryId === managedLibrary.id ? <LoaderCircle size={15} className="spin" /> : <X size={15} />}{t(cancelingLibraryId === managedLibrary.id ? 'app.scanCanceling' : 'app.cancelScan')}</button> : <button className="button primary" onClick={() => void scanLibrary(managedLibrary)} disabled={libraryBusy}>{libraryBusy ? <LoaderCircle size={15} className="spin" /> : <RefreshCw size={15} />}{t('app.rescan')}</button>}</div></>}</Dialog>}
    {toast && <div className={`toast ${toast.error ? 'error' : ''}`} role={toast.error ? 'alert' : 'status'}>{toast.error ? <AlertCircle size={17} /> : <Check size={17} />}<span>{toast.text}</span><button onClick={() => setToast(null)} aria-label={t('app.dismissNotification')}><X size={15} /></button></div>}
  </div>;
}
