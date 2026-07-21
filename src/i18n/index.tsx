import React, { createContext, useContext, useState } from 'react'
import { zhCN } from './zh-CN'
import { enUS } from './en-US'

export type Lang = 'zh-CN' | 'en-US'
export type Translations = typeof enUS

const translations: Record<Lang, Translations> = { 'zh-CN': zhCN as Translations, 'en-US': enUS }

interface I18nContextValue {
  lang: Lang
  t: Translations
  setLang: (l: Lang) => void
}

const I18nContext = createContext<I18nContextValue>({
  lang: 'zh-CN',
  t: translations['zh-CN'],
  setLang: () => {},
})

export function I18nProvider({ children }: { children: React.ReactNode }) {
  const [lang, setLangState] = useState<Lang>(() => {
    try { return (localStorage.getItem('lang') as Lang) || 'zh-CN' }
    catch { return 'zh-CN' }
  })
  const setLang = (l: Lang) => {
    setLangState(l)
    try { localStorage.setItem('lang', l) } catch {}
  }
  return <I18nContext.Provider value={{ lang, t: translations[lang], setLang }}>{children}</I18nContext.Provider>
}

export function useI18n() {
  return useContext(I18nContext)
}

export function formatMessage(template: string, values: Record<string, string | number>) {
  return template.replace(/\{([a-zA-Z0-9_]+)\}/g, (match, key: string) =>
    Object.prototype.hasOwnProperty.call(values, key) ? String(values[key]) : match,
  )
}

export function nextLanguage(lang: Lang): Lang {
  return lang === 'zh-CN' ? 'en-US' : 'zh-CN'
}

export function selectLocalizedCopy<T>(lang: string, zhCN: T, enUS: T): T {
  return lang === 'zh-CN' ? zhCN : enUS
}
