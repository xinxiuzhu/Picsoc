import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  ArrowDownToLine, ArrowUp, Check, ChevronLeft, ChevronRight, Copy, Folder, FolderPlus,
  HardDrive, ImageOff, LoaderCircle, Plus, Search, Star, Tag, X,
} from 'lucide-react';
import type { FormEvent, ReactNode } from 'react';
import { api, errorMessage, formatDate, formatSize } from './api';
import type { Asset, DirectoryListing, Library } from './api';

export function Dialog({ children, onClose, className = '', labelledBy }: {
  children: ReactNode;
  onClose: () => void;
  className?: string;
  labelledBy: string;
}) {
  const element = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const oldOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    const focusable = 'button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled), a[href], summary, [tabindex="0"]';
    const visibleItems = (selector: string) => [...(element.current?.querySelectorAll<HTMLElement>(selector) ?? [])].filter(item => item.getClientRects().length > 0);
    const frame = requestAnimationFrame(() => {
      (visibleItems('[data-autofocus]')[0] ?? visibleItems(focusable)[0] ?? element.current)?.focus();
    });
    const keydown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') closeRef.current();
      if (event.key !== 'Tab') return;
      const items = visibleItems(focusable);
      if (!items.length) { event.preventDefault(); return; }
      const first = items[0];
      const last = items[items.length - 1];
      if (event.shiftKey && (document.activeElement === first || !element.current?.contains(document.activeElement))) {
        event.preventDefault(); last.focus();
      } else if (!event.shiftKey && (document.activeElement === last || !element.current?.contains(document.activeElement))) {
        event.preventDefault(); first.focus();
      }
    };
    document.addEventListener('keydown', keydown);
    return () => {
      cancelAnimationFrame(frame);
      document.body.style.overflow = oldOverflow;
      document.removeEventListener('keydown', keydown);
      previous?.focus();
    };
  }, []);

  return <div className="dialog-backdrop" onMouseDown={event => {
    if (event.target === event.currentTarget) onClose();
  }}>
    <div className={`dialog ${className}`} ref={element} role="dialog" aria-modal="true" aria-labelledby={labelledBy} tabIndex={-1}>
      {children}
    </div>
  </div>;
}

export function AddLibraryDialog({ onClose, onAdded }: {
  onClose: () => void;
  onAdded: (library: Library) => void;
}) {
  const { t } = useTranslation();
  const [path, setPath] = useState('');
  const [name, setName] = useState('');
  const [pending, setPending] = useState(false);
  const [showPicker, setShowPicker] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (pending) return;
    setPending(true);
    setError(null);
    try {
      const normalized = path.trim();
      const library = await api<Library>('/api/libraries', {
        method: 'POST',
        body: JSON.stringify({ path: normalized, name: name.trim() || normalized.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || t('components.common.myLibrary') }),
      });
      onAdded(library);
      onClose();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally { setPending(false); }
  };
  if (showPicker) return <DirectoryPicker initialPath={path} onClose={() => setShowPicker(false)} onSelect={(selectedPath, selectedName) => {
    setPath(selectedPath);
    if (!name.trim()) setName(selectedName);
    setShowPicker(false);
  }} />;
  return <Dialog onClose={() => { if (!pending) onClose(); }} labelledBy="add-library-title" className="add-dialog">
    <button className="icon-button dialog-close" onClick={onClose} disabled={pending} aria-label={t('components.common.close')}><X size={20} /></button>
    <div className="dialog-symbol"><FolderPlus size={25} /></div>
    <h2 id="add-library-title">{t('components.addLibrary.title')}</h2>
    <p className="dialog-intro">{t('components.addLibrary.intro')}</p>
    <form onSubmit={submit}>
      <button type="button" className="folder-chooser" onClick={() => setShowPicker(true)} disabled={pending} data-autofocus><span className="chooser-icon"><Folder size={28} strokeWidth={1.6} /></span><span><strong>{t('components.addLibrary.chooseFolder')}</strong><small>{t('components.addLibrary.chooseHint')}</small></span><ChevronRight size={19} /></button>
      <label className="field-label" htmlFor="library-path">{t('components.addLibrary.path')} <span>{t('components.common.required')}</span></label>
      <div className="path-input-row"><input id="library-path" className="text-input" value={path} onChange={event => setPath(event.target.value)} required autoComplete="off" placeholder={t('components.addLibrary.pathPlaceholder')} disabled={pending} /></div>
      <label className="field-label" htmlFor="library-name">{t('components.addLibrary.name')} <span>{t('components.common.optional')}</span></label>
      <input id="library-name" className="text-input" value={name} onChange={event => setName(event.target.value)} autoComplete="off" maxLength={100} placeholder={t('components.addLibrary.namePlaceholder')} disabled={pending} />
      <p className="quiet-note">{t('components.addLibrary.readonly')}</p>
      <details className="import-help"><summary>{t('components.addLibrary.help')}</summary><div className="path-help"><strong>{t('components.addLibrary.pathHelpTitle')}</strong><p>{t('components.addLibrary.pathHelpBefore')} <code>/library</code>{t('components.addLibrary.pathHelpAfter')}</p><p>{t('components.addLibrary.photosHint')}</p></div></details>
      {error && <div className="inline-error" role="alert">{error}</div>}
      <div className="dialog-actions"><button type="button" className="button secondary" onClick={onClose} disabled={pending}>{t('components.common.cancel')}</button><button className="button primary" disabled={pending || !path.trim()}>{pending ? <LoaderCircle size={16} className="spin" /> : <Plus size={16} />}{t(pending ? 'components.addLibrary.adding' : 'components.addLibrary.add')}</button></div>
    </form>
  </Dialog>;
}

function DirectoryPicker({ initialPath, onClose, onSelect }: {
  initialPath: string;
  onClose: () => void;
  onSelect: (path: string, name: string) => void;
}) {
  const { t } = useTranslation();
  const [listing, setListing] = useState<DirectoryListing | null>(null);
  const [draftPath, setDraftPath] = useState(initialPath);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const controller = useRef<AbortController | null>(null);
  const load = async (path: string | null) => {
    controller.current?.abort();
    const activeController = new AbortController();
    controller.current = activeController;
    setLoading(true); setError(null);
    try {
      const result = await api<DirectoryListing>(`/api/directories${path ? `?path=${encodeURIComponent(path)}` : ''}`, { signal: activeController.signal });
      if (activeController.signal.aborted) return;
      setListing(result); setDraftPath(result.path ?? '');
    } catch (cause) {
      if (!activeController.signal.aborted) setError(errorMessage(cause));
    } finally {
      if (!activeController.signal.aborted) setLoading(false);
    }
  };
  useEffect(() => {
    void load(initialPath.trim() || null);
    return () => controller.current?.abort();
    // The initial folder is loaded once; navigation uses load directly.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const currentName = listing?.path?.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || t('components.common.library');

  return <Dialog onClose={onClose} labelledBy="directory-picker-title" className="directory-dialog">
    <div className="directory-title"><div><span className="eyebrow">{t('components.directory.eyebrow')}</span><h2 id="directory-picker-title">{t('components.directory.title')}</h2></div><button className="icon-button" onClick={onClose} aria-label={t('components.directory.close')}><X size={20} /></button></div>
    <p className="directory-hint">{t('components.directory.hint')}</p>
    <form className="directory-path-form" onSubmit={event => { event.preventDefault(); void load(draftPath.trim() || null); }}><input value={draftPath} onChange={event => setDraftPath(event.target.value)} placeholder={t('components.directory.pathPlaceholder')} aria-label={t('components.directory.pathLabel')} autoComplete="off" data-autofocus /><button type="submit" className="icon-button compact" aria-label={t('components.directory.go')} disabled={loading}><ChevronRight size={17} /></button></form>
    <div className="directory-tools"><button onClick={() => void load(null)} disabled={loading}><HardDrive size={14} />{t('components.directory.locations')}</button><button onClick={() => void load(listing?.parent ?? null)} disabled={loading || !listing?.path}><ArrowUp size={14} />{t('components.directory.up')}</button><span>{loading ? t('components.directory.reading') : listing?.path ? t('components.directory.subfolders', { count: listing.directories.length }) : t('components.directory.selectLocation')}</span></div>
    {error && <div className="inline-error" role="alert">{error}</div>}
    <div className="directory-list" aria-label={t('components.directory.listLabel')} aria-busy={loading}>
      {loading ? <div className="directory-loading"><LoaderCircle className="spin" size={24} /><span>{t('components.directory.readingFolders')}</span></div> : <>
        {listing && !listing.path && listing.roots.map(root => <button className="directory-item" key={root.path} onClick={() => void load(root.path)}><span className="directory-folder-icon"><HardDrive size={18} /></span><span><strong>{root.name === '用户目录' ? t('components.directory.home') : root.name === '文件系统 /' ? t('components.directory.filesystem') : root.name}</strong><small>{root.path}</small></span><ChevronRight size={15} /></button>)}
        {listing?.path && listing.directories.map(directory => <button className="directory-item" key={directory.path} onClick={() => void load(directory.path)} title={directory.path}><span className="directory-folder-icon"><Folder size={18} /></span><span><strong>{directory.name}</strong></span><ChevronRight size={15} /></button>)}
        {listing?.path && !listing.directories.length && <div className="directory-empty"><Folder size={26} /><p>{t('components.directory.noChildren')}</p><span>{t('components.directory.noChildrenHint')}</span></div>}
        {listing?.truncated && <p className="quiet-note directory-truncated">{t('components.directory.truncated')}</p>}
      </>}
    </div>
    <div className="directory-selection"><span>{t('components.directory.current')}</span><strong title={listing?.path ?? ''}>{listing?.path ?? t('components.directory.unselected')}</strong></div>
    <div className="dialog-actions"><button type="button" className="button secondary" onClick={onClose}>{t('components.common.back')}</button><button type="button" className="button primary" disabled={!listing?.path || loading || Boolean(error)} onClick={() => { if (listing?.path) onSelect(listing.path, currentName); }}><Check size={15} />{t('components.directory.select')}</button></div>
  </Dialog>;
}

export function Thumbnail({ asset }: { asset: Asset }) {
  const { t } = useTranslation();
  const [attempt, setAttempt] = useState(0);
  const [failed, setFailed] = useState(false);
  const [loaded, setLoaded] = useState(false);
  useEffect(() => {
    if (!failed) return;
    const timer = setTimeout(() => {
      setFailed(false);
      setAttempt(value => value + 1);
    }, Math.min(1000 * 2 ** attempt, 30000));
    return () => clearTimeout(timer);
  }, [failed, attempt]);
  const url = attempt ? `${asset.thumbnail_url}${asset.thumbnail_url.includes('?') ? '&' : '?'}retry=${attempt}` : asset.thumbnail_url;
  return <>
    {!loaded && <div className="thumbnail-placeholder" aria-hidden="true">{failed && attempt >= 4 ? <><ImageOff size={23} /><span>{t('components.thumbnail.unavailable')}</span></> : <span className="thumbnail-shimmer" />}</div>}
    {!failed && <img src={url} alt="" loading="lazy" decoding="async" onLoad={() => setLoaded(true)} onError={() => { setFailed(true); setLoaded(false); }} className={loaded ? 'thumbnail loaded' : 'thumbnail'} />}
  </>;
}

export function AssetCard({ asset, onOpen, onFavorite, selectionMode = false, selected = false, selectionDisabled = false, onSelect }: {
  asset: Asset | undefined;
  onOpen: () => void;
  onFavorite: (asset: Asset) => void;
  selectionMode?: boolean;
  selected?: boolean;
  selectionDisabled?: boolean;
  onSelect?: () => void;
}) {
  const { t } = useTranslation();
  if (!asset) return <div className="asset-card skeleton-card" aria-hidden="true"><div className="card-image skeleton" /><div className="skeleton-line" /><div className="skeleton-line short" /></div>;
  return <article className={`asset-card ${selected ? 'is-selected' : ''}`}>
    <button className="card-open" onClick={selectionMode ? onSelect : onOpen} aria-label={t(selectionMode ? selected ? 'components.card.unselect' : 'components.card.select' : 'components.card.preview', { name: asset.name })} aria-pressed={selectionMode ? selected : undefined} disabled={selectionMode && selectionDisabled}>
      <div className="card-image"><Thumbnail key={asset.thumbnail_url} asset={asset} /><span className="format-badge">{asset.format.toUpperCase()}</span></div>
      <div className="card-caption"><h3 title={asset.name}>{asset.name}</h3><div className="card-meta"><span>{asset.width && asset.height ? `${asset.width} × ${asset.height}` : t('components.card.dimensionsPending')}</span><span>{formatSize(asset.size)}</span></div></div>
    </button>
    {selectionMode ? <label className="card-select-control"><input type="checkbox" checked={selected} onChange={onSelect} disabled={selectionDisabled} aria-label={t('components.card.select', { name: asset.name })} /><span aria-hidden="true">{selected && <Check size={13} strokeWidth={2.7} />}</span></label> : <button className={`card-favorite ${asset.favorite ? 'is-favorite' : ''}`} onClick={() => onFavorite(asset)} aria-label={t(asset.favorite ? 'components.card.unfavoriteName' : 'components.card.favoriteName', { name: asset.name })} aria-pressed={asset.favorite} title={t(asset.favorite ? 'components.card.unfavorite' : 'components.card.favorite')}><Star size={15} fill={asset.favorite ? 'currentColor' : 'none'} /></button>}
    {asset.tags.length > 0 && <span className="card-tag-count" title={asset.tags.join(t('components.common.tagSeparator'))}><Tag size={11} />{asset.tags.length}</span>}
  </article>;
}

export function BatchTagsDialog({ count, existingTags, onClose, onApply }: {
  count: number;
  existingTags: string[];
  onClose: () => void;
  onApply: (mode: 'add' | 'remove', tags: string[]) => Promise<void>;
}) {
  const { t, i18n } = useTranslation();
  const [mode, setMode] = useState<'add' | 'remove'>('add');
  const [input, setInput] = useState('');
  const [chosenTags, setChosenTags] = useState<string[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const parseTags = () => [...new Set([...chosenTags, ...input.split(/[,，;；\n]/).map(tag => tag.trim()).filter(Boolean)])];
  const addInput = () => {
    const next = parseTags();
    if (next.length > 50 || next.some(tag => [...tag].length > 50)) {
      setError(t('components.batchTags.limitError'));
      return chosenTags;
    }
    setError(null);
    setChosenTags(next); setInput('');
    return next;
  };
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (pending) return;
    const selectedTags = parseTags();
    if (!selectedTags.length) return;
    if (selectedTags.length > 50 || selectedTags.some(tag => [...tag].length > 50)) {
      setError(t('components.batchTags.limitError'));
      return;
    }
    setPending(true); setError(null);
    try { await onApply(mode, selectedTags); onClose(); }
    catch (cause) { setError(errorMessage(cause)); }
    finally { setPending(false); }
  };
  return <Dialog onClose={() => { if (!pending) onClose(); }} labelledBy="batch-tags-title" className="add-dialog batch-tags-dialog">
    <button className="icon-button dialog-close" onClick={onClose} disabled={pending} aria-label={t('components.common.close')}><X size={20} /></button>
    <div className="dialog-symbol"><Tag size={24} /></div>
    <h2 id="batch-tags-title">{t('components.batchTags.title')}</h2>
    <p className="dialog-intro">{t('components.batchTags.intro', { count, formattedCount: count.toLocaleString(i18n.language === 'en' ? 'en-US' : 'zh-CN') })}</p>
    <div className="batch-tag-mode" role="group" aria-label={t('components.batchTags.action')}><button className={mode === 'add' ? 'active' : ''} onClick={() => setMode('add')} aria-pressed={mode === 'add'} disabled={pending}><Plus size={14} />{t('components.batchTags.add')}</button><button className={mode === 'remove' ? 'active' : ''} onClick={() => setMode('remove')} aria-pressed={mode === 'remove'} disabled={pending}><X size={14} />{t('components.batchTags.remove')}</button></div>
    <form onSubmit={event => void submit(event)}>
      <label className="field-label" htmlFor="batch-tags-input">{t('components.preview.tags')}</label>
      {chosenTags.length > 0 && <div className="editable-tags">{chosenTags.map(tag => <span className="tag-chip" key={tag}>{tag}<button type="button" onClick={() => setChosenTags(chosenTags.filter(item => item !== tag))} disabled={pending} aria-label={t('components.preview.removeTag', { tag })}><X size={12} /></button></span>)}</div>}
      <textarea id="batch-tags-input" className="batch-tags-input" ref={inputRef} value={input} onChange={event => setInput(event.target.value)} placeholder={t('components.batchTags.placeholder')} rows={3} maxLength={2500} disabled={pending} data-autofocus onKeyDown={event => { if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); addInput(); } }} />
      <p className="quiet-note">{t(mode === 'add' ? 'components.batchTags.addHint' : 'components.batchTags.removeHint')}</p>
      {existingTags.length > 0 && <div className="batch-tag-suggestions"><span>{t('components.batchTags.existingTags')}</span><div>{existingTags.slice(0, 30).map(tag => <button type="button" key={tag} disabled={pending || chosenTags.includes(tag) || chosenTags.length >= 50} onClick={() => { setChosenTags([...new Set([...chosenTags, tag])]); inputRef.current?.focus(); }}>{tag}<Plus size={11} /></button>)}</div></div>}
      {error && <div className="inline-error" role="alert">{error}</div>}
      <div className="dialog-actions"><button type="button" className="button secondary" onClick={onClose} disabled={pending}>{t('components.common.cancel')}</button><button className="button primary" disabled={pending || !parseTags().length}>{pending ? <LoaderCircle size={15} className="spin" /> : <Check size={15} />}{t(pending ? 'components.batchTags.applying' : 'components.batchTags.apply')}</button></div>
    </form>
  </Dialog>;
}

export function EmptyState({ kind, onAdd, onReset }: { kind: 'welcome' | 'filtered' | 'empty'; onAdd: () => void; onReset: () => void }) {
  const { t } = useTranslation();
  return <div className="empty-state">
    <div className="empty-illustration" aria-hidden="true"><span className="empty-frame back" /><span className="empty-frame front"><span className="empty-sun" /><span className="empty-mountain" /></span><span className="empty-spark">✦</span></div>
    <span className="eyebrow">{t(kind === 'welcome' ? 'components.empty.welcomeEyebrow' : 'components.empty.otherEyebrow')}</span>
    <h2>{t(`components.empty.${kind}Title`)}</h2>
    <p>{t(`components.empty.${kind}Description`)}</p>
    <button className="button primary" onClick={kind === 'filtered' ? onReset : onAdd}>{kind === 'filtered' ? <Search size={16} /> : <FolderPlus size={17} />}{t(kind === 'filtered' ? 'components.empty.reset' : 'components.empty.add')}</button>
    {kind === 'welcome' && <div className="empty-features"><span>{t('components.empty.keepOriginals')}</span><i /><span>{t('components.empty.localData')}</span><i /><span>{t('components.empty.gif')}</span></div>}
  </div>;
}

/** A fresh URL owns fresh loading state, including images served from browser cache. */
function PreviewImage({ url, name, retry }: { url: string; name: string; retry: boolean }) {
  const { t } = useTranslation();
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!retry || !failed || attempt >= 4) return;
    const timer = setTimeout(() => {
      setFailed(false);
      setAttempt(value => value + 1);
    }, Math.min(1000 * 2 ** attempt, 8000));
    return () => clearTimeout(timer);
  }, [retry, failed, attempt]);
  const source = attempt ? `${url}${url.includes('?') ? '&' : '?'}retry=${attempt}` : url;
  const showError = failed && (!retry || attempt >= 4);
  return <>
    {!loaded && !showError && <LoaderCircle className="spin preview-loader" size={28} />}
    {showError ? <div className="preview-image-error"><ImageOff size={40} /><p>{t('components.preview.loadError')}</p><span>{t('components.preview.loadErrorReason')}<br />{t('components.preview.loadErrorAction')}</span></div>
      : !failed && <img key={attempt} src={source} alt={name} onLoad={() => setLoaded(true)} onError={() => { setFailed(true); setLoaded(false); }} className={loaded ? 'original-image visible' : 'original-image'} />}
  </>;
}

export function AssetPreview({ asset: initialAsset, library, index, total, onClose, onNavigate, onUpdated }: {
  asset: Asset;
  library?: Library;
  index: number;
  total: number;
  onClose: () => void;
  onNavigate: (direction: number) => void;
  onUpdated: (asset: Asset) => void;
}) {
  const { t, i18n } = useTranslation();
  const [asset, setAsset] = useState(initialAsset);
  const [tags, setTags] = useState(initialAsset.tags);
  const [tagInput, setTagInput] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [copiedId, setCopiedId] = useState(false);
  const copyTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const navigationRef = useRef(onNavigate);
  navigationRef.current = direction => { if (!busy) onNavigate(direction); };
  const isTiff = asset.format.toLowerCase() === 'tiff' || asset.format.toLowerCase() === 'tif';
  const previewBaseUrl = isTiff ? asset.thumbnail_url : asset.original_url;

  useEffect(() => {
    setAsset(initialAsset);
    setTags(initialAsset.tags);
    setTagInput('');
    setError(null);
    setCopied(false);
    setCopiedId(false);
    const controller = new AbortController();
    void api<Asset>(`/api/assets/${initialAsset.id}`, { signal: controller.signal }).then(result => {
      setAsset(result); setTags(result.tags);
    }).catch(cause => { if (!controller.signal.aborted) setError(errorMessage(cause)); });
    return () => controller.abort();
    // Load fresh metadata when navigation selects a different asset.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialAsset.id]);

  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement;
      if (target.matches('input, textarea, select') || target.isContentEditable) return;
      if (event.key === 'ArrowLeft') { event.preventDefault(); navigationRef.current(-1); }
      if (event.key === 'ArrowRight') { event.preventDefault(); navigationRef.current(1); }
    };
    document.addEventListener('keydown', keydown);
    return () => {
      document.removeEventListener('keydown', keydown);
      if (copyTimer.current) clearTimeout(copyTimer.current);
    };
  }, []);

  const patch = async (changes: Partial<Pick<Asset, 'favorite' | 'tags'>>) => {
    setBusy(true); setError(null);
    try {
      const updated = await api<Asset>(`/api/assets/${asset.id}`, { method: 'PATCH', body: JSON.stringify(changes) });
      setAsset(updated);
      if (changes.tags) setTags(updated.tags);
      onUpdated(updated);
    } catch (cause) { setError(errorMessage(cause)); }
    finally { setBusy(false); }
  };
  const addTags = () => {
    const added = tagInput.split(/[,，;；\n]/).map(item => item.trim()).filter(Boolean);
    if (!added.length) return tags;
    const next = [...new Set([...tags, ...added])];
    setTags(next); setTagInput('');
    return next;
  };
  const copyValue = async (value: string, isId = false) => {
    setError(null);
    try {
      if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(value);
      } else {
        const field = document.createElement('textarea');
        field.value = value;
        field.style.position = 'fixed'; field.style.opacity = '0';
        document.body.appendChild(field); field.select();
        const successful = document.execCommand('copy');
        field.remove();
        if (!successful) throw new Error(t('components.preview.clipboardError'));
      }
      setCopied(!isId); setCopiedId(isId);
      if (copyTimer.current) clearTimeout(copyTimer.current);
      copyTimer.current = setTimeout(() => { setCopied(false); setCopiedId(false); }, 2000);
    } catch (cause) { setError(errorMessage(cause)); }
  };
  const dirty = tags.join('\0') !== asset.tags.join('\0') || tagInput.trim().length > 0;

  return <Dialog onClose={onClose} labelledBy="preview-title" className="preview-dialog">
    <div className="preview-stage">
      <div className="preview-stage-top"><span>{(index + 1).toLocaleString(i18n.resolvedLanguage)} <i>/</i> {total.toLocaleString(i18n.resolvedLanguage)}</span><span className="preview-key-hint">{t('components.preview.keyHint')}</span><button className="icon-button preview-mobile-close" onClick={onClose} aria-label={t('components.preview.close')}><X size={20} /></button></div>
      <div className="preview-image-area">
        <PreviewImage key={previewBaseUrl} url={previewBaseUrl} name={asset.name} retry={isTiff} />
        <button className="preview-nav previous" onClick={() => onNavigate(-1)} disabled={index === 0 || busy} aria-label={t('components.preview.previous')}><ChevronLeft size={22} /></button>
        <button className="preview-nav next" onClick={() => onNavigate(1)} disabled={index + 1 >= total || busy} aria-label={t('components.preview.next')}><ChevronRight size={22} /></button>
      </div>
      <div className="preview-stage-bottom"><span>{t(isTiff ? 'components.preview.thumbnailFormat' : asset.format.toUpperCase() === 'GIF' ? 'components.preview.animationFormat' : 'components.preview.originalFormat', { format: asset.format.toUpperCase() })}</span><span>{asset.width && asset.height ? `${asset.width} × ${asset.height}` : ''}</span></div>
    </div>
    <aside className="preview-details">
      <div className="preview-details-header"><span className="eyebrow">{t('components.preview.details')}</span><button className="icon-button" onClick={onClose} aria-label={t('components.preview.close')}><X size={20} /></button></div>
      <h2 id="preview-title">{asset.name}</h2>
      <p className="preview-library">{library?.name ?? t('components.common.library')}</p>
      <div className="preview-main-actions"><button className={`button ${asset.favorite ? 'favorited' : 'secondary'}`} disabled={busy} onClick={() => void patch({ favorite: !asset.favorite })}><Star size={16} fill={asset.favorite ? 'currentColor' : 'none'} />{t(asset.favorite ? 'components.preview.favorited' : 'components.preview.favorite')}</button><a className="button secondary" href={`${asset.original_url}${asset.original_url.includes('?') ? '&' : '?'}download=1`} download={asset.name}><ArrowDownToLine size={16} />{t('components.preview.download')}</a></div>
      {isTiff && <p className="quiet-note">{t('components.preview.tiffHint')}</p>}
      <div className="detail-section"><h3>{t('components.preview.fileInfo')}</h3><dl className="metadata"><div><dt>{t('components.preview.format')}</dt><dd>{asset.format.toUpperCase()}</dd></div><div><dt>{t('components.preview.dimensions')}</dt><dd>{asset.width && asset.height ? `${asset.width} × ${asset.height}` : t('components.preview.dimensionsPending')}</dd></div><div><dt>{t('components.preview.fileSize')}</dt><dd>{formatSize(asset.size)}</dd></div><div><dt>{t('components.preview.modifiedDate')}</dt><dd>{formatDate(asset.modified_at)}</dd></div></dl></div>
      <div className="detail-section"><div className="section-heading"><h3>{t('components.preview.tags')}</h3><span>{tags.length}</span></div><div className="editable-tags">{tags.map(tag => <span className="tag-chip" key={tag}>{tag}<button onClick={() => setTags(tags.filter(item => item !== tag))} disabled={busy} aria-label={t('components.preview.removeTag', { tag })}><X size={12} /></button></span>)}{!tags.length && <span className="no-tags">{t('components.preview.noTags')}</span>}</div><form className="add-tag-form" onSubmit={event => { event.preventDefault(); addTags(); }}><input aria-label={t('components.preview.newTag')} value={tagInput} onChange={event => setTagInput(event.target.value)} placeholder={t('components.preview.tagPlaceholder')} maxLength={200} disabled={busy} /><button type="submit" className="icon-button" disabled={busy || !tagInput.trim()} aria-label={t('components.preview.addTag')}><Plus size={16} /></button></form><button className="button save-tags" disabled={!dirty || busy} onClick={() => void patch({ tags: addTags() })}>{busy ? <LoaderCircle size={14} className="spin" /> : <Check size={14} />}{t('components.preview.saveTags')}</button></div>
      <div className="detail-section asset-id-section"><div className="section-heading"><h3>{t('designs.assetId')}</h3><button className="icon-button compact" onClick={() => void copyValue(String(asset.id), true)} aria-label={t('designs.copyAssetId')} title={t('designs.copyAssetId')}>{copiedId ? <Check size={15} /> : <Copy size={15} />}</button></div><code>{asset.id}</code>{copiedId && <span className="copy-feedback" role="status">{t('designs.assetIdCopied')}</span>}</div>
      <div className="detail-section"><div className="section-heading"><h3>{t('components.preview.relativePath')}</h3><button className="icon-button compact" onClick={() => void copyValue(asset.relative_path)} aria-label={t('components.preview.copyPath')} title={t('components.preview.copyPath')}>{copied ? <Check size={15} /> : <Copy size={15} />}</button></div><p className="file-path">{asset.relative_path}</p>{copied && <span className="copy-feedback" role="status">{t('components.preview.copied')}</span>}</div>
      {error && <div className="inline-error" role="alert">{error}</div>}
    </aside>
  </Dialog>;
}
