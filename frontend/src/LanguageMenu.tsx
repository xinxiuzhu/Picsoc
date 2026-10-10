import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Check, ChevronDown, Languages } from 'lucide-react';

export function LanguageMenu() {
  const { t, i18n } = useTranslation();
  const [open, setOpen] = useState(false);
  const container = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const items = useRef<Array<HTMLButtonElement | null>>([]);
  const language = i18n.resolvedLanguage?.startsWith('en') ? 'en' : 'zh-CN';
  const options = [{ value: 'zh-CN', label: t('app.languageChinese') }, { value: 'en', label: t('app.languageEnglish') }];
  const close = () => { setOpen(false); trigger.current?.focus(); };

  useEffect(() => {
    if (!open) return;
    items.current[language === 'en' ? 1 : 0]?.focus();
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !container.current?.contains(event.target)) setOpen(false);
    };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [open, language]);

  return <div className="language-control" ref={container} onBlur={event => {
    if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
  }}>
    <button ref={trigger} className="language-trigger" aria-label={t('app.selectLanguage')} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(previous => !previous)} onKeyDown={event => {
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); setOpen(true); }
    }}><Languages size={15} /><span>{options.find(option => option.value === language)?.label}</span><ChevronDown size={12} className={open ? 'rotate' : ''} /></button>
    {open && <div className="language-menu" role="menu" aria-label={t('app.selectLanguage')} onKeyDown={event => {
      const index = items.current.indexOf(document.activeElement as HTMLButtonElement);
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(); }
      else if (event.key === 'Tab') { setOpen(false); trigger.current?.focus(); }
      else if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
        event.preventDefault();
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + 2) % 2;
        items.current[next]?.focus();
      }
    }}>{options.map((option, index) => <button key={option.value} ref={element => { items.current[index] = element; }} type="button" role="menuitemradio" aria-checked={language === option.value} tabIndex={-1} onClick={() => { void i18n.changeLanguage(option.value); close(); }}><span>{option.label}</span>{language === option.value && <Check size={15} />}</button>)}</div>}
  </div>;
}
