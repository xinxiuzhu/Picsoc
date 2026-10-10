import { useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Settings2, X } from 'lucide-react';

export type DisplayPreferences = { size: number; columns: number | null };
export const DEFAULT_DISPLAY: DisplayPreferences = { size: 220, columns: null };
const STORAGE_KEY = 'picsoc-display';

export function normalizeDisplayPreferences(value: unknown): DisplayPreferences {
  const saved = value && typeof value === 'object' && !Array.isArray(value) ? value as Partial<DisplayPreferences> : {};
  return {
    size: typeof saved.size === 'number' && [160, 220, 280].includes(saved.size) ? saved.size : DEFAULT_DISPLAY.size,
    columns: typeof saved.columns === 'number' && Number.isFinite(saved.columns) ? Math.max(1, Math.min(12, Math.round(saved.columns))) : null,
  };
}

export function getSavedDisplay(): DisplayPreferences {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved !== null) return normalizeDisplayPreferences(JSON.parse(saved));
    const legacy = localStorage.getItem('picsoc-density');
    return normalizeDisplayPreferences({ size: legacy !== null && legacy.trim() ? Number(legacy) : DEFAULT_DISPLAY.size });
  } catch { return { ...DEFAULT_DISPLAY }; }
}

export function saveDisplay(preferences: DisplayPreferences) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(preferences));
    localStorage.setItem('picsoc-density', String(preferences.size));
  } catch { /* Preview settings still work when browser storage is unavailable. */ }
}

export function DisplaySettings({ preferences, columns, onChange }: {
  preferences: DisplayPreferences;
  columns: number;
  onChange: (preferences: DisplayPreferences) => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const container = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const columnSlider = useRef<HTMLInputElement>(null);
  const id = useId();
  const close = () => { setOpen(false); trigger.current?.focus(); };
  const update = (changes: Partial<DisplayPreferences>) => onChange(normalizeDisplayPreferences({ ...preferences, ...changes }));

  useEffect(() => {
    if (!open) return;
    columnSlider.current?.focus();
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !container.current?.contains(event.target)) setOpen(false);
    };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [open]);

  return <div className="display-settings" ref={container} onBlur={event => {
    if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
  }}>
    <button ref={trigger} className={`icon-button display-trigger ${open ? 'active' : ''}`} aria-label={t('app.display.title')} title={t('app.display.title')} aria-haspopup="dialog" aria-expanded={open} aria-controls={`${id}-panel`} onClick={() => setOpen(previous => !previous)}><Settings2 size={17} /></button>
    {open && <div className="display-panel" id={`${id}-panel`} role="dialog" aria-labelledby={`${id}-title`} onKeyDown={event => {
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(); }
    }}>
      <div className="display-panel-heading"><h2 id={`${id}-title`}>{t('app.display.title')}</h2><button className="icon-button compact" aria-label={t('app.display.close')} onClick={close}><X size={16} /></button></div>
      <label className="display-slider-label" htmlFor={`${id}-columns`}><span>{t('app.display.columns')}</span><output>{t('app.display.columnCount', { count: preferences.columns ?? columns })}</output></label>
      <input ref={columnSlider} id={`${id}-columns`} className="display-slider" type="range" min={1} max={12} step={1} value={preferences.columns ?? Math.min(12, columns)} aria-label={t('app.display.columns')} aria-valuetext={t('app.display.columnCount', { count: preferences.columns ?? Math.min(12, columns) })} onChange={event => update({ columns: Number(event.target.value) })} />
      <div className="display-column-summary">{t('app.display.currentColumns', { count: columns })}</div>
      <p className="display-hint">{t('app.display.columnsHint')}</p>
      <button className={`display-auto ${preferences.columns === null ? 'active' : ''}`} aria-pressed={preferences.columns === null} onClick={() => update({ columns: null })}>{t('app.display.automatic')}</button>
      <p className="display-hint">{t('app.display.automaticHint')}</p>
      <div className="display-panel-footer"><span>{t('app.display.savedHint')}</span><button onClick={() => onChange({ ...DEFAULT_DISPLAY })}>{t('app.display.reset')}</button></div>
    </div>}
  </div>;
}
