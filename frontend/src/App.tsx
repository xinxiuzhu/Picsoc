import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useTranslation } from 'react-i18next';
import {
  AlertCircle, ArrowUpRight, Check, ChevronDown, Folder, Grid2X2,
  HardDrive, Images, Languages, LayoutGrid, LoaderCircle, Menu, MoreHorizontal, Plus,
  RefreshCw, Search, SlidersHorizontal, Star, Tag, Trash2, X,
} from 'lucide-react';
import { api, errorMessage, formatSize } from './api';
import type { Asset, Library, Stats, Tag as AssetTag } from './api';
import { AddLibraryDialog, AssetCard, AssetPreview, Dialog, EmptyState } from './components';
import { useAssets } from './useAssets';

type Category = 'all' | 'favorites';
const EMPTY_STATS: Stats = { total_assets: 0, total_size: 0, total_favorites: 0, total_libraries: 0 };

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
  const [selected, setSelected] = useState<{ asset: Asset; index: number } | null>(null);
  const [toast, setToast] = useState<{ text: string; error?: boolean } | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [showAllTags, setShowAllTags] = useState(false);
  const scrollElement = useRef<HTMLDivElement>(null);
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
    if (category === 'favorites') parameters.set('favorite', 'true');
    if (format) parameters.set('format', format);
    if (selectedTag) parameters.set('tag', selectedTag);
    return parameters.toString();
  }, [debouncedSearch, libraryId, category, format, sort, selectedTag]);
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
  }, [query]);

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
  };
  const setGridDensity = (value: number) => {
    setDensity(value);
    try { localStorage.setItem('picsoc-density', String(value)); } catch { /* Storage may be disabled. */ }
  };
  const resetFilters = () => {
    setSearch(''); setDebouncedSearch(''); setFormat(''); setSelectedTag('');
    setCategory('all'); setLibraryId(null);
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

  const hasFilters = Boolean(search.trim() || format || selectedTag || category === 'favorites' || libraryId !== null);
  const progressLibrary = activeLibrary?.scan.state === 'scanning' ? activeLibrary : libraries.find(item => item.scan.state === 'scanning');
  const scanErrors = libraries.filter(item => item.scan.state === 'error');

  return <div className="app-shell">
    {sidebarOpen && <button className="sidebar-backdrop" aria-label={t('app.closeNavigation')} onClick={() => setSidebarOpen(false)} />}
    <aside className={`sidebar ${sidebarOpen ? 'open' : ''}`}>
      <a className="brand" href="#" onClick={event => { event.preventDefault(); resetFilters(); setSidebarOpen(false); }} aria-label={t('app.brandHome')}><span className="brand-mark"><Images size={23} strokeWidth={1.8} /></span><span>Picsoc<span className="brand-subtitle">{t('app.brandSubtitle')}</span></span></a>
      <div className="sidebar-scroll">
        <nav className="primary-navigation" aria-label={t('app.mainNavigation')}>
          <button className={`navigation-item ${category === 'all' && libraryId === null ? 'active' : ''}`} onClick={() => setScope('all', null)}><LayoutGrid size={18} /><span>{t('app.allAssets')}</span><span className="nav-count">{number(stats.total_assets)}</span></button>
          <button className={`navigation-item ${category === 'favorites' ? 'active' : ''}`} onClick={() => setScope('favorites', null)}><Star size={18} /><span>{t('app.favorites')}</span><span className="nav-count">{number(stats.total_favorites)}</span></button>
        </nav>
        <section className="sidebar-section"><div className="sidebar-section-heading"><h2>{t('app.libraries')}</h2><button className="icon-button compact" onClick={() => setShowAdd(true)} aria-label={t('app.addLibrary')}><Plus size={16} /></button></div>
          <div className="library-list">{libraries.map(library => <div className={`library-navigation ${libraryId === library.id ? 'active' : ''}`} key={library.id}><button className="library-select" onClick={() => setScope('all', library.id)} title={library.path}>{library.scan.state === 'scanning' ? <LoaderCircle size={17} className="spin" /> : library.scan.state === 'error' ? <AlertCircle size={17} className="warning-icon" /> : <Folder size={17} />}<span>{library.name}</span><span className="nav-count">{number(library.asset_count)}</span></button><button className="library-menu" onClick={() => manage(library)} aria-label={t('app.manageLibrary', { name: library.name })}><MoreHorizontal size={16} /></button></div>)}</div>
          {!libraries.length && <p className="sidebar-placeholder">{t('app.noLibraries')}</p>}
          <button className="add-library-link" onClick={() => setShowAdd(true)}><Plus size={15} />{t('app.addLibrary')}</button>
        </section>
        <section className="sidebar-section tag-section"><div className="sidebar-section-heading"><h2>{t('app.tags')}</h2><Tag size={14} /></div><div className="sidebar-tags">{(showAllTags ? tags : tags.slice(0, 16)).map(tag => <button className={`sidebar-tag ${selectedTag === tag.name ? 'active' : ''}`} key={tag.name} onClick={() => { setSelectedTag(previous => previous === tag.name ? '' : tag.name); setSidebarOpen(false); }}><span className="tag-dot" /><span>{tag.name}</span><span className="nav-count">{number(tag.count)}</span></button>)}</div>{!tags.length && <p className="sidebar-placeholder">{t('app.noTags')}</p>}{tags.length > 16 && <button className="show-tags" onClick={() => setShowAllTags(value => !value)}>{showAllTags ? t('app.collapseTags') : t('app.viewAllTags', { count: tags.length, formattedCount: number(tags.length) })}<ChevronDown size={13} className={showAllTags ? 'rotate' : ''} /></button>}</section>
      </div>
      <div className="sidebar-footer"><span className="storage-icon"><HardDrive size={17} /></span><div><strong>{formatSize(stats.total_size)}</strong><span>{t('app.storageSummary', { count: libraries.length, formattedCount: number(libraries.length) })}</span></div><span className={`connection-dot ${serviceError ? 'disconnected' : ''}`} title={t(serviceError ? 'app.serviceDisconnected' : 'app.serviceConnected')} /></div>
    </aside>

    <main className="workspace">
      <header className="topbar"><div className="topbar-leading"><button className="icon-button mobile-menu" onClick={() => setSidebarOpen(true)} aria-label={t('app.openNavigation')}><Menu size={21} /></button><span className="breadcrumb">{t('app.assetSpace')}<span>/</span><strong>{title}</strong></span></div><div className="topbar-actions"><label className="language-control"><Languages size={15} /><select aria-label={t('app.selectLanguage')} value={i18n.resolvedLanguage?.startsWith('en') ? 'en' : 'zh-CN'} onChange={event => { void i18n.changeLanguage(event.target.value); }}><option value="zh-CN">{t('app.languageChinese')}</option><option value="en">{t('app.languageEnglish')}</option></select><ChevronDown size={11} /></label><button className="button primary top-add" onClick={() => setShowAdd(true)} aria-label={t('app.addLibrary')}><Plus size={16} /><span>{t('app.addLibrary')}</span></button></div></header>
      <section className="workspace-heading"><div><div className="heading-label">{t('app.headingTagline')}</div><h1>{title}<span className="heading-count">{assets.total === null ? '…' : number(assets.total)}</span></h1><p>{activeLibrary ? activeLibrary.path : t(category === 'favorites' ? 'app.favoritesDescription' : 'app.allAssetsDescription')}</p></div><div className="workspace-icon" aria-hidden="true">{category === 'favorites' ? <Star size={32} strokeWidth={1.2} /> : <Images size={35} strokeWidth={1.2} />}</div></section>
      <div className="toolbar"><label className="search-field"><Search size={18} /><input value={search} onChange={event => setSearch(event.target.value)} placeholder={t('app.searchPlaceholder')} aria-label={t('app.searchAssets')} />{search && <button className="icon-button compact" onClick={() => setSearch('')} aria-label={t('app.clearSearch')}><X size={15} /></button>}</label><div className="toolbar-controls"><label className="select-control format-select"><SlidersHorizontal size={15} /><select value={format} onChange={event => setFormat(event.target.value)} aria-label={t('app.imageFormat')}><option value="">{t('app.allFormats')}</option>{['jpg', 'png', 'gif', 'webp', 'bmp', 'tiff'].map(value => <option key={value} value={value}>{value.toUpperCase()}</option>)}</select><ChevronDown size={12} /></label><label className="select-control sort-select"><select value={sort} onChange={event => setSort(event.target.value)} aria-label={t('app.sortBy')}><option value="modified">{t('app.sortModified')}</option><option value="name">{t('app.sortName')}</option><option value="size">{t('app.sortSize')}</option></select><ChevronDown size={12} /></label><div className="density-controls" aria-label={t('app.gridDensity')}><button className={density === 280 ? 'active' : ''} onClick={() => setGridDensity(280)} aria-label={t('app.largeGrid')} aria-pressed={density === 280} title={t('app.large')}><Grid2X2 size={17} /></button><button className={density === 220 ? 'active' : ''} onClick={() => setGridDensity(220)} aria-label={t('app.mediumGrid')} aria-pressed={density === 220} title={t('app.medium')}><LayoutGrid size={17} /></button><button className={density === 160 ? 'active' : ''} onClick={() => setGridDensity(160)} aria-label={t('app.compactGrid')} aria-pressed={density === 160} title={t('app.compact')}><span className="dense-grid-icon">▦</span></button></div><button className="icon-button refresh-button" onClick={() => { assets.refresh(); void loadMetadata(); }} aria-label={t('app.refreshList')} title={t('app.refreshList')}><RefreshCw size={16} /></button></div></div>
      {(selectedTag || format || search.trim()) && <div className="active-filters"><span>{t('app.currentFilters')}</span>{selectedTag && <button onClick={() => setSelectedTag('')}><Tag size={12} />{selectedTag}<X size={12} /></button>}{format && <button onClick={() => setFormat('')}>{format.toUpperCase()}<X size={12} /></button>}{search.trim() && <button onClick={() => setSearch('')}>{t('app.searchFilter', { query: search.trim() })}<X size={12} /></button>}<button className="clear-filters" onClick={() => { setSearch(''); setFormat(''); setSelectedTag(''); }}>{t('app.clearFilters')}</button></div>}
      {serviceError && <div className="notice error-notice" role="alert"><AlertCircle size={17} /><span>{serviceError}</span><button onClick={() => { void loadMetadata(); assets.refresh(); }}>{t('app.retry')}</button></div>}
      {progressLibrary && <div className="notice scan-notice" role="status"><LoaderCircle size={16} className="spin" /><span>{t('app.scanning')} <strong>{progressLibrary.name}</strong><span className="scan-count">{t('app.scanProcessed', { count: progressLibrary.scan.processed, formattedCount: number(progressLibrary.scan.processed) })}{progressLibrary.scan.total !== null ? ` / ${number(progressLibrary.scan.total)}` : ''}</span></span><span className="scan-availability">{t('app.keepBrowsing')}</span>{progressLibrary.scan.total !== null && progressLibrary.scan.total > 0 && <div className="scan-progress" style={{ width: `${Math.min(100, progressLibrary.scan.processed / progressLibrary.scan.total * 100)}%` }} />}</div>}
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
              return <AssetCard key={asset?.id ?? `placeholder-${index}`} asset={asset} onFavorite={item => void favorite(item)} onOpen={() => { if (asset) setSelected({ asset, index }); }} />;
            })}
          </div>)}
        </div>}
      </div>
      <footer className="workspace-footer"><span>{assets.total === null ? t('app.readingAssets') : t('app.assetCount', { count: assets.total, formattedCount: number(assets.total) })}{selectedTag ? t('app.filteredTag', { tag: selectedTag }) : ''}</span><span className="footer-message">{t('app.footerMotto')} <ArrowUpRight size={12} /></span></footer>
    </main>

    {showAdd && <AddLibraryDialog onClose={() => setShowAdd(false)} onAdded={library => { setLibraries(previous => [...previous, library]); setScope('all', library.id); void loadMetadata(); assets.refresh(); notify(t('app.libraryAdded')); }} />}
    {selected && <AssetPreview asset={selected.asset} index={selected.index} total={assets.total ?? 0} library={libraries.find(item => item.id === selected.asset.library_id)} onClose={() => setSelected(null)} onNavigate={direction => void navigate(direction)} onUpdated={assetUpdated} />}
    {managedLibrary && <Dialog onClose={() => { if (!libraryBusy) setManagedLibrary(null); }} labelledBy="manage-library-title" className="manage-dialog"><button className="icon-button dialog-close" onClick={() => setManagedLibrary(null)} aria-label={t('app.close')} disabled={libraryBusy}><X size={20} /></button><div className="dialog-symbol"><Folder size={25} /></div><h2 id="manage-library-title">{managedLibrary.name}</h2><p className="manage-path">{managedLibrary.path}</p><div className="library-details"><span><strong>{number(managedLibrary.asset_count)}</strong> {t('app.imageNoun', { count: managedLibrary.asset_count })}</span><span className={`library-state ${managedLibrary.scan.state}`}><span />{t(`app.scanState.${managedLibrary.scan.state}`)}</span></div>{managedLibrary.scan.error && <div className="inline-error">{managedLibrary.scan.error}</div>}{libraryError && <div className="inline-error" role="alert">{libraryError}</div>}{confirmRemove ? <div className="remove-confirm"><h3>{t('app.removeLibraryQuestion')}</h3><p>{t('app.removeLibraryDescription')}</p><div className="dialog-actions"><button className="button secondary" onClick={() => setConfirmRemove(false)} disabled={libraryBusy}>{t('app.cancel')}</button><button className="button danger" onClick={() => void removeLibrary(managedLibrary)} disabled={libraryBusy}>{libraryBusy ? <LoaderCircle size={15} className="spin" /> : <Trash2 size={15} />}{t('app.removeIndex')}</button></div></div> : <><p className="quiet-note">{t('app.rescanHint')}</p><div className="dialog-actions library-actions"><button className="button danger-ghost" onClick={() => setConfirmRemove(true)} disabled={libraryBusy || managedLibrary.scan.state === 'scanning'}><Trash2 size={15} />{t('app.removeLibrary')}</button><button className="button primary" onClick={() => void scanLibrary(managedLibrary)} disabled={libraryBusy || managedLibrary.scan.state === 'scanning'}>{libraryBusy || managedLibrary.scan.state === 'scanning' ? <LoaderCircle size={15} className="spin" /> : <RefreshCw size={15} />}{t(managedLibrary.scan.state === 'scanning' ? 'app.scanningEllipsis' : 'app.rescan')}</button></div></>}</Dialog>}
    {toast && <div className={`toast ${toast.error ? 'error' : ''}`} role={toast.error ? 'alert' : 'status'}>{toast.error ? <AlertCircle size={17} /> : <Check size={17} />}<span>{toast.text}</span><button onClick={() => setToast(null)} aria-label={t('app.dismissNotification')}><X size={15} /></button></div>}
  </div>;
}
