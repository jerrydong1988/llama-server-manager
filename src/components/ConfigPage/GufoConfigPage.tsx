import { useId, useState, type ReactNode } from 'react'
import { ChevronDown, Eye, FolderOpen, LoaderCircle, Save } from 'lucide-react'
import { useAppStore } from '../../store'
import type { GufoConfig, Instance, InstanceConfig } from '../../store/types'
import { useConfigDraft } from './useConfigDraft'
import { useConfigSaveShortcut } from './useConfigSaveShortcut'
import { isEqualValue } from './configWorkspace'
import { getConfigPageLabels } from '../../i18n/configPageCopy'
import { useI18n } from '../../i18n'
import { gufoCopy } from '../../i18n/gufoCopy'
import { resolveEffectiveEngine } from '../../store/engineResolution'
import { Button, SelectInput, Surface, TextInput } from '../ui'
import { ModelAssetPicker } from '../ModelAssetPicker'
import { BackToInstancesButton } from './BackToInstancesButton'

type GufoPathField = 'model_path' | 'mmproj_path' | 'mtp_model' | 'dflash_model'

function GufoSection({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return <section aria-labelledby={id} className="grid min-w-0 gap-4 p-5 sm:p-6 lg:grid-cols-[10rem_minmax(0,1fr)] lg:gap-8">
    <h3 id={id} className="text-sm font-semibold text-ui-ink lg:pt-1">{title}</h3>
    <div className="min-w-0 space-y-4">{children}</div>
  </section>
}

export function GufoConfigPage({ instance }: { instance: Instance }) {
  const { lang } = useI18n()
  const copy = gufoCopy(lang)
  const labels = getConfigPageLabels(lang)
  const saveShortcut = useConfigSaveShortcut()
  const id = useId()
  const draft = useConfigDraft(instance.id)
  const local = draft.local ?? instance.config
  const setLocal = draft.setLocal
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [saved, setSaved] = useState(false)
  const [command, setCommand] = useState('')
  const [pickerTarget, setPickerTarget] = useState<GufoPathField | null>(null)
  const [pickerCollapsed, setPickerCollapsed] = useState<Set<string>>(new Set())
  const saving = draft.saveInFlightRef
  const models = useAppStore(state => state.models)
  const modelDirs = useAppStore(state => state.modelDirs)
  const engines = useAppStore(state => state.engines)
  const defaultEngineId = useAppStore(state => state.defaultEngineId)
  const engine = resolveEffectiveEngine(local, engines, defaultEngineId)
  const draftTokensIgnored = engine?.capabilities?.status === 'detected'
    ? engine.capabilities.gufoDraftTokensIgnored ?? undefined : undefined
  const running = instance.status === 'running'
  const reasoningBudget = local.gufo.reasoning_budget ?? -1
  const set = <K extends keyof InstanceConfig>(key: K, value: InstanceConfig[K]) => {
    setSaved(false); setCommand(''); setError('')
    draft.editRevisionRef.current += 1
    setLocal(current => current ? ({ ...current, [key]: value, explicit_overrides: [] }) : current)
  }
  const gufo = <K extends keyof GufoConfig>(key: K, value: GufoConfig[K]) => set('gufo', { ...local.gufo, [key]: value })
  const pathLabels: Record<GufoPathField, string> = {
    model_path: copy.model, mmproj_path: copy.projector, mtp_model: copy.mtp, dflash_model: copy.dflash,
  }
  const setPath = (key: GufoPathField, value: string) => {
    if (key === 'model_path' || key === 'mmproj_path') set(key, value)
    else gufo(key, value)
  }
  const getPath = (key: GufoPathField) => key === 'model_path' || key === 'mmproj_path' ? local[key] : local.gufo[key]
  const pathField = (key: GufoPathField) => {
    const value = getPath(key)
    const label = pathLabels[key]
    const chooseLabel = `${copy.chooseFile}: ${label}`
    return <div className="min-w-0 space-y-2 text-sm">
      <label htmlFor={`${id}-${key}`} className="block font-medium leading-5">{label}</label>
      <div className="flex min-w-0 items-center gap-2">
        <TextInput id={`${id}-${key}`} value={value} title={value} onChange={event => setPath(key, event.target.value)} />
        <Button size="icon" className="h-11 w-11 shrink-0" title={chooseLabel} aria-label={chooseLabel}
          disabled={busy || running} onClick={() => setPickerTarget(key)}><FolderOpen className="h-4 w-4" aria-hidden="true" /></Button>
      </div>
    </div>
  }
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
      <input type="checkbox" className="h-4 w-4 shrink-0 accent-ui-positive focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ui-focus"
        checked={local.gufo[key]} onChange={event => gufo(key, event.target.checked)} />{label}
    </label>
  )
  const probeDraftPolicy = async () => {
    if (!engine || busy || running) return
    setBusy(true); setError('')
    try { await useAppStore.getState().probeEngineCapabilities(engine.id) }
    catch (cause) { setError(String(cause)) }
    finally { setBusy(false) }
  }
  const validate = async (persist: boolean) => {
    if (saving.current || running) return
    if (!engine) { setError(copy.missing); return }
    if (!Number.isSafeInteger(reasoningBudget) || reasoningBudget < -1) { setError(copy.reasoningBudgetInvalid); return }
    saving.current = true; draft.setSaveStage('validating'); setBusy(true); setError(''); setSaved(false)
    const store = useAppStore.getState()
    const previous = store.instances.find(item => item.id === instance.id)?.config
    try {
      if (draftTokensIgnored === undefined) await store.probeEngineCapabilities(engine.id)
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
  saveShortcut.current = () => validate(true)
  const changed = !isEqualValue(local, draft.baseline ?? instance.config)
  const sampling: Array<[keyof InstanceConfig, string]> = [
    ['temp', copy.temperature], ['top_k', copy.topK], ['top_p', copy.topP], ['min_p', copy.minP],
    ['seed', copy.seed], ['repeat_penalty', copy.repeatPenalty], ['repeat_last_n', copy.repeatLast],
    ['frequency_penalty', copy.frequency], ['presence_penalty', copy.presence],
  ]
  return <div className="console-page min-w-0 space-y-4" data-workspace="config" data-testid="gufo-config-page">
    <header className="console-heading-row gufo-actions">
      <div className="min-w-0">
        <h2 className="console-title break-words" aria-label={`${copy.title} · ${instance.name}`}>{instance.name}</h2>
        <p className="mt-1 text-sm text-ui-muted">{copy.title}{!engine && ` · ${copy.missing}`}</p>
      </div>
      <div className="console-action-row">
        {changed && <span role="status" className="text-xs text-ui-warn">{labels.unsavedChanges}</span>}
        <BackToInstancesButton disabled={busy} />
        <Button size="toolbar" icon={<Eye size={16}/>} disabled={busy || running || !engine} onClick={() => void validate(false)}>{copy.preview}</Button>
        <Button size="toolbar" variant="primary" icon={busy ? <LoaderCircle size={16} className="animate-spin"/> : <Save size={16}/>}
          disabled={busy || running || !engine} onClick={() => void validate(true)}>{busy ? copy.busy : copy.save}</Button>
      </div>
    </header>
    {error && <p role="alert" className="break-words text-sm text-ui-danger">{error}</p>}
    {saved && <p role="status" className="text-sm text-ui-positive">{copy.saved}</p>}
    {command && <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-lg bg-ui-inset p-4 text-xs text-ui-ink">{command}</pre>}
    {running && <p role="status" className="text-sm text-ui-warn">{copy.running}</p>}
    <Surface>
      <fieldset disabled={busy || running} className="min-w-0 divide-y divide-ui-line disabled:opacity-60">
        <GufoSection id={`${id}-files`} title={copy.filesSection}>
          {pathField('model_path')}{pathField('mmproj_path')}
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
            {local.gufo.speculative !== 'off' && (draftTokensIgnored === false
              ? gufoField('draft_tokens', copy.drafts, true)
              : <div className="flex min-w-0 flex-col gap-2 text-sm">
                <label htmlFor={`${id}-draft-policy`} className="font-medium leading-5">{copy.drafts}</label>
                <TextInput id={`${id}-draft-policy`} readOnly aria-describedby={`${id}-draft-policy-hint`}
                  value={draftTokensIgnored ? copy.modelOwnedDrafts : copy.draftPolicyUnknown} />
              </div>)}
          </div>
          {local.gufo.speculative !== 'off' && draftTokensIgnored !== false && <div className="space-y-2">
            <p id={`${id}-draft-policy-hint`} className="max-w-3xl text-sm leading-6 text-ui-muted">
              {draftTokensIgnored ? copy.modelOwnedDraftsHint : copy.draftPolicyUnknownHint}
            </p>
            {draftTokensIgnored === undefined && <Button variant="secondary" disabled={busy || running || !engine}
              onClick={() => void probeDraftPolicy()}>{busy ? copy.busy : copy.probeEngine}</Button>}
          </div>}
          {local.gufo.speculative === 'mtp' && pathField('mtp_model')}
          {local.gufo.speculative === 'dflash2' && pathField('dflash_model')}
        </GufoSection>
        <GufoSection id={`${id}-thinking`} title={copy.thinkingSection}>
          <div className="grid items-end gap-4 sm:grid-cols-3">
            {select('think', copy.think, [['auto', copy.auto], ['on', copy.on], ['off', copy.off]])}
            {select('reasoning_effort', copy.effort, ['auto', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'].map(value => [value, value]))}
            <div className="flex min-w-0 flex-col gap-2 text-sm">
              <label htmlFor={`${id}-reasoning-budget`} className="font-medium leading-5">{copy.reasoningBudget}</label>
              <TextInput id={`${id}-reasoning-budget`} type="number" min={-1} max={Number.MAX_SAFE_INTEGER} step={1}
                aria-invalid={!Number.isSafeInteger(reasoningBudget) || reasoningBudget < -1}
                aria-describedby={`${id}-reasoning-budget-hint`} value={Number.isNaN(reasoningBudget) ? '' : reasoningBudget}
                onChange={event => gufo('reasoning_budget', event.target.value === '' ? Number.NaN : Number(event.target.value))} />
            </div>
          </div>
          <p id={`${id}-reasoning-budget-hint`} className="max-w-3xl text-sm leading-6 text-ui-muted">{copy.reasoningBudgetHint}</p>
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
          <p className="max-w-3xl text-sm leading-6 text-ui-muted">{copy.cacheHint}</p>
        </GufoSection>
        <details className="group p-5 sm:p-6">
          <summary className="flex min-h-11 cursor-pointer list-none items-center justify-between gap-3 pr-1 text-sm font-semibold focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-ui-focus">
            {copy.sampling}<ChevronDown size={18} aria-hidden="true" className="shrink-0 text-ui-muted transition-transform group-open:rotate-180" />
          </summary>
          <div className="grid items-end gap-4 pt-5 sm:grid-cols-2 lg:grid-cols-3">{sampling.map(([key, label]) => field(key, label, true))}</div>
        </details>
      </fieldset>
      <p className="border-t border-ui-line px-5 py-3 text-xs leading-5 text-ui-muted">{copy.scope}</p>
    </Surface>
    {pickerTarget && <ModelAssetPicker
      target={pickerTarget === 'model_path' ? 'model' : pickerTarget === 'mmproj_path' ? 'mmproj' : 'draft'}
      models={models.filter(model => !model.is_shard && /\.gguf$/i.test(model.path))}
      modelDirs={modelDirs} collapsed={pickerCollapsed} onCollapsedChange={setPickerCollapsed}
      selectedPath={getPath(pickerTarget)}
      description={pathLabels[pickerTarget]} emptyLabel={copy.noFiles}
      onPick={model => {
        setPickerTarget(null)
        const current = useAppStore.getState().instances.find(item => item.id === instance.id)
        if (saving.current) return
        if (current?.status === 'running' || !current) { setError(copy.running); return }
        setPath(pickerTarget, model.path)
      }}
      onClose={() => setPickerTarget(null)}
    />}
  </div>
}
