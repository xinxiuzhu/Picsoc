import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AlertCircle, ArrowDownToLine, Check, Code2, FileJson, ImageOff, Images, LoaderCircle, Paintbrush, Plus, RefreshCw, Save, Sparkles } from 'lucide-react';
import { api, ApiError, errorMessage, formatDate } from './api';
import type { DesignFont, DesignPage, DesignScene, DesignSummary, RenderJob, StoredDesign } from './api';
import { Dialog } from './components';

const PAGE_SIZE = 50;
const MAX_LAYOUT_LENGTH = 1_000_000;
const jobActive = (job: RenderJob | null) => job?.status === 'queued' || job?.status === 'running';
const serializeScene = (scene: DesignScene) => JSON.stringify(scene, null, 2);

function blankScene(name: string): DesignScene {
  return { version: 1, name, canvas: { width: 960, height: 540, background: '#101827' }, layers: [] };
}

function readScene(value: string): DesignScene {
  const scene: unknown = JSON.parse(value);
  if (!scene || typeof scene !== 'object' || Array.isArray(scene)) throw new Error('scene-object');
  return scene as DesignScene;
}

function downloadLayout(text: string, name: string) {
  const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
  const link = document.createElement('a');
  link.href = url;
  link.download = `${name || 'design'}.json`;
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export function DesignStudio({ active, onBrowseAssets }: { active: boolean; onBrowseAssets: () => void }) {
  const { t, i18n } = useTranslation();
  const editorId = useId();
  const [designs, setDesigns] = useState<DesignSummary[]>([]);
  const [loadingList, setLoadingList] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  const [fonts, setFonts] = useState<DesignFont[]>([]);
  const [fontsError, setFontsError] = useState<string | null>(null);
  const [selected, setSelected] = useState<StoredDesign | null>(null);
  const [editor, setEditor] = useState(() => serializeScene(blankScene(t('designs.untitled'))));
  const [savedEditor, setSavedEditor] = useState(editor);
  const [job, setJob] = useState<RenderJob | null>(null);
  const [successfulJob, setSuccessfulJob] = useState<RenderJob | null>(null);
  const [busy, setBusy] = useState<'loading' | 'saving' | 'rendering' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const [pendingSelection, setPendingSelection] = useState<string | null>(null);
  const [imageFailed, setImageFailed] = useState(false);
  const [imageRetry, setImageRetry] = useState(0);
  const selectionGeneration = useRef(0);
  const listGeneration = useRef(0);
  const actionController = useRef<AbortController | null>(null);
  const dirty = editor !== savedEditor;
  const parsed = useMemo(() => {
    try { return { scene: readScene(editor), valid: true }; }
    catch { return { scene: null, valid: false }; }
  }, [editor]);
  const newerRevision = selected && designs.find(item => item.design_id === selected.design_id)?.revision !== undefined
    && (designs.find(item => item.design_id === selected.design_id)?.revision ?? 0) > selected.revision;
  const previewUrl = successfulJob?.preview_url ?? successfulJob?.output_url;
  const number = (value: number) => value.toLocaleString(i18n.resolvedLanguage?.startsWith('en') ? 'en-US' : 'zh-CN');

  const loadList = useCallback(async (offset: number, signal?: AbortSignal) => {
    const generation = ++listGeneration.current;
    setLoadingList(true); setListError(null);
    try {
      const page = await api<DesignPage>(`/api/designs?limit=${PAGE_SIZE}&offset=${offset}`, { signal });
      if (signal?.aborted || generation !== listGeneration.current) return;
      setDesigns(previous => offset ? [...previous, ...page.designs.filter(item => !previous.some(existing => existing.design_id === item.design_id))] : page.designs);
      setHasMore(page.designs.length === page.limit);
    } catch (cause) {
      if (!signal?.aborted && generation === listGeneration.current) setListError(errorMessage(cause));
    } finally {
      if (generation === listGeneration.current) setLoadingList(false);
    }
  }, []);

  useEffect(() => {
    if (!active) return;
    const controller = new AbortController();
    void loadList(0, controller.signal);
    setFontsError(null);
    void api<{ fonts: DesignFont[] }>('/api/design-fonts', { signal: controller.signal }).then(result => {
      if (!controller.signal.aborted) setFonts(result.fonts);
    }).catch(cause => {
      if (!controller.signal.aborted) setFontsError(errorMessage(cause));
    });
    return () => controller.abort();
  }, [active, loadList]);

  useEffect(() => {
    if (!active || !job || !jobActive(job)) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let stopped = false;
    const poll = async () => {
      try {
        const next = await api<RenderJob>(`/api/design-jobs/${encodeURIComponent(job.job_id)}`, { signal: controller.signal });
        if (stopped) return;
        setJob(next);
        if (next.status === 'succeeded') {
          setSuccessfulJob(next); setImageFailed(false); setStatus(null); setError(null);
          void loadList(0);
        } else if (next.status === 'failed') {
          setError(next.error || t('designs.renderFailed'));
        } else timer = setTimeout(() => { void poll(); }, 1500);
      } catch (cause) {
        if (stopped || controller.signal.aborted) return;
        setError(errorMessage(cause));
        // A lost connection can recover; auth errors already return to AuthGate.
        if (!(cause instanceof ApiError && cause.status === 401)) timer = setTimeout(() => { void poll(); }, 5000);
      }
    };
    void poll();
    return () => { stopped = true; controller.abort(); if (timer) clearTimeout(timer); };
  }, [active, job?.job_id, job?.status, loadList, t]);

  useEffect(() => {
    if (!dirty) return;
    const beforeUnload = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = ''; };
    window.addEventListener('beforeunload', beforeUnload);
    return () => window.removeEventListener('beforeunload', beforeUnload);
  }, [dirty]);

  useEffect(() => () => { actionController.current?.abort(); }, []);

  const applyDesign = (design: StoredDesign) => {
    const text = serializeScene(design.scene);
    setSelected(design); setEditor(text); setSavedEditor(text);
    setJob(design.latest_job ?? null);
    setSuccessfulJob(design.latest_job?.status === 'succeeded' ? design.latest_job : null);
    setError(null); setStatus(null); setConflict(false); setImageFailed(false); setImageRetry(0);
  };

  const openDesign = async (designId: string) => {
    if (busy) return;
    if (designId === 'new') {
      ++selectionGeneration.current;
      const text = serializeScene(blankScene(t('designs.untitled')));
      setSelected(null); setEditor(text); setSavedEditor(text); setJob(null); setSuccessfulJob(null);
      setError(null); setStatus(null); setConflict(false); setImageFailed(false); setImageRetry(0);
      return;
    }
    const generation = ++selectionGeneration.current;
    const controller = new AbortController();
    actionController.current?.abort(); actionController.current = controller;
    setBusy('loading'); setError(null);
    try {
      const design = await api<StoredDesign>(`/api/designs/${encodeURIComponent(designId)}`, { signal: controller.signal });
      if (!controller.signal.aborted && generation === selectionGeneration.current) applyDesign(design);
    } catch (cause) { if (!controller.signal.aborted) setError(errorMessage(cause)); }
    finally { if (!controller.signal.aborted) setBusy(null); }
  };

  const chooseDesign = (designId: string) => {
    if (busy) return;
    if (dirty) setPendingSelection(designId);
    else void openDesign(designId);
  };

  const save = async (signal: AbortSignal): Promise<StoredDesign> => {
    let scene: DesignScene;
    try { scene = readScene(editor); }
    catch { throw new Error(t('designs.invalidJson')); }
    const design = await api<StoredDesign>('/api/designs', {
      method: 'POST', signal,
      body: JSON.stringify({ ...(selected ? { design_id: selected.design_id, expected_revision: selected.revision } : {}), scene }),
    });
    if (!signal.aborted) {
      applyDesign(design);
      void loadList(0);
    }
    return design;
  };

  const submit = async (quality?: 'preview' | 'final') => {
    if (busy || jobActive(job) || !parsed.valid) return;
    const controller = new AbortController();
    actionController.current?.abort(); actionController.current = controller;
    setBusy(quality ? 'rendering' : 'saving'); setError(null); setStatus(null); setConflict(false);
    try {
      const design = dirty || !selected ? await save(controller.signal) : selected;
      if (quality) {
        const next = await api<RenderJob>(`/api/designs/${encodeURIComponent(design.design_id)}/render`, {
          method: 'POST', signal: controller.signal, body: JSON.stringify({ revision: design.revision, quality }),
        });
        if (!controller.signal.aborted) {
          setJob(next);
          if (next.status === 'succeeded') { setSuccessfulJob(next); setImageFailed(false); }
        }
      } else if (!controller.signal.aborted) setStatus(t('designs.saved', { revision: design.revision }));
    } catch (cause) {
      if (!controller.signal.aborted) {
        setConflict(cause instanceof ApiError && cause.status === 409);
        setError(errorMessage(cause));
      }
    } finally { if (!controller.signal.aborted) setBusy(null); }
  };

  const formatJson = () => {
    try { setEditor(serializeScene(readScene(editor))); setError(null); }
    catch { setError(t('designs.invalidJson')); }
  };

  const download = () => {
    if (!parsed.valid) { setError(t('designs.invalidJson')); return; }
    downloadLayout(serializeScene(parsed.scene!), parsed.scene?.name ?? selected?.name ?? 'design');
  };

  const jobLabel = job?.status === 'queued' ? t('designs.queued') : job?.status === 'running' ? t('designs.rendering')
    : job?.status === 'succeeded' ? t(job.quality === 'final' ? 'designs.finalReady' : 'designs.previewReady')
    : job?.status === 'failed' ? t('designs.renderFailed') : t('designs.noPreview');

  return <section className="design-studio" aria-label={t('designs.title')}>
    <div className="design-heading"><div><h1>{t('designs.title')}</h1><p>{t('designs.description')}</p></div><button className="button primary" disabled={Boolean(busy)} onClick={() => chooseDesign('new')}><Plus size={16} />{t('designs.new')}</button></div>
    <div className="design-layout">
      <aside className="design-list-panel" aria-label={t('designs.list')}>
        <div className="design-section-heading"><h2>{t('designs.list')}</h2><button className="icon-button compact" disabled={loadingList} onClick={() => { void loadList(0); }} aria-label={t('designs.refresh')} title={t('designs.refresh')}><RefreshCw size={15} className={loadingList ? 'spin' : ''} /></button></div>
        {listError && <div className="inline-error" role="alert">{listError}<button className="design-text-button" onClick={() => { void loadList(0); }}>{t('app.retry')}</button></div>}
        {loadingList && !designs.length && <div className="design-list-message" role="status"><LoaderCircle size={17} className="spin" />{t('designs.loading')}</div>}
        {!loadingList && !listError && !designs.length && <div className="design-list-empty"><Paintbrush size={25} /><p>{t('designs.emptyList')}</p><span>{t('designs.emptyListHint')}</span></div>}
        <div className="design-list">{designs.map(design => <button key={design.design_id} className={`design-list-item ${selected?.design_id === design.design_id ? 'active' : ''}`} disabled={Boolean(busy)} aria-current={selected?.design_id === design.design_id ? 'page' : undefined} onClick={() => chooseDesign(design.design_id)}>
          <span className="design-list-thumbnail">{design.latest_job?.status === 'succeeded' && design.latest_job.preview_url ? <img src={design.latest_job.preview_url} alt="" loading="lazy" /> : <FileJson size={21} />}</span>
          <span className="design-list-caption"><strong>{design.name}</strong><span>{design.width} × {design.height} · {t('designs.revision', { revision: design.revision })}</span><span>{formatDate(design.updated_at)}</span></span>
        </button>)}</div>
        {hasMore && <button className="button secondary design-load-more" disabled={loadingList} onClick={() => { void loadList(designs.length); }}>{loadingList ? <LoaderCircle size={14} className="spin" /> : <Plus size={14} />}{t('designs.loadMore')}</button>}
      </aside>
      <div className="design-editor-panel" aria-busy={Boolean(busy)}>
        <div className="design-editor-heading"><div><h2>{selected?.name ?? t('designs.untitled')}</h2><span>{selected ? t('designs.revision', { revision: selected.revision }) : t('designs.notSaved')}{dirty ? ` · ${t('designs.unsavedChanges')}` : ''}</span></div><button className="button secondary" onClick={onBrowseAssets}><Images size={15} />{t('designs.browseAssets')}</button></div>
        {(newerRevision || conflict) && selected && <div className="design-update-notice" role="status"><AlertCircle size={16} /><span>{t('designs.newerRevision')}</span><button className="design-text-button" disabled={Boolean(busy)} onClick={() => chooseDesign(selected.design_id)}>{t('designs.loadLatest')}</button></div>}
        {error && <div className="inline-error design-editor-error" role="alert">{error}</div>}
        {status && <div className="design-save-feedback" role="status"><Check size={14} />{status}</div>}
        <div className="design-preview">
          {previewUrl && !imageFailed ? <img key={`${previewUrl}-${imageRetry}`} src={`${previewUrl}${imageRetry ? `?retry=${imageRetry}` : ''}`} alt={t('designs.previewAlt', { name: selected?.name ?? t('designs.untitled') })} onError={() => setImageFailed(true)} /> : <div className="design-preview-empty">{imageFailed ? <ImageOff size={35} /> : <Paintbrush size={35} strokeWidth={1.3} />}<strong>{t(imageFailed ? 'designs.previewLoadFailed' : 'designs.previewEmpty')}</strong><p>{t(imageFailed ? 'designs.previewLoadFailedHint' : 'designs.previewEmptyHint')}</p>{imageFailed && <button className="button secondary" onClick={() => { setImageFailed(false); setImageRetry(previous => previous + 1); }}>{t('app.retry')}</button>}</div>}
          {jobActive(job) && <div className="design-render-overlay" role="status"><LoaderCircle size={24} className="spin" /><span>{jobLabel}</span></div>}
        </div>
        <div className="design-preview-caption"><span className={`design-job-status ${job?.status ?? ''}`} role="status">{job?.status === 'succeeded' ? <Check size={13} /> : jobActive(job) ? <LoaderCircle size={13} className="spin" /> : null}{jobLabel}{successfulJob ? ` · ${successfulJob.width} × ${successfulJob.height}` : ''}</span>{dirty && successfulJob && <span>{t('designs.previewOutdated')}</span>}</div>
        <div className="design-actions"><button className="button secondary" disabled={Boolean(busy) || jobActive(job) || !parsed.valid || (!dirty && Boolean(selected))} onClick={() => { void submit(); }}>{busy === 'saving' ? <LoaderCircle size={15} className="spin" /> : <Save size={15} />}{t(busy === 'saving' ? 'designs.savingLayout' : 'designs.saveLayout')}</button><button className="button secondary" disabled={Boolean(busy) || jobActive(job) || !parsed.valid} onClick={() => { void submit('preview'); }}><Sparkles size={15} />{t('designs.renderPreview')}</button><button className="button primary" disabled={Boolean(busy) || jobActive(job) || !parsed.valid} onClick={() => { void submit('final'); }}>{busy === 'rendering' ? <LoaderCircle size={15} className="spin" /> : <Paintbrush size={15} />}{t('designs.renderFinal')}</button>{successfulJob?.output_url && <a className="button secondary" href={successfulJob.output_url} download={`${selected?.name ?? 'design'}${successfulJob.quality === 'preview' ? '-preview' : ''}.png`}><ArrowDownToLine size={15} />{t(successfulJob.quality === 'final' ? 'designs.downloadPng' : 'designs.downloadPreview')}</a>}<button className="button secondary" disabled={!parsed.valid} onClick={download}><FileJson size={15} />{t('designs.downloadLayout')}</button></div>
        <div className="design-json-heading"><label htmlFor={editorId}><Code2 size={15} />{t('designs.layoutJson')}</label><button className="design-text-button" disabled={Boolean(busy) || !parsed.valid} onClick={formatJson}>{t('designs.formatJson')}</button></div>
        <textarea id={editorId} className="design-json-editor" value={editor} onChange={event => { setEditor(event.target.value); setStatus(null); }} maxLength={MAX_LAYOUT_LENGTH} spellCheck={false} autoCapitalize="off" autoCorrect="off" disabled={Boolean(busy)} aria-invalid={!parsed.valid} aria-describedby={`${editorId}-hint`} />
        {!parsed.valid && <p className="design-json-error" role="status">{t('designs.invalidJson')}</p>}
        <p className="design-editor-hint" id={`${editorId}-hint`}>{t('designs.editorHint')}</p>
        <details className="design-help"><summary>{t('designs.help')}</summary><p>{t('designs.workflowHint')}</p><pre>{`{"type":"image","asset_id":1,"x":80,"y":80,"width":320,"height":240,"fit":"contain","opacity":1}`}</pre><p>{t('designs.imageHint')}</p><h3>{t('designs.fonts')}</h3>{fontsError ? <p className="design-json-error">{fontsError}</p> : !fonts.length ? <p>{t('designs.noFonts')}</p> : <ul className="design-font-list">{fonts.map(font => <li key={font.id}><code>{font.id}</code><span>{font.name}</span>{font.supports_chinese && <span className="design-font-badge">{t('designs.chineseFont')}</span>}</li>)}</ul>}<p>{t('designs.fontHint')}</p></details>
        <p className="design-storage-note">{t('designs.storageNote')}{selected && <span>{t('designs.designId', { id: selected.design_id })}</span>}{parsed.scene?.layers && <span>{t('designs.layerCount', { count: Array.isArray(parsed.scene.layers) ? parsed.scene.layers.length : 0, formattedCount: number(Array.isArray(parsed.scene.layers) ? parsed.scene.layers.length : 0) })}</span>}</p>
      </div>
    </div>
    {pendingSelection && <Dialog onClose={() => setPendingSelection(null)} labelledBy={`${editorId}-discard-title`} className="design-discard-dialog"><h2 id={`${editorId}-discard-title`}>{t('designs.discardTitle')}</h2><p>{t('designs.discardHint')}</p><div className="dialog-actions"><button className="button secondary" onClick={() => setPendingSelection(null)}>{t('app.cancel')}</button><button className="button secondary" disabled={!parsed.valid} onClick={download}><FileJson size={15} />{t('designs.downloadDraft')}</button><button className="button primary" onClick={() => { const next = pendingSelection; setPendingSelection(null); void openDesign(next); }}>{t('designs.discardAndContinue')}</button></div></Dialog>}
  </section>;
}
