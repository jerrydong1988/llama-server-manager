import { useId, useState } from 'react'
import { useAppStore } from '../../store'
import type { GufoConfig, Instance, InstanceConfig } from '../../store/types'
import { useConfigDraft } from './useConfigDraft'
import { useI18n } from '../../i18n'
import { gufoCopy } from '../../i18n/gufoCopy'
import { resolveEffectiveEngine } from '../../store/engineResolution'
import { Button, SectionHeader, Surface, TextInput } from '../ui'

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
    <label key={key} htmlFor={`${id}-${key}`} className="block space-y-1 text-sm">
      <span>{label}</span>
      <TextInput id={`${id}-${key}`} type={numeric ? 'number' : 'text'} step={numeric ? 'any' : undefined}
        value={String(local[key] ?? '')} onChange={event => set(key, (numeric ? Number(event.target.value) : event.target.value) as never)} />
    </label>
  )
  const gufoField = (key: keyof GufoConfig, label: string, numeric = false) => (
    <label key={key} htmlFor={`${id}-${key}`} className="block space-y-1 text-sm">
      <span>{label}</span>
      <TextInput id={`${id}-${key}`} type={numeric ? 'number' : 'text'} min={numeric ? 1 : undefined}
        value={String(local.gufo[key])} onChange={event => gufo(key, (numeric ? Number(event.target.value) : event.target.value) as never)} />
    </label>
  )
  const select = (key: keyof GufoConfig, label: string, options: Array<[string, string]>) => (
    <label htmlFor={`${id}-${key}`} className="block space-y-1 text-sm">
      <span>{label}</span>
      <select id={`${id}-${key}`} value={String(local.gufo[key])} onChange={event => {
        const value = event.target.value
        set('gufo', { ...local.gufo, [key]: value, ...(key === 'model_family' ? { speculative: 'off' } : {}) })
      }} className="w-full rounded-lg border border-slate-300 bg-white px-3 py-2 text-slate-900 focus-visible:outline focus-visible:outline-2 focus-visible:outline-blue-500 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-100">
        {options.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
      </select>
    </label>
  )
  const checkbox = (key: 'preserve_thinking' | 'cache_disk', label: string) => (
    <label className="flex min-h-10 items-center gap-2 text-sm">
      <input type="checkbox" checked={local.gufo[key]} onChange={event => gufo(key, event.target.checked)} />{label}
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
  return <div className="space-y-5">
    <SectionHeader title={`${copy.title} · ${instance.name}`} description={copy.description} />
    <p className="text-sm text-slate-600 dark:text-slate-300">{copy.scope}</p>
    {running && <p role="status" className="text-sm text-amber-700 dark:text-amber-300">{copy.running}</p>}
    <Surface className="p-5">
      <fieldset disabled={busy || running} className="space-y-5 disabled:opacity-60">
        <div className="grid gap-4 md:grid-cols-2">
          {field('model_path', copy.model)}{field('mmproj_path', copy.projector)}
          {field('alias', copy.alias)}{field('port', copy.port, true)}
          {field('ctx_size', copy.context, true)}{field('n_predict', copy.maxTokens, true)}
          {select('model_family', copy.family, [['qwen38_flash_next', 'Qwen3.8-Flash-Next'], ['qwen38_27b', 'Qwen3.8-27B']])}
          {select('speculative', copy.speculative, [['off', copy.off], local.gufo.model_family === 'qwen38_27b' ? ['dflash2', 'DFlash2'] : ['mtp', 'MTP']])}
          {local.gufo.speculative === 'mtp' && gufoField('mtp_model', copy.mtp)}
          {local.gufo.speculative === 'dflash2' && gufoField('dflash_model', copy.dflash)}
          {local.gufo.speculative !== 'off' && gufoField('draft_tokens', copy.drafts, true)}
          {select('think', copy.think, [['auto', copy.auto], ['on', copy.on], ['off', copy.off]])}
          {select('reasoning_effort', copy.effort, ['auto', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'].map(value => [value, value]))}
          <div className="space-y-1 text-sm">
            <label htmlFor={`${id}-reasoning-budget`}>{copy.reasoningBudget}</label>
            <TextInput id={`${id}-reasoning-budget`} type="number" min={-1} max={Number.MAX_SAFE_INTEGER} step={1}
              aria-describedby={`${id}-reasoning-budget-hint`} value={Number.isNaN(reasoningBudget) ? '' : reasoningBudget}
              onChange={event => gufo('reasoning_budget', event.target.value === '' ? Number.NaN : Number(event.target.value))} />
            <p id={`${id}-reasoning-budget-hint`} className="text-sm text-slate-600 dark:text-slate-300">{copy.reasoningBudgetHint}</p>
          </div>
          {checkbox('preserve_thinking', copy.preserve)}
          {gufoField('startup_timeout_seconds', copy.timeout, true)}
        </div>
        <div className="space-y-3">
          <h3 className="font-semibold">{copy.cache}</h3>
          <p className="text-sm text-slate-600 dark:text-slate-300">{copy.cacheHint}</p>
          <div className="grid gap-4 md:grid-cols-2">
            {select('cache_ram_mode', copy.ram, [['auto', copy.auto], ['off', copy.off], ['limit', copy.limit]])}
            {local.gufo.cache_ram_mode === 'limit' && gufoField('cache_ram_gib', copy.ramGiB, true)}
            {checkbox('cache_disk', copy.disk)}
            {local.gufo.cache_disk && gufoField('cache_disk_gib', copy.diskGiB, true)}
          </div>
        </div>
        <details>
          <summary className="cursor-pointer py-2 font-semibold">{copy.sampling}</summary>
          <div className="grid gap-4 pt-3 md:grid-cols-3">{sampling.map(([key, label]) => field(key, label, true))}</div>
        </details>
      </fieldset>
    </Surface>
    {error && <p role="alert" className="break-words text-sm text-red-700 dark:text-red-300">{error}</p>}
    {saved && <p role="status" className="text-sm text-emerald-700 dark:text-emerald-300">{copy.saved}</p>}
    <div className="flex flex-wrap gap-3">
      <Button disabled={busy || running || !engine} onClick={() => void validate(true)}>{busy ? copy.busy : copy.save}</Button>
      <Button variant="secondary" disabled={busy || running || !engine} onClick={() => void validate(false)}>{copy.preview}</Button>
      <Button variant="subtle" onClick={() => useAppStore.getState().setActiveTab('instances')}>{copy.back}</Button>
    </div>
    {command && <pre className="overflow-auto whitespace-pre-wrap break-all rounded-lg bg-slate-100 p-4 text-xs dark:bg-slate-900">{command}</pre>}
  </div>
}
