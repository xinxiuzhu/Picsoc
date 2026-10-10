import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AlertCircle, ChevronDown, ChevronRight, Folder, LoaderCircle, MoreHorizontal } from 'lucide-react';
import { api, errorMessage } from './api';
import type { Library, LibraryFolder, LibraryFolders } from './api';

interface FolderPage {
  result?: LibraryFolders;
  loading?: boolean;
  error?: string;
  refreshFailed?: boolean;
}

export function folderBreadcrumbs(path: string, separator: string): { name: string; path: string }[] {
  const parts = path.split(separator).filter(Boolean);
  return parts.map((name, index) => ({ name, path: parts.slice(0, index + 1).join(separator) }));
}

export function LibraryTree({ library, active, selectedFolder, revision, onSelect, onManage, onSelectedInfo, onMissingFolder }: {
  library: Library;
  active: boolean;
  selectedFolder: string;
  revision: number;
  onSelect: (libraryId: number, folder: string, separator: string) => void;
  onManage: (library: Library) => void;
  onSelectedInfo: (directCount: number, separator: string) => void;
  onMissingFolder: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const [pages, setPages] = useState<Record<string, FolderPage>>({});
  const pagesRef = useRef(pages);
  const generation = useRef(0);
  const revealedSelection = useRef<string | null>(null);
  const loadedCounts = useRef(new Map<string, number>());
  const separator = useRef('/');
  const controllers = useRef(new Map<string, AbortController>());
  pagesRef.current = pages;
  const number = (value: number) => value.toLocaleString(i18n.resolvedLanguage?.startsWith('en') ? 'en-US' : 'zh-CN');

  const load = useCallback(async (parent: string, force = false, cursor?: string) => {
    if (controllers.current.has(parent) || (!force && Object.hasOwn(pagesRef.current, parent))) return;
    const requestGeneration = generation.current;
    const controller = new AbortController();
    controllers.current.set(parent, controller);
    setPages(previous => ({ ...previous, [parent]: { ...previous[parent], loading: true, error: undefined, refreshFailed: false } }));
    try {
      const parameters = new URLSearchParams({ parent, limit: '200' });
      if (cursor) parameters.set('cursor', cursor);
      const result = await api<LibraryFolders>(`/api/libraries/${library.id}/folders?${parameters.toString()}`, { signal: controller.signal });
      if (!controller.signal.aborted && requestGeneration === generation.current) {
        if (!parent) separator.current = result.separator;
        setPages(previous => {
          const previousFolders = cursor ? previous[parent]?.result?.folders ?? [] : [];
          const paths = new Set(previousFolders.map(item => item.path));
          const folders = [...previousFolders, ...result.folders.filter(item => !paths.has(item.path))];
          return { ...previous, [parent]: { result: { ...result, folders } } };
        });
      }
    } catch (cause) {
      if (!controller.signal.aborted && requestGeneration === generation.current) {
        setPages(previous => ({ ...previous, [parent]: { ...previous[parent], loading: false, error: errorMessage(cause) } }));
      }
    } finally {
      if (requestGeneration === generation.current) controllers.current.delete(parent);
    }
  }, [library.id]);

  const refresh = useCallback(async (parent: string, targetCount: number) => {
    if (controllers.current.has(parent)) return;
    const requestGeneration = generation.current;
    const controller = new AbortController();
    controllers.current.set(parent, controller);
    setPages(previous => ({ ...previous, [parent]: { ...previous[parent], loading: true, error: undefined, refreshFailed: false } }));
    try {
      const folders: LibraryFolder[] = [];
      const paths = new Set<string>();
      let cursor: string | null = null;
      let result: LibraryFolders;
      do {
        const parameters = new URLSearchParams({ parent, limit: '200' });
        if (cursor) parameters.set('cursor', cursor);
        result = await api<LibraryFolders>(`/api/libraries/${library.id}/folders?${parameters.toString()}`, { signal: controller.signal });
        if (controller.signal.aborted || requestGeneration !== generation.current) return;
        for (const item of result.folders) {
          if (!paths.has(item.path)) { paths.add(item.path); folders.push(item); }
        }
        cursor = result.next_cursor;
      } while (cursor && folders.length < targetCount);
      if (!parent) separator.current = result.separator;
      // Publish the complete fresh page set at once, keeping the sidebar height stable.
      setPages(previous => ({ ...previous, [parent]: { result: { ...result, folders } } }));
    } catch (cause) {
      if (!controller.signal.aborted && requestGeneration === generation.current) {
        setPages(previous => ({ ...previous, [parent]: { ...previous[parent], loading: false, error: errorMessage(cause), refreshFailed: true } }));
      }
    } finally {
      if (requestGeneration === generation.current) controllers.current.delete(parent);
    }
  }, [library.id]);

  // Refresh visible branches in the background; discard only hidden cached branches.
  useEffect(() => {
    if (library.scan.state === 'scanning') return;
    const retainedCounts = new Map<string, number>();
    const visible = (path: string) => expanded.has('') && expanded.has(path)
      && folderBreadcrumbs(path, separator.current).slice(0, -1).every(item => expanded.has(item.path));
    loadedCounts.current.forEach((count, path) => { if (visible(path)) retainedCounts.set(path, count); });
    const rememberVisible = (parent: string) => {
      if (!expanded.has(parent)) return;
      const result = pagesRef.current[parent]?.result;
      if (!result) return;
      retainedCounts.set(parent, Math.max(result.folders.length, retainedCounts.get(parent) ?? 0));
      result.folders.forEach(item => { if (item.has_children) rememberVisible(item.path); });
    };
    rememberVisible('');
    if (active) {
      const parents = ['', ...folderBreadcrumbs(selectedFolder, separator.current).slice(0, -1).map(item => item.path)];
      for (const parent of parents) {
        retainedCounts.set(parent, Math.max(pagesRef.current[parent]?.result?.folders.length ?? 0, retainedCounts.get(parent) ?? 0));
      }
    }
    loadedCounts.current = retainedCounts;
    generation.current++;
    controllers.current.forEach(controller => controller.abort());
    controllers.current.clear();
    const retainedPages = Object.fromEntries(Object.entries(pagesRef.current).filter(([parent]) => retainedCounts.has(parent)));
    pagesRef.current = retainedPages;
    setPages(retainedPages);
    retainedCounts.forEach((count, parent) => { void refresh(parent, count); });
  }, [library.asset_count, library.scan.state, revision, i18n.resolvedLanguage, refresh]);

  useEffect(() => {
    if (!active) { revealedSelection.current = null; return; }
    const selection = `${library.id}:${selectedFolder}`;
    if (revealedSelection.current === selection) return;
    const separator = pages['']?.result?.separator;
    if (!selectedFolder || separator) revealedSelection.current = selection;
    setExpanded(previous => {
      const next = new Set(previous);
      next.add('');
      if (separator) folderBreadcrumbs(selectedFolder, separator).forEach(item => next.add(item.path));
      return next.size === previous.size ? previous : next;
    });
  }, [active, selectedFolder, library.id, pages['']?.result?.separator]);

  useEffect(() => {
    if (active) void load('');
    const loadVisible = (parent: string) => {
      if (!expanded.has(parent)) return;
      void load(parent);
      const page = pages[parent];
      const result = page?.result;
      if (result?.next_cursor && !page.loading && !page.error && result.folders.length < (loadedCounts.current.get(parent) ?? 0)) {
        void load(parent, true, result.next_cursor);
      }
      result?.folders.forEach(item => { if (item.has_children) loadVisible(item.path); });
    };
    loadVisible('');
  }, [active, expanded, pages, load]);

  useEffect(() => {
    if (!active) return;
    const separator = pages['']?.result?.separator;
    if (!separator) return;
    if (!selectedFolder) {
      const root = pages['']?.result;
      if (root) onSelectedInfo(root.direct_asset_count, separator);
      return;
    }
    const ancestors = folderBreadcrumbs(selectedFolder, separator);
    for (let index = 0; index < ancestors.length; index++) {
      const item = ancestors[index];
      const parent = index ? ancestors[index - 1].path : '';
      const page = pages[parent];
      if (page?.loading || page?.error) return;
      const listing = page?.result;
      if (!listing) { void load(parent); return; }
      const match = listing.folders.find(candidate => candidate.path === item.path);
      if (!match && listing.next_cursor) {
        if (!page.error) void load(parent, true, listing.next_cursor);
        return;
      }
      if (!match && !listing.truncated && library.scan.state !== 'scanning') {
        onMissingFolder();
        return;
      }
      if (match && index === ancestors.length - 1) onSelectedInfo(match.direct_asset_count, separator);
    }
  }, [active, selectedFolder, pages, library.scan.state, load, onSelectedInfo, onMissingFolder]);

  useEffect(() => () => {
    generation.current++;
    controllers.current.forEach(controller => controller.abort());
  }, []);

  const toggle = (path: string) => setExpanded(previous => {
    const next = new Set(previous);
    if (next.has(path)) next.delete(path); else next.add(path);
    return next;
  });
  const renderChildren = (parent: string, depth: number) => {
    if (!expanded.has(parent)) return null;
    const page = pages[parent];
    return <ul className="folder-tree-children" aria-label={t('app.folders.childrenOf', { name: parent || library.name })}>
      {page?.loading && !page.result && <li className="folder-tree-status" role="status"><LoaderCircle size={12} className="spin" />{t('app.folders.loading')}</li>}
      {page?.error && <li className="folder-tree-status folder-tree-error" role="alert"><span>{page.error}</span><button onClick={() => {
        if (page.refreshFailed) void refresh(parent, Math.max(page.result?.folders.length ?? 0, loadedCounts.current.get(parent) ?? 0));
        else void load(parent, true, page.result?.next_cursor ?? undefined);
      }}>{t('app.retry')}</button></li>}
      {page?.result?.folders.map(item => renderNode(item, depth))}
      {page?.result && !page.result.folders.length && <li className="folder-tree-status">{t('app.folders.noSubfolders')}</li>}
      {page?.result?.next_cursor && <li className="folder-tree-more"><button disabled={page.loading || page.refreshFailed} onClick={() => {
        loadedCounts.current.set(parent, (page.result?.folders.length ?? 0) + 200);
        void load(parent, true, page.result?.next_cursor ?? undefined);
      }}>{t('app.folders.loadMore')}</button></li>}
    </ul>;
  };
  const renderNode = (item: LibraryFolder, depth: number): React.ReactNode => <li key={item.path}>
    <div className={`folder-tree-row ${active && selectedFolder === item.path ? 'active' : ''}`} style={{ paddingLeft: Math.min(depth, 6) * 12 }}>
      {item.has_children ? <button className="tree-disclosure" onClick={() => toggle(item.path)} aria-expanded={expanded.has(item.path)} aria-label={t(expanded.has(item.path) ? 'app.folders.collapseBranch' : 'app.folders.expandBranch', { name: item.name })}>{pages[item.path]?.loading && expanded.has(item.path) ? <LoaderCircle size={13} className="spin" /> : expanded.has(item.path) ? <ChevronDown size={13} /> : <ChevronRight size={13} />}</button> : <span className="tree-disclosure-spacer" />}
      <button className="folder-tree-select" onClick={() => onSelect(library.id, item.path, pages['']?.result?.separator ?? '/')} title={item.path} aria-current={active && selectedFolder === item.path ? 'page' : undefined}><Folder size={14} /><span>{item.name}</span><small className="nav-count" title={t('app.folders.countIncludingChildren', { count: item.asset_count, formattedCount: number(item.asset_count) })}>{number(item.asset_count)}</small></button>
    </div>
    {item.has_children && renderChildren(item.path, depth + 1)}
  </li>;

  return <div className="library-tree">
    <div className={`library-navigation ${active && !selectedFolder ? 'active' : ''}`}>
      <button className="tree-disclosure library-disclosure" onClick={() => toggle('')} aria-expanded={expanded.has('')} aria-label={t(expanded.has('') ? 'app.folders.collapseBranch' : 'app.folders.expandBranch', { name: library.name })}>{expanded.has('') ? <ChevronDown size={13} /> : <ChevronRight size={13} />}</button>
      <button className="library-select" aria-current={active && !selectedFolder ? 'page' : undefined} onClick={() => onSelect(library.id, '', pages['']?.result?.separator ?? '/')} title={library.path}>{library.scan.state === 'scanning' || pages['']?.loading ? <LoaderCircle size={17} className="spin" /> : library.scan.state === 'error' ? <AlertCircle size={17} className="warning-icon" /> : <Folder size={17} />}<span>{library.name}</span><span className="nav-count">{number(library.asset_count)}</span></button>
      <button className="library-menu" onClick={() => onManage(library)} aria-label={t('app.manageLibrary', { name: library.name })}><MoreHorizontal size={16} /></button>
    </div>
    {renderChildren('', 1)}
  </div>;
}
