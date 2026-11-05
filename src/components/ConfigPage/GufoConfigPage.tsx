import { useId, useState, type ReactNode } from 'react'
import { ChevronDown } from 'lucide-react'
import { useAppStore } from '../../store'
import type { GufoConfig, Instance, InstanceConfig } from '../../store/types'
import { useConfigDraft } from './useConfigDraft'
import { useI18n } from '../../i18n'
import { gufoCopy } from '../../i18n/gufoCopy'
import { resolveEffectiveEngine } from '../../store/engineResolution'
import { Button, SectionHeader, SelectInput, Surface, TextInput } from '../ui'

function GufoSection({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return <section aria-labelledby={id} className="grid min-w-0 gap-4 p-5 sm:p-6 lg:grid-cols-[10rem_minmax(0,1fr)] lg:gap-8">
    <h3 id={id} className="text-sm font-semibold text-slate-950 dark:text-slate-100 lg:pt-1">{title}</h3>
    <div className="min-w-0 space-y-4">{children}</div>
  </section>
}

export function GufoConfigPage({ instance }: { instance: Instance }) {
  const { lang } = useI18n()
  const copy = gufoCopy(lang)
  const id = useId()
  const draft = useConfigDraft(instance.id)
  const local = draft.local ?? instance.config
  const setLocal = draft.setLocal
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [saved, setSaved] = useState(false)
  const [command, setCommand] = useState('')
  const saving = draft.saveInFlightRef
  const engines = useAppStore(state => state.engines)
  const defaultEngineId = useAppStore(state => state.defaultEngineId)
  const engine = resolveEffectiveEngine(local, engines, defaultEngineId)
  const running = instance.status === 'running'
  const reasoningBudget = local.gufo.reasoning_budget ?? -1
  const set = <K extends keyof InstanceConfig>(key: K, value: InstanceConfig[K]) => {
    setSaved(false); setCommand(''); setError('')
    draft.editRevisionRef.current += 1
    setLocal(current => current ? ({ ...current, [key]: value, explicit_overrides: [] }) : current)
  }
  const gufo = <K extends keyof GufoConfig>(key: K, value: GufoConfig[K]) => set('gufo', { ...local.gufo, [key]: value })
  const field = (key: keyof InstanceConfig, label: string, numeric = false) => (
    <label key={key} htmlFor={`${id}-${key}`} className="flex min-w-0 flex-col gap-2 text-sm">
      <span className="font-medium leading-5">{label}</span>
      <TextInput id={`${id}-${key}`} type={numeric ? 'number' : 'text'} step={numeric ? 'any' : undefined}
        title={numeric ? undefined : String(local[key] ?? '')}
        value={String(local[key] ?? '')} onChange={event => set(key, (numeric ? Number(event.target.value) : event.target.value) as never)} />
    </label>
  )
  const gufoField = (key: keyof GufoConfig, label: string, numeric = false) => (
    <label key={key} htmlFor={`${id}-${key}`} className="flex min-w-0 flex-col gap-2 text-sm">
      <span className="font-medium leading-5">{label}</span>
      <TextInput id={`${id}-${key}`} type={numeric ? 'number' : 'text'} min={numeric ? 1 : undefined}
        title={numeric ? undefined : String(local.gufo[key])}
        value={String(local.gufo[key])} onChange={event => gufo(key, (numeric ? Number(event.target.value) : event.target.value) as never)} />
    </label>
  )
  const select = (key: keyof GufoConfig, label: string, options: Array<[string, string]>) => (
    <label htmlFor={`${id}-${key}`} className="flex min-w-0 flex-col gap-2 text-sm">
      <span className="font-medium leading-5">{label}</span>
      <SelectInput id={`${id}-${key}`} value={String(local.gufo[key])} onChange={event => {
        const value = event.target.value
        set('gufo', { ...local.gufo, [key]: value, ...(key === 'model_family' ? { speculative: 'off' } : {}) })
      }} className="w-full">
        {options.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
      </SelectInput>
    </label>
  )
  const checkbox = (key: 'preserve_thinking' | 'cache_disk', label: string) => (
    <label className="flex min-h-11 cursor-pointer items-center gap-2.5 text-sm font-medium">
      <input type="checkbox" className="h-4 w-4 shrink-0 accent-blue-600 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500"
        checked={local.gufo[key]} onChange={event => gufo(key, event.target.checked)} />{label}
    </label>
  )
  const validate = async (persist: boolean) => {
    if (saving.current || running) return
    if (!engine) { setError(copy.missing); return }
    if (!Number.isSafeInteger(reasoningBudget) || reasoningBudget < -1) { setError(copy.reasoningBudgetInvalid); return }
    saving.current = true; draft.setSaveStage('validating'); setBusy(true); setError(''); setSaved(false)
    const store = useAppStore.getState()
    const previous = store.instances.find(item => item.id === instance.id)?.config
    try {
      const preview = await store.generateCommand(local, engine.exe)
      setCommand(preview.command.map((arg, index, all) => all[index - 1] === '--api-key' ? '"***"' : JSON.stringify(arg)).join(' '))
      if (!persist) return
      const current = useAppStore.getState().instances.find(item => item.id === instance.id)
      if (!current || current.status === 'running' || current.config !== previous) throw new Error(copy.running)
      draft.setSaveStage('persisting')
      store.updateInstance(instance.id, { config: local })
      const pendingSave = store.saveConfig()
      const submitted = useAppStore.getState().instances.find(item => item.id === instance.id)?.config
      try { await pendingSave } catch (cause) {
        const latest = useAppStore.getState().instances.find(item => item.id === instance.id)?.config
        if (previous && latest === submitted) store.updateInstance(instance.id, { config: previous })
        else if (previous && latest) draft.setBaseline(latest, previous)
        throw cause
      }
      const persisted = useAppStore.getState().instances.find(item => item.id === instance.id)?.config ?? local
      draft.setBaseline(persisted, local)
      setSaved(true)
    } catch (cause) { setError(String(cause)) }
    finally { saving.current = false; draft.setSaveStage(null); setBusy(false) }
  }
  const sampling: Array<[keyof InstanceConfig, string]> = [
    ['temp', copy.temperature], ['top_k', copy.topK], ['top_p', copy.topP], ['min_p', copy.minP],
    ['seed', copy.seed], ['repeat_penalty', copy.repeatPenalty], ['repeat_last_n', copy.repeatLast],
    ['frequency_penalty', copy.frequency], ['presence_penalty', copy.presence],
  ]
  return <div className="min-w-0 space-y-5" data-testid="gufo-config-page">
    <div className="space-y-2">
      <SectionHeader title={`${copy.title} · ${instance.name}`} description={copy.description} />
      <p className="text-sm leading-6 text-slate-600 dark:text-slate-300">{copy.scope}</p>
    </div>
    {running && <p role="status" className="text-sm text-amber-700 dark:text-amber-300">{copy.running}</p>}
    <Surface>
      <fieldset disabled={busy || running} className="min-w-0 divide-y divide-slate-200 disabled:opacity-60 dark:divide-slate-800">
        <GufoSection id={`${id}-files`} title={copy.filesSection}>
          {field('model_path', copy.model)}{field('mmproj_path', copy.projector)}
        </GufoSection>
        <GufoSection id={`${id}-service`} title={copy.serviceSection}>
          <div className="grid items-end gap-4 sm:grid-cols-3">
            <div className="min-w-0 sm:col-span-2">{field('alias', copy.alias)}</div>
            {field('port', copy.port, true)}
            {field('ctx_size', copy.context, true)}{field('n_predict', copy.maxTokens, true)}
            {gufoField('startup_timeout_seconds', copy.timeout, true)}
          </div>
        </GufoSection>
        <GufoSection id={`${id}-speculation`} title={copy.speculationSection}>
          <div className={`grid items-end gap-4 ${local.gufo.speculative === 'off' ? 'sm:grid-cols-2' : 'sm:grid-cols-3'}`}>
            {select('model_family', copy.family, [['qwen38_flash_next', 'Qwen3.8-Flash-Next'], ['qwen38_27b', 'Qwen3.8-27B']])}
            {select('speculative', copy.speculative, [['off', copy.off], local.gufo.model_family === 'qwen38_27b' ? ['dflash2', 'DFlash2'] : ['mtp', 'MTP']])}
            {local.gufo.speculative !== 'off' && gufoField('draft_tokens', copy.drafts, true)}
          </div>
          {local.gufo.speculative === 'mtp' && gufoField('mtp_model', copy.mtp)}
          {local.gufo.speculative === 'dflash2' && gufoField('dflash_model', copy.dflash)}
        </GufoSection>
        <GufoSection id={`${id}-thinking`} title={copy.thinkingSection}>
          <div className="grid items-end gap-4 sm:grid-cols-3">
            {select('think', copy.think, [['auto', copy.auto], ['on', copy.on], ['off', copy.off]])}
            {select('reasoning_effort', copy.effort, ['auto', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'].map(value => [value, value]))}
            <div className="flex min-w-0 flex-col gap-2 text-sm">
              <label htmlFor={`${id}-reasoning-budget`} className="font-medium leading-5">{copy.reasoningBudget}</label>
              <TextInput id={`${id}-reasoning-budget`} type="number" min={-1} max={Number.MAX_SAFE_INTEGER} step={1}
                aria-describedby={`${id}-reasoning-budget-hint`} value={Number.isNaN(reasoningBudget) ? '' : reasoningBudget}
                onChange={event => gufo('reasoning_budget', event.target.value === '' ? Number.NaN : Number(event.target.value))} />
            </div>
          </div>
          <p id={`${id}-reasoning-budget-hint`} className="max-w-3xl text-sm leading-6 text-slate-600 dark:text-slate-300">{copy.reasoningBudgetHint}</p>
          {checkbox('preserve_thinking', copy.preserve)}
        </GufoSection>
        <GufoSection id={`${id}-cache`} title={copy.cache}>
          <div className="grid items-start gap-6 sm:grid-cols-2">
            <div className={`grid items-end gap-4 ${local.gufo.cache_ram_mode === 'limit' ? 'grid-cols-[minmax(0,1fr)_8rem]' : ''}`}>
              {select('cache_ram_mode', copy.ram, [['auto', copy.auto], ['off', copy.off], ['limit', copy.limit]])}
              {local.gufo.cache_ram_mode === 'limit' && gufoField('cache_ram_gib', copy.ramGiB, true)}
            </div>
            <div className={`grid items-end gap-4 sm:self-end ${local.gufo.cache_disk ? 'grid-cols-[minmax(0,1fr)_8rem]' : ''}`}>
              {checkbox('cache_disk', copy.disk)}
              {local.gufo.cache_disk && gufoField('cache_disk_gib', copy.diskGiB, true)}
            </div>
          </div>
          <p className="max-w-3xl text-sm leading-6 text-slate-600 dark:text-slate-300">{copy.cacheHint}</p>
        </GufoSection>
        <details className="group p-5 sm:p-6">
          <summary className="flex min-h-11 cursor-pointer list-none items-center justify-between gap-3 pr-1 text-sm font-semibold focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-blue-500">
            {copy.sampling}<ChevronDown size={18} aria-hidden="true" className="shrink-0 text-slate-500 transition-transform group-open:rotate-180" />
          </summary>
          <div className="grid items-end gap-4 pt-5 sm:grid-cols-2 lg:grid-cols-3">{sampling.map(([key, label]) => field(key, label, true))}</div>
        </details>
      </fieldset>
      <div className="space-y-3 border-t border-slate-200 p-5 dark:border-slate-800 sm:px-6">
        {error && <p role="alert" className="break-words text-sm text-red-700 dark:text-red-300">{error}</p>}
        {saved && <p role="status" className="text-sm text-emerald-700 dark:text-emerald-300">{copy.saved}</p>}
        <div className="flex flex-wrap gap-3">
          <Button variant="primary" disabled={busy || running || !engine} onClick={() => void validate(true)}>{busy ? copy.busy : copy.save}</Button>
          <Button variant="secondary" disabled={busy || running || !engine} onClick={() => void validate(false)}>{copy.preview}</Button>
          <Button variant="subtle" onClick={() => useAppStore.getState().setActiveTab('instances')}>{copy.back}</Button>
        </div>
      </div>
    </Surface>
    {command && <pre className="overflow-auto whitespace-pre-wrap break-all rounded-lg bg-slate-100 p-4 text-xs dark:bg-slate-900">{command}</pre>}
  </div>
}
