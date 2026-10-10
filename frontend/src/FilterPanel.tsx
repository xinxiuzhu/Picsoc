import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { SlidersHorizontal, X } from 'lucide-react';
import { Dialog } from './components';

export interface ImageFilters {
  orientation: string;
  aspect_ratio: string;
  min_width: string;
  max_width: string;
  min_height: string;
  max_height: string;
  min_size_mb: string;
  max_size_mb: string;
}
export const EMPTY_IMAGE_FILTERS: ImageFilters = { orientation: '', aspect_ratio: '', min_width: '', max_width: '', min_height: '', max_height: '', min_size_mb: '', max_size_mb: '' };
export const ASPECT_RATIOS = ['1:1', '4:3', '3:2', '16:9', '2:1', '3:4', '2:3', '9:16'];
export const IMAGE_FORMATS = ['jpg', 'png', 'gif', 'webp', 'bmp', 'tiff'];
export const SORT_OPTIONS = ['modified', 'name', 'name_desc', 'size', 'size_asc', 'width', 'height', 'pixels'];
export const SORT_KEYS: Record<string, string> = { modified: 'app.sortModified', name: 'app.sortName', name_desc: 'app.filters.sortNameDesc', size: 'app.sortSize', size_asc: 'app.filters.sortSizeAsc', width: 'app.filters.sortWidth', height: 'app.filters.sortHeight', pixels: 'app.filters.sortPixels' };

export function parseExcludedNames(value: string): string[] {
  const seen = new Set<string>();
  return value.split(/[\r\n,，]+/).map(keyword => keyword.trim()).filter(keyword => {
    if (!keyword) return false;
    const key = keyword.replace(/[A-Z]/g, character => character.toLowerCase());
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

export function excludedNamesError(value: string): string | null {
  if (value.includes('\0')) return 'app.filters.excludedNameInvalidCharacter';
  if (new TextEncoder().encode(value).length > 4096) return 'app.filters.excludedNamesTooLong';
  const keywords = parseExcludedNames(value);
  if (keywords.length > 50) return 'app.filters.excludedNamesTooMany';
  if (keywords.some(keyword => [...keyword].length > 100)) return 'app.filters.excludedNameTooLong';
  return null;
}

export function appendImageFilters(parameters: URLSearchParams, filters: ImageFilters) {
  for (const key of ['orientation', 'aspect_ratio', 'min_width', 'max_width', 'min_height', 'max_height'] as const) {
    if (filters[key]) parameters.set(key, filters[key]);
  }
  if (filters.min_size_mb) parameters.set('min_size', String(Math.round(Number(filters.min_size_mb) * 1024 * 1024)));
  if (filters.max_size_mb) parameters.set('max_size', String(Math.round(Number(filters.max_size_mb) * 1024 * 1024)));
}

export function FilterPanel({ filters, format, tag, sort, favoriteOnly, excludedNames, tags, onApply, onClose }: {
  filters: ImageFilters;
  format: string;
  tag: string;
  sort: string;
  favoriteOnly: boolean;
  excludedNames: string;
  tags: string[];
  onApply: (filters: ImageFilters, format: string, tag: string, sort: string, favoriteOnly: boolean, excludedNames: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(filters);
  const [draftFormat, setDraftFormat] = useState(format);
  const [draftTag, setDraftTag] = useState(tag);
  const [draftSort, setDraftSort] = useState(sort);
  const [draftFavoriteOnly, setDraftFavoriteOnly] = useState(favoriteOnly);
  const [draftExcludedNames, setDraftExcludedNames] = useState(excludedNames);
  const [customRatio, setCustomRatio] = useState(Boolean(filters.aspect_ratio && !ASPECT_RATIOS.includes(filters.aspect_ratio)));
  const [error, setError] = useState('');
  const update = (key: keyof ImageFilters, value: string) => { setDraft(previous => ({ ...previous, [key]: value })); setError(''); };
  const reset = () => { setDraft({ ...EMPTY_IMAGE_FILTERS }); setDraftFormat(''); setDraftTag(''); setDraftSort('modified'); setDraftFavoriteOnly(false); setDraftExcludedNames(''); setCustomRatio(false); setError(''); };
  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    const excludedError = excludedNamesError(draftExcludedNames);
    if (excludedError) { setError(t(excludedError)); return; }
    const normalized = { ...draft };
    for (const [minimum, maximum, dimension] of [['min_width', 'max_width', 'width'], ['min_height', 'max_height', 'height'], ['min_size_mb', 'max_size_mb', 'fileSize']] as const) {
      const isSize = dimension === 'fileSize';
      for (const key of [minimum, maximum]) {
        const value = normalized[key].trim();
        normalized[key] = value;
        if (!value) continue;
        const numeric = Number(value);
        const validPattern = isSize ? /^\d+(?:\.\d+)?$/.test(value) : /^\d+$/.test(value);
        const valid = validPattern && Number.isFinite(numeric) && (isSize ? numeric >= 0 && Number.isSafeInteger(Math.round(numeric * 1024 * 1024)) : numeric > 0 && Number.isSafeInteger(numeric) && numeric <= 4294967295);
        if (!valid) { setError(t(isSize ? 'app.filters.invalidSize' : 'app.filters.invalidPixels')); return; }
        normalized[key] = String(numeric);
      }
      if (normalized[minimum] && normalized[maximum] && Number(normalized[minimum]) > Number(normalized[maximum])) {
        setError(t('app.filters.invalidRange', { field: t(`app.filters.${dimension}`) })); return;
      }
    }
    if (customRatio && !normalized.aspect_ratio.trim()) { setError(t('app.filters.invalidRatio')); return; }
    if (normalized.aspect_ratio) {
      const ratio = normalized.aspect_ratio.replace(/\s/g, '');
      const match = /^(\d+(?:\.\d+)?):(\d+(?:\.\d+)?)$/.exec(ratio);
      if (!match || !Number.isFinite(Number(match[1])) || !Number.isFinite(Number(match[2])) || !Number.isFinite(Number(match[1]) / Number(match[2]) * 1.02) || Number(match[1]) <= 0 || Number(match[2]) <= 0 || Number(match[1]) / Number(match[2]) <= 0) {
        setError(t('app.filters.invalidRatio')); return;
      }
      normalized.aspect_ratio = `${Number(match[1])}:${Number(match[2])}`;
    }
    onApply(normalized, draftFormat, draftTag, draftSort, draftFavoriteOnly, parseExcludedNames(draftExcludedNames).join('\n'));
    onClose();
  };
  const range = (label: string, minimum: keyof ImageFilters, maximum: keyof ImageFilters, unit: string, decimal = false) => <fieldset className="filter-fieldset"><legend>{label}<span>{unit}</span></legend><div className="filter-range"><label><span>{t('app.filters.minimum')}</span><input type="text" inputMode={decimal ? 'decimal' : 'numeric'} value={draft[minimum]} onChange={event => update(minimum, event.target.value)} placeholder={t('app.filters.noLimit')} aria-label={`${label} · ${t('app.filters.minimum')}`} /></label><span className="range-separator" aria-hidden="true">—</span><label><span>{t('app.filters.maximum')}</span><input type="text" inputMode={decimal ? 'decimal' : 'numeric'} value={draft[maximum]} onChange={event => update(maximum, event.target.value)} placeholder={t('app.filters.noLimit')} aria-label={`${label} · ${t('app.filters.maximum')}`} /></label></div></fieldset>;

  return <Dialog onClose={onClose} labelledBy="filter-panel-title" className="filter-dialog">
    <header className="filter-dialog-header"><span className="filter-symbol"><SlidersHorizontal size={20} /></span><div><h2 id="filter-panel-title">{t('app.filters.title')}</h2><p>{t('app.filters.description')}</p></div><button className="icon-button" onClick={onClose} aria-label={t('app.close')}><X size={19} /></button></header>
    <form onSubmit={submit}>
      <div className="filter-panel-body">
        <label className="filter-favorite"><input type="checkbox" checked={draftFavoriteOnly} onChange={event => setDraftFavoriteOnly(event.target.checked)} /><span>{t('app.filters.favoriteOnly')}</span></label>
        <label className="filter-excluded-names"><span>{t('app.filters.excludeNames')}</span><textarea value={draftExcludedNames} onChange={event => { setDraftExcludedNames(event.target.value); setError(''); }} placeholder={t('app.filters.excludeNamesPlaceholder')} rows={3} aria-describedby="excluded-names-hint" spellCheck={false} /><small id="excluded-names-hint">{t('app.filters.excludeNamesHint')}</small></label>
        <fieldset className="filter-fieldset"><legend>{t('app.filters.orientation')}</legend><div className="filter-options">{['', 'landscape', 'portrait', 'square'].map(value => <button type="button" key={value} className={draft.orientation === value ? 'active' : ''} aria-pressed={draft.orientation === value} onClick={() => update('orientation', value)}>{t(value ? `app.filters.${value}` : 'app.filters.any')}</button>)}</div></fieldset>
        <fieldset className="filter-fieldset"><legend>{t('app.filters.aspectRatio')}</legend><div className="filter-options ratio-options"><button type="button" className={!draft.aspect_ratio && !customRatio ? 'active' : ''} aria-pressed={!draft.aspect_ratio && !customRatio} onClick={() => { update('aspect_ratio', ''); setCustomRatio(false); }}>{t('app.filters.any')}</button>{ASPECT_RATIOS.map(value => <button type="button" key={value} className={draft.aspect_ratio === value && !customRatio ? 'active' : ''} aria-pressed={draft.aspect_ratio === value && !customRatio} onClick={() => { update('aspect_ratio', value); setCustomRatio(false); }}><span className="ratio-preview" style={{ aspectRatio: value.replace(':', '/') }} />{value}</button>)}<button type="button" className={customRatio ? 'active' : ''} aria-pressed={customRatio} onClick={() => { setCustomRatio(true); update('aspect_ratio', ''); }}>{t('app.filters.custom')}</button></div>{customRatio && <label className="custom-ratio"><span>{t('app.filters.customRatio')}</span><input type="text" value={draft.aspect_ratio} onChange={event => update('aspect_ratio', event.target.value)} placeholder="21:9" aria-describedby="ratio-hint" /></label>}<p id="ratio-hint" className="filter-hint">{t('app.filters.ratioHint')}</p></fieldset>
        <div className="filter-range-grid">{range(t('app.filters.width'), 'min_width', 'max_width', 'px')}{range(t('app.filters.height'), 'min_height', 'max_height', 'px')}</div>
        {range(t('app.filters.fileSize'), 'min_size_mb', 'max_size_mb', 'MB', true)}
        <div className="filter-select-grid"><label className="filter-select-field"><span>{t('app.imageFormat')}</span><select value={draftFormat} onChange={event => setDraftFormat(event.target.value)}><option value="">{t('app.allFormats')}</option>{IMAGE_FORMATS.map(value => <option key={value} value={value}>{value.toUpperCase()}</option>)}</select></label><label className="filter-select-field"><span>{t('app.tags')}</span><select value={draftTag} onChange={event => setDraftTag(event.target.value)}><option value="">{t('app.filters.allTags')}</option>{[...new Set([...tags, ...(draftTag ? [draftTag] : [])])].map(value => <option key={value} value={value}>{value}</option>)}</select></label></div>
        <label className="filter-select-field"><span>{t('app.sortBy')}</span><select value={draftSort} onChange={event => setDraftSort(event.target.value)}>{SORT_OPTIONS.map(value => <option key={value} value={value}>{t(SORT_KEYS[value])}</option>)}</select></label>
        {error && <p className="inline-error" role="alert">{error}</p>}
      </div>
      <footer className="filter-dialog-footer"><button type="button" className="filter-reset" onClick={reset}>{t('app.filters.reset')}</button><button type="button" className="button secondary" onClick={onClose}>{t('app.cancel')}</button><button type="submit" className="button primary">{t('app.filters.apply')}</button></footer>
    </form>
  </Dialog>;
}
