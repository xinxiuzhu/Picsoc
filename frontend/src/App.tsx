import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useTranslation } from 'react-i18next';
import {
  AlertCircle, ArrowUp, Check, ChevronDown, ChevronRight, Folder, Grid2X2, Grid3X3,
  HardDrive, Images, Languages, LayoutGrid, ListChecks, LoaderCircle, Menu, MoreHorizontal, Plus,
  RefreshCw, Search, SlidersHorizontal, Star, Tag, Trash2, X,
} from 'lucide-react';
import { api, errorMessage, formatSize } from './api';
import type { Asset, BatchChanges, Library, LibraryFolder, LibraryFolders, Stats, Tag as AssetTag } from './api';
import { AddLibraryDialog, AssetCard, AssetPreview, BatchTagsDialog, Dialog, EmptyState } from './components';
import { useAssets } from './useAssets';

type Category = 'all' | 'favorites';
const EMPTY_STATS: Stats = { total_assets: 0, total_size: 0, total_favorites: 0, total_libraries: 0 };
const MAX_SELECTION = 500;

function getSavedDensity(): number {
  try { return Number(localStorage.getItem('picsoc-density')) || 220; }
  catch { return 220; }
}

export default function App() {
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
  const [density, setDensity] = useState(getSavedDensity);
  const [showAdd, setShowAdd] = useState(false);
  const [managedLibrary, setManagedLibrary] = useState<Library | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [libraryBusy, setLibraryBusy] = useState(false);
  const [libraryError, setLibraryError] = useState<string | null>(null);
  const [cancelingLibraryId, setCancelingLibraryId] = useState<number | null>(null);
  const [selected, setSelected] = useState<{ asset: Asset; index: number } | null>(null);
  const [toast, setToast] = useState<{ text: string; error?: boolean } | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [showAllTags, setShowAllTags] = useState(false);
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<number>>(() => new Set());
  const [batchBusy, setBatchBusy] = useState(false);
  const [showBatchTags, setShowBatchTags] = useState(false);
  const [folder, setFolder] = useState('');
  const [folderTrail, setFolderTrail] = useState<LibraryFolder[]>([]);
  const [folders, setFolders] = useState<LibraryFolder[]>([]);
  const [foldersTruncated, setFoldersTruncated] = useState(false);
  const [foldersLoading, setFoldersLoading] = useState(false);
  const [foldersError, setFoldersError] = useState<string | null>(null);
  const [showAllFolders, setShowAllFolders] = useState(false);
  const foldersController = useRef<AbortController | null>(null);
  const foldersGeneration = useRef(0);
  const scrollElement = useRef<HTMLDivElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.isComposing || document.querySelector('[role="dialog"]')) return;
      const target = event.target;
      const editing = target instanceof HTMLElement && (target.matches('input, textarea, select') || target.isContentEditable);
      if (((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') || (!editing && !event.metaKey && !event.ctrlKey && !event.altKey && event.key === '/')) {
        event.preventDefault();
        searchInput.current?.focus();
        searchInput.current?.select();
      }
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, []);
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
    if (libraryId !== null && folder) parameters.set('folder', folder);
    if (category === 'favorites') parameters.set('favorite', 'true');
    if (format) parameters.set('format', format);
    if (selectedTag) parameters.set('tag', selectedTag);
    return parameters.toString();
  }, [debouncedSearch, libraryId, category, format, sort, selectedTag, folder]);
  const assets = useAssets(query);
  const refreshAssets = useRef(assets.refresh);
  refreshAssets.current = assets.refresh;

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

  const loadFolders = useCallback(async () => {
    foldersController.current?.abort();
    const requestGeneration = ++foldersGeneration.current;
    if (libraryId === null) {
      setFolders([]); setFoldersError(null); setFoldersLoading(false); setFoldersTruncated(false);
      return;
    }
    const controller = new AbortController();
    foldersController.current = controller;
    setFoldersLoading(true);
    try {
      const result = await api<LibraryFolders>(`/api/libraries/${libraryId}/folders?parent=${encodeURIComponent(folder)}`, { signal: controller.signal });
      if (controller.signal.aborted || requestGeneration !== foldersGeneration.current) return;
      setFolders(result.folders); setFoldersTruncated(result.truncated); setFoldersError(null);
    } catch (cause) {
      if (!controller.signal.aborted && requestGeneration === foldersGeneration.current) setFoldersError(errorMessage(cause));
    } finally {
      if (!controller.signal.aborted && requestGeneration === foldersGeneration.current) setFoldersLoading(false);
    }
  }, [libraryId, folder, i18n.resolvedLanguage]);
  const refreshFolders = useRef(loadFolders);
  refreshFolders.current = loadFolders;

  useEffect(() => {
    setFolders([]); setShowAllFolders(false);
    void loadFolders();
    return () => { foldersGeneration.current++; foldersController.current?.abort(); };
  }, [loadFolders]);

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
      setLibraryId(null); setFolder(''); setFolderTrail([]);
    }
    if (cancelingLibraryId !== null && !libraries.some(item => item.id === cancelingLibraryId && item.scan.state === 'scanning')) {
      setCancelingLibraryId(null);
    }
  }, [metadataLoaded, libraries, libraryId, cancelingLibraryId]);

  useEffect(() => () => {
    if (toastTimer.current) clearTimeout(toastTimer.current);
  }, []);

  const columns = Math.max(1, Math.floor((gridWidth + 16) / (density + 16)));
  const cardWidth = (gridWidth - Math.max(0, columns - 1) * 16) / columns;
  const rowHeight = Math.round(cardWidth * 0.75) + 80;
  const virtualizer = useVirtualizer({
    count: Math.ceil((assets.total ?? 0) / columns),
    getScrollElement: () => scrollElement.current,
    estimateSize: () => rowHeight,
    overscan: 3,
  });
  const rows = virtualizer.getVirtualItems();
  const firstVisible = rows[0]?.index ?? 0;
  const lastVisible = rows[rows.length - 1]?.index ?? 0;

  useEffect(() => { virtualizer.measure(); }, [rowHeight, virtualizer]);
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
    const noLongerMatches = (category === 'favorites' && !asset.favorite) || (Boolean(selectedTag) && !asset.tags.includes(selectedTag));
    setSelected(previous => previous?.asset.id === asset.id ? noLongerMatches ? null : { ...previous, asset } : previous);
    void loadMetadata();
    if (category === 'favorites' || selectedTag) assets.refresh();
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
    setFolder(''); setFolderTrail([]);
  };
  const setGridDensity = (value: number) => {
    setDensity(value);
    try { localStorage.setItem('picsoc-density', String(value)); } catch { /* Storage may be disabled. */ }
  };
  const resetFilters = () => {
    setSearch(''); setDebouncedSearch(''); setFormat(''); setSelectedTag('');
    setCategory('all'); setLibraryId(null);
    setFolder(''); setFolderTrail([]);
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
      if (libraryId === library.id) setLibraryId(null);
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

  const hasFilters = Boolean(search.trim() || format || selectedTag || category === 'favorites' || libraryId !== null);
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
    const nextTrail = folderTrail.slice(0, targetIndex + 1);
    setFolderTrail(nextTrail); setFolder(nextTrail.at(-1)?.path ?? '');
  };

  return <div className="app-shell">
    {sidebarOpen && <button className="sidebar-backdrop" aria-label={t('app.closeNavigation')} onClick={() => setSidebarOpen(false)} />}
    <aside className={`sidebar ${sidebarOpen ? 'open' : ''}`}>
      <a className="brand" href="#" onClick={event => { event.preventDefault(); resetFilters(); setSidebarOpen(false); }} aria-label={t('app.brandHome')}><span className="brand-mark"><Images size={23} strokeWidth={1.8} /></span><span>Picsoc<span className="brand-subtitle">{t('app.brandSubtitle')}</span></span></a>
      <div className="sidebar-scroll">
        <nav className="primary-navigation" aria-label={t('app.mainNavigation')}>
          <button className={`navigation-item ${category === 'all' && libraryId === null ? 'active' : ''}`} aria-current={category === 'all' && libraryId === null ? 'page' : undefined} onClick={() => setScope('all', null)}><LayoutGrid size={18} /><span>{t('app.allAssets')}</span><span className="nav-count">{number(stats.total_assets)}</span></button>
          <button className={`navigation-item ${category === 'favorites' ? 'active' : ''}`} aria-current={category === 'favorites' ? 'page' : undefined} onClick={() => setScope('favorites', null)}><Star size={18} /><span>{t('app.favorites')}</span><span className="nav-count">{number(stats.total_favorites)}</span></button>
        </nav>
        <section className="sidebar-section"><div className="sidebar-section-heading"><h2>{t('app.libraries')}</h2><button className="icon-button compact" onClick={() => setShowAdd(true)} aria-label={t('app.addLibrary')}><Plus size={16} /></button></div>
          <div className="library-list">{libraries.map(library => <div className={`library-navigation ${libraryId === library.id ? 'active' : ''}`} key={library.id}><button className="library-select" aria-current={libraryId === library.id ? 'page' : undefined} onClick={() => setScope('all', library.id)} title={library.path}>{library.scan.state === 'scanning' ? <LoaderCircle size={17} className="spin" /> : library.scan.state === 'error' ? <AlertCircle size={17} className="warning-icon" /> : <Folder size={17} />}<span>{library.name}</span><span className="nav-count">{number(library.asset_count)}</span></button><button className="library-menu" onClick={() => manage(library)} aria-label={t('app.manageLibrary', { name: library.name })}><MoreHorizontal size={16} /></button></div>)}</div>
          {!libraries.length && <p className="sidebar-placeholder">{t('app.noLibraries')}</p>}
          <button className="add-library-link" onClick={() => setShowAdd(true)}><Plus size={15} />{t('app.addLibrary')}</button>
        </section>
        <section className="sidebar-section tag-section"><div className="sidebar-section-heading"><h2>{t('app.tags')}</h2><Tag size={14} /></div><div className="sidebar-tags">{(showAllTags ? tags : tags.slice(0, 16)).map(tag => <button className={`sidebar-tag ${selectedTag === tag.name ? 'active' : ''}`} key={tag.name} onClick={() => { setSelectedTag(previous => previous === tag.name ? '' : tag.name); setSidebarOpen(false); }}><span className="tag-dot" /><span>{tag.name}</span><span className="nav-count">{number(tag.count)}</span></button>)}</div>{!tags.length && <p className="sidebar-placeholder">{t('app.noTags')}</p>}{tags.length > 16 && <button className="show-tags" onClick={() => setShowAllTags(value => !value)}>{showAllTags ? t('app.collapseTags') : t('app.viewAllTags', { count: tags.length, formattedCount: number(tags.length) })}<ChevronDown size={13} className={showAllTags ? 'rotate' : ''} /></button>}</section>
      </div>
      <div className="sidebar-footer"><span className="storage-icon"><HardDrive size={17} /></span><div><strong>{formatSize(stats.total_size)}</strong><span>{t('app.storageSummary', { count: libraries.length, formattedCount: number(libraries.length) })}</span></div><span className={`connection-dot ${serviceError ? 'disconnected' : ''}`} title={t(serviceError ? 'app.serviceDisconnected' : 'app.serviceConnected')} /></div>
    </aside>

    <main className="workspace">
      <header className="topbar"><div className="topbar-leading"><button className="icon-button mobile-menu" onClick={() => setSidebarOpen(true)} aria-label={t('app.openNavigation')}><Menu size={21} /></button><span className="topbar-location"><Images size={18} />{t('app.assetSpace')}</span></div><label className="search-field"><Search size={18} /><input ref={searchInput} type="search" value={search} onChange={event => setSearch(event.target.value)} placeholder={t('app.searchPlaceholder')} aria-label={t('app.searchAssets')} />{!search && <kbd aria-hidden="true">/</kbd>}{search && <button className="icon-button compact" onClick={() => setSearch('')} aria-label={t('app.clearSearch')}><X size={15} /></button>}</label><div className="topbar-actions"><label className="language-control"><Languages size={15} /><select aria-label={t('app.selectLanguage')} value={i18n.resolvedLanguage?.startsWith('en') ? 'en' : 'zh-CN'} onChange={event => { void i18n.changeLanguage(event.target.value); }}><option value="zh-CN">{t('app.languageChinese')}</option><option value="en">{t('app.languageEnglish')}</option></select><ChevronDown size={11} /></label><button className="button primary top-add" onClick={() => setShowAdd(true)} aria-label={t('app.addLibrary')}><Plus size={16} /><span>{t('app.addLibrary')}</span></button></div></header>
      <section className="workspace-heading"><div><h1>{title}<span className="heading-count">{assets.total === null ? '…' : number(assets.total)}</span></h1><p>{activeLibrary ? activeLibrary.path : t(category === 'favorites' ? 'app.favoritesDescription' : 'app.allAssetsDescription')}</p></div><div className="workspace-tools"><button className={`button selection-toggle ${selectionMode ? 'active' : 'secondary'}`} aria-pressed={selectionMode} disabled={batchBusy || (!selectionMode && !assets.total)} onClick={() => { setSelectionMode(value => !value); setSelectedIds(new Set()); setSelected(null); }}><ListChecks size={16} /><span>{t(selectionMode ? 'app.batch.done' : 'app.batch.start')}</span></button></div></section>
      <div className="toolbar"><div className="toolbar-controls"><label className="select-control format-select"><SlidersHorizontal size={15} /><select value={format} onChange={event => setFormat(event.target.value)} aria-label={t('app.imageFormat')}><option value="">{t('app.allFormats')}</option>{['jpg', 'png', 'gif', 'webp', 'bmp', 'tiff'].map(value => <option key={value} value={value}>{value.toUpperCase()}</option>)}</select><ChevronDown size={12} /></label><label className="select-control sort-select"><select value={sort} onChange={event => setSort(event.target.value)} aria-label={t('app.sortBy')}><option value="modified">{t('app.sortModified')}</option><option value="name">{t('app.sortName')}</option><option value="size">{t('app.sortSize')}</option></select><ChevronDown size={12} /></label><div className="density-controls" role="group" aria-label={t('app.gridDensity')}><button className={density === 280 ? 'active' : ''} onClick={() => setGridDensity(280)} aria-label={t('app.largeGrid')} aria-pressed={density === 280} title={t('app.large')}><Grid2X2 size={17} /></button><button className={density === 220 ? 'active' : ''} onClick={() => setGridDensity(220)} aria-label={t('app.mediumGrid')} aria-pressed={density === 220} title={t('app.medium')}><LayoutGrid size={17} /></button><button className={density === 160 ? 'active' : ''} onClick={() => setGridDensity(160)} aria-label={t('app.compactGrid')} aria-pressed={density === 160} title={t('app.compact')}><Grid3X3 size={17} /></button></div><button className="icon-button refresh-button" onClick={() => { assets.refresh(); void loadMetadata(); void loadFolders(); }} aria-label={t('app.refreshList')} title={t('app.refreshList')}><RefreshCw size={16} /></button></div></div>
      {activeLibrary && <section className="folder-navigation" aria-label={t('app.folders.navigation')}>
        <div className="folder-navigation-header"><nav className="folder-breadcrumb" aria-label={t('app.folders.breadcrumb')}><Folder size={14} /><button onClick={() => navigateFolder(-1)} aria-current={!folder ? 'page' : undefined} title={t('app.folders.allInLibrary')}>{activeLibrary.name}</button>{folderTrail.map((item, index) => <span key={item.path}><ChevronRight size={11} /><button onClick={() => navigateFolder(index)} aria-current={index === folderTrail.length - 1 ? 'page' : undefined} title={item.path}>{item.name}</button></span>)}</nav><div className="folder-navigation-actions">{foldersLoading && <LoaderCircle size={13} className="spin" aria-label={t('app.folders.loading')} />}{folder && <button className="folder-up" onClick={() => navigateFolder(folderTrail.length - 2)}><ArrowUp size={13} />{t('app.folders.up')}</button>}</div></div>
        {foldersError ? <div className="folder-error" role="alert"><span>{foldersError}</span><button onClick={() => void loadFolders()}>{t('app.retry')}</button></div> : folders.length > 0 ? <div className={`folder-chips ${showAllFolders ? 'expanded' : ''}`}>{(showAllFolders ? folders : folders.slice(0, 12)).map(item => <button key={item.path} className="folder-chip" onClick={() => { setFolder(item.path); setFolderTrail(previous => [...previous, item]); }} title={item.path} aria-label={t('app.folders.open', { name: item.path })}><Folder size={13} /><span>{item.name}</span><small title={t('app.assetCount', { count: item.asset_count, formattedCount: number(item.asset_count) })}>{number(item.asset_count)}</small></button>)}</div> : !foldersLoading && <p className="folder-empty-hint">{t(folder ? 'app.folders.noChildren' : 'app.folders.indexedOnly')}</p>}
        {folders.length > 12 && <button className="folder-show-more" onClick={() => setShowAllFolders(value => !value)}>{t(showAllFolders ? 'app.folders.collapse' : 'app.folders.showAll', { count: folders.length, formattedCount: number(folders.length) })}<ChevronDown size={12} className={showAllFolders ? 'rotate' : ''} /></button>}
        {foldersTruncated && <p className="folder-empty-hint">{t('app.folders.truncated')}</p>}
      </section>}
      {selectionMode && <div className="batch-toolbar" aria-label={t('app.batch.toolbar')}><div className="batch-summary"><span className="batch-selection-count"><Check size={13} />{t('app.batch.selected', { count: selectedIds.size, formattedCount: number(selectedIds.size) })}</span><span className="batch-limit">{t('app.batch.limitShort', { count: MAX_SELECTION })}</span></div><div className="batch-selection-actions"><button onClick={selectVisible} disabled={batchBusy}>{t('app.batch.selectVisible')}</button><button onClick={() => setSelectedIds(new Set())} disabled={batchBusy || !selectedIds.size}>{t('app.batch.clear')}</button></div><div className="batch-actions"><button onClick={() => { void applyBatch({ favorite: true }).catch(cause => notify(errorMessage(cause), true)); }} disabled={batchBusy || !selectedIds.size}><Star size={13} />{t('app.batch.favorite')}</button><button onClick={() => { void applyBatch({ favorite: false }).catch(cause => notify(errorMessage(cause), true)); }} disabled={batchBusy || !selectedIds.size}><Star size={13} />{t('app.batch.unfavorite')}</button><button onClick={() => setShowBatchTags(true)} disabled={batchBusy || !selectedIds.size}><Tag size={13} />{t('app.batch.tags')}</button>{batchBusy && <LoaderCircle size={15} className="spin" aria-label={t('app.batch.applying')} />}</div></div>}
      {(selectedTag || format || search.trim()) && <div className="active-filters"><span>{t('app.currentFilters')}</span>{selectedTag && <button onClick={() => setSelectedTag('')}><Tag size={12} />{selectedTag}<X size={12} /></button>}{format && <button onClick={() => setFormat('')}>{format.toUpperCase()}<X size={12} /></button>}{search.trim() && <button onClick={() => setSearch('')}>{t('app.searchFilter', { query: search.trim() })}<X size={12} /></button>}<button className="clear-filters" onClick={() => { setSearch(''); setFormat(''); setSelectedTag(''); }}>{t('app.clearFilters')}</button></div>}
      {serviceError && <div className="notice error-notice" role="alert"><AlertCircle size={17} /><span>{serviceError}</span><button onClick={() => { void loadMetadata(); assets.refresh(); }}>{t('app.retry')}</button></div>}
      {progressLibrary && <div className="notice scan-notice" role="status"><LoaderCircle size={16} className="spin" /><span>{t('app.scanning')} <strong>{progressLibrary.name}</strong><span className="scan-count">{t('app.scanProcessed', { count: progressLibrary.scan.processed, formattedCount: number(progressLibrary.scan.processed) })}{progressLibrary.scan.total !== null ? ` / ${number(progressLibrary.scan.total)}` : ''}</span></span><span className="scan-availability">{t('app.keepBrowsing')}</span><button className="scan-cancel" disabled={cancelingLibraryId !== null} onClick={() => { void cancelScan(progressLibrary); }}>{t(cancelingLibraryId === progressLibrary.id ? 'app.scanCanceling' : 'app.cancelScan')}</button>{progressLibrary.scan.total !== null && progressLibrary.scan.total > 0 && <div className="scan-progress" style={{ width: `${Math.min(100, progressLibrary.scan.processed / progressLibrary.scan.total * 100)}%` }} />}</div>}
      {!progressLibrary && scanErrors.length > 0 && <div className="notice error-notice"><AlertCircle size={16} /><span>{t('app.scanFailed', { name: scanErrors[0].name })}</span><button onClick={() => manage(scanErrors[0])}>{t('app.viewDetails')}</button></div>}

      <div className="asset-scroll" ref={scrollElement}>
        {assets.error && <div className="notice error-notice asset-error" role="alert"><AlertCircle size={17} /><span>{assets.error}</span><button onClick={assets.refresh}>{t('app.reload')}</button></div>}
        {assets.total === null && !assets.error && <div className="loading-state"><LoaderCircle size={26} className="spin" /><span>{t('app.loadingAssets')}</span></div>}
        {assets.total === 0 && !assets.error && <EmptyState kind={metadataLoaded && libraries.length === 0 ? 'welcome' : hasFilters ? 'filtered' : 'empty'} onAdd={() => setShowAdd(true)} onReset={resetFilters} />}
        {assets.total !== null && assets.total > 0 && <div className="virtual-grid" style={{ height: virtualizer.getTotalSize() }} aria-label={t('app.assetList')}>
          {rows.map(row => <div className="asset-row" key={row.key} style={{ transform: `translateY(${row.start}px)`, height: rowHeight, gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`, '--image-height': `${Math.round(cardWidth * 0.75)}px` } as React.CSSProperties}>
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

    {showAdd && <AddLibraryDialog onClose={() => setShowAdd(false)} onAdded={library => { setLibraries(previous => [...previous, library]); setScope('all', library.id); void loadMetadata(); assets.refresh(); notify(t('app.libraryAdded')); }} />}
    {showBatchTags && <BatchTagsDialog count={selectedIds.size} existingTags={tags.map(item => item.name)} onClose={() => { if (!batchBusy) setShowBatchTags(false); }} onApply={async (mode, tags) => { await applyBatch(mode === 'add' ? { add_tags: tags } : { remove_tags: tags }); }} />}
    {selected && <AssetPreview asset={selected.asset} index={selected.index} total={assets.total ?? 0} library={libraries.find(item => item.id === selected.asset.library_id)} onClose={() => setSelected(null)} onNavigate={direction => void navigate(direction)} onUpdated={assetUpdated} />}
    {managedLibrary && <Dialog onClose={() => { if (!libraryBusy) setManagedLibrary(null); }} labelledBy="manage-library-title" className="manage-dialog"><button className="icon-button dialog-close" onClick={() => setManagedLibrary(null)} aria-label={t('app.close')} disabled={libraryBusy}><X size={20} /></button><div className="dialog-symbol"><Folder size={25} /></div><h2 id="manage-library-title">{managedLibrary.name}</h2><p className="manage-path">{managedLibrary.path}</p><div className="library-details"><span><strong>{number(managedLibrary.asset_count)}</strong> {t('app.imageNoun', { count: managedLibrary.asset_count })}</span><span className={`library-state ${managedLibrary.scan.state}`}><span />{t(`app.scanState.${managedLibrary.scan.state}`)}</span></div>{managedLibrary.scan.error && <div className="inline-error">{managedLibrary.scan.error}</div>}{libraryError && <div className="inline-error" role="alert">{libraryError}</div>}{confirmRemove ? <div className="remove-confirm"><h3>{t('app.removeLibraryQuestion')}</h3><p>{t('app.removeLibraryDescription')}</p><div className="dialog-actions"><button className="button secondary" onClick={() => setConfirmRemove(false)} disabled={libraryBusy}>{t('app.cancel')}</button><button className="button danger" onClick={() => void removeLibrary(managedLibrary)} disabled={libraryBusy}>{libraryBusy ? <LoaderCircle size={15} className="spin" /> : <Trash2 size={15} />}{t('app.removeIndex')}</button></div></div> : <><p className="quiet-note">{t('app.rescanHint')}</p><div className="dialog-actions library-actions"><button className="button danger-ghost" onClick={() => setConfirmRemove(true)} disabled={libraryBusy || managedLibrary.scan.state === 'scanning'}><Trash2 size={15} />{t('app.removeLibrary')}</button>{managedLibrary.scan.state === 'scanning' ? <button className="button secondary" onClick={() => { void cancelScan(managedLibrary); }} disabled={cancelingLibraryId !== null}>{cancelingLibraryId === managedLibrary.id ? <LoaderCircle size={15} className="spin" /> : <X size={15} />}{t(cancelingLibraryId === managedLibrary.id ? 'app.scanCanceling' : 'app.cancelScan')}</button> : <button className="button primary" onClick={() => void scanLibrary(managedLibrary)} disabled={libraryBusy}>{libraryBusy ? <LoaderCircle size={15} className="spin" /> : <RefreshCw size={15} />}{t('app.rescan')}</button>}</div></>}</Dialog>}
    {toast && <div className={`toast ${toast.error ? 'error' : ''}`} role={toast.error ? 'alert' : 'status'}>{toast.error ? <AlertCircle size={17} /> : <Check size={17} />}<span>{toast.text}</span><button onClick={() => setToast(null)} aria-label={t('app.dismissNotification')}><X size={15} /></button></div>}
  </div>;
}
