import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import appZh from './locales/app.zh-CN.json';
import appEn from './locales/app.en.json';
import componentsZh from './locales/components.zh-CN.json';
import componentsEn from './locales/components.en.json';
import commonZh from './locales/common.zh-CN.json';
import commonEn from './locales/common.en.json';

export const supportedLanguages = ['zh-CN', 'en'] as const;
export type SupportedLanguage = typeof supportedLanguages[number];
const LANGUAGE_KEY = 'picsoc-language';

function savedLanguage(): SupportedLanguage {
  try {
    const saved = localStorage.getItem(LANGUAGE_KEY);
    if (supportedLanguages.some(language => language === saved)) return saved as SupportedLanguage;
  } catch { /* The interface also works when browser storage is unavailable. */ }
  return 'zh-CN';
}

void i18n.use(initReactI18next).init({
  lng: savedLanguage(),
  fallbackLng: 'zh-CN',
  supportedLngs: [...supportedLanguages],
  load: 'currentOnly',
  resources: {
    'zh-CN': { translation: { ...appZh, ...componentsZh, ...commonZh } },
    en: { translation: { ...appEn, ...componentsEn, ...commonEn } },
  },
  interpolation: { escapeValue: false },
  returnEmptyString: false,
});

function applyLanguage(language: string) {
  document.documentElement.lang = language;
  document.title = i18n.t('common.title');
  try { localStorage.setItem(LANGUAGE_KEY, language); }
  catch { /* Persisting a language preference is optional. */ }
}

i18n.on('languageChanged', applyLanguage);
applyLanguage(i18n.language);

export function currentLocale(): string {
  return i18n.language === 'en' ? 'en-US' : i18n.language || 'zh-CN';
}

export default i18n;
