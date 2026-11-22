import { useEffect, useMemo, useRef, useState } from 'react'
import { Activity, AlertTriangle, CheckCircle2, ListChecks, LoaderCircle, RotateCcw, Settings, ShieldCheck, Save, Sparkles, X } from 'lucide-react'
import { useAppStore, type InstanceConfig, defaultInstanceConfig } from '../store'
import { useI18n } from '../i18n'
import { getConfigPageLabels, getConfigTemplates, type ConfigTemplate } from '../i18n/configPageCopy'
import { validateConfig, type Warning } from '../validators'
import { BasicSection, ReasoningSection, PerformanceSection, AdvancedSection } from './ConfigPage/sections'
import { getActiveParams } from './ConfigPage/activeParams'
import { pathBasename } from '../utils/path'
import { formatHostPort } from '../utils/network'
import { detectModelWorkload, isModelWorkloadLocked, normalizeConfigForSelectedModel, normalizeInstanceConfig, type VectorCleanupChange } from '../modelPolicy'
import { normalizeModelPath } from '../store/bootstrap'
import { getEngineCompatibilityMode, normalizeEngineVersionStatus } from '../engineCapabilities'
import { canonicalConfigFields, fieldLabel, getConfigChanges, getTemplateChanges, groupTemplateChanges, isEqualValue, restoreReviewField, reviewFieldKeys, type TemplateSnapshot } from './ConfigPage/configWorkspace'
import { useEngineCompatibility } from './ConfigPage/useEngineCompatibility'
import { EngineCompatibilityNotice } from './ConfigPage/EngineCompatibilityNotice'
import { runRevisionGuarded } from './ConfigPage/configSaveGuard'
import { resolveEffectiveEngine } from '../store/engineResolution'
import { findMatchingProjector } from '../modelProjector'
import { Badge, Button, EmptyState, InsetSurface, PathText, Surface } from './ui'
import { applyExplicitOverrides, explicitOverrideKeys, inheritParameters, markExplicitOverride } from '../parameterIntent'
import { LaunchModePanel } from './ConfigPage/LaunchModePanel'
import { ConfigDirectory } from './ConfigPage/ConfigDirectory'
import { ParameterSearch } from './ConfigPage/ParameterSearch'
import { ConfigChangePanel } from './ConfigPage/ConfigChangePanel'
import { ConfigFloatingActions } from './ConfigPage/ConfigFloatingActions'
import { ModelAssetPicker, type ModelAssetPickerTarget } from './ConfigPage/ModelAssetPicker'
import { FieldRuntimeProvider } from './ConfigPage/shared'
import { canReuseConfigPreflight, configForPreflight, createConfigPreflightKey } from './ConfigPage/configPreflight'
import { beginOperationTiming, type OperationOutcome } from '../operationTiming'
import { useConfigSaveShortcut } from './ConfigPage/useConfigSaveShortcut'
import { CheckpointPanel } from './ConfigPage/CheckpointPanel'
import { applyCheckpointRequirements } from './ConfigPage/checkpointRequirements'
import { useConfigDraft } from './ConfigPage/useConfigDraft'
import { useGuideTourStore } from './guide/guideTourStore'
import { BackToInstancesButton } from './ConfigPage/BackToInstancesButton'
import { GufoConfigPage } from './ConfigPage/GufoConfigPage'

const ConfigPage = () => {
  const instance = useAppStore(state => state.instances.find(item => item.id === state.activeConfigInstanceId))
  return instance?.config.engine_kind === 'gufo' ? <GufoConfigPage key={instance.id} instance={instance} /> : <LlamaConfigPage />
}

const LlamaConfigPage = () => {
  const instances = useAppStore(state => state.instances)
  const activeConfigInstanceId = useAppStore(state => state.activeConfigInstanceId)
  const updateInstance = useAppStore(state => state.updateInstance)
  const saveConfig = useAppStore(state => state.saveConfig)
  const models = useAppStore(state => state.models)
  const modelDirs = useAppStore(state => state.modelDirs)
  const engines = useAppStore(state => state.engines)
  const defaultEngineId = useAppStore(state => state.defaultEngineId)
  const setActiveTab = useAppStore(state => state.setActiveTab)
  const generateCommand = useAppStore(state => state.generateCommand)
  const addRuntimeWarning = useAppStore(state => state.addRuntimeWarning)
  const { t, lang } = useI18n()
  const inst = instances.find(instance => instance.id === activeConfigInstanceId)
  const configInstanceId = inst?.id

  const { local, setLocal, baseline, setBaseline, committedModelPathRef, editRevisionRef, saveInFlightRef, saving, saveStage, setSaveStage } = useConfigDraft(configInstanceId)
  const [saved, setSaved] = useState(false)
  const [saveFailedFor, setSaveFailedFor] = useState<string | null>(null)
  const [showPicker, setShowPicker] = useState(false)
  const [pickerTarget, setPickerTarget] = useState<ModelAssetPickerTarget>('model')
  const [pickerCollapsed, setPickerCollapsed] = useState<Set<string>>(new Set())
  const [saveWarnings, setSaveWarnings] = useState<Warning[]>([])
  const [vectorCleanupChanges, setVectorCleanupChanges] = useState<VectorCleanupChange[]>([])
  const [searchQuery, setSearchQuery] = useState('')
  const [appliedTemplateId, setAppliedTemplateId] = useState<string | null>(null)
  const [showPresetAssistant, setShowPresetAssistant] = useState(false)
  const [selectedTemplateId, setSelectedTemplateId] = useState('safe-start')
  const [lastTemplateSnapshot, setLastTemplateSnapshot] = useState<TemplateSnapshot | null>(null)
  const mountedRef = useRef(true)
  const saveShortcutRef = useConfigSaveShortcut()
  const saveFeedbackTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
      if (saveFeedbackTimerRef.current !== null) {
        clearTimeout(saveFeedbackTimerRef.current)
      }
    }
  }, [])

  useEffect(() => {
    setAppliedTemplateId(null)
    setLastTemplateSnapshot(null)
    setShowPresetAssistant(false)
    setVectorCleanupChanges([])
  }, [activeConfigInstanceId])

  const set = <K extends keyof InstanceConfig,>(key: K, value: InstanceConfig[K]) => {
    editRevisionRef.current += 1
    setAppliedTemplateId(null)
    setLastTemplateSnapshot(null)
    setSaved(false)
    setSaveWarnings([])
    setLocal(current => (current ? markExplicitOverride(current, key, value) : current))
  }

  const inherit = (keys: Array<keyof InstanceConfig>) => {
    editRevisionRef.current += 1
    setAppliedTemplateId(null)
    setLastTemplateSnapshot(null)
    setSaved(false)
    setSaveWarnings([])
    setLocal(current => (current ? inheritParameters(current, keys) : current))
  }

  const currentModel = useMemo(() => {
    const modelPath = local?.model_path ? normalizeModelPath(local.model_path) : ''
    return modelPath
      ? models.find(model => normalizeModelPath(model.path) === modelPath) ?? null
      : null
  }, [local?.model_path, models])
  const currentProjector = useMemo(() => {
    const projectorPath = local?.mmproj_path ? normalizeModelPath(local.mmproj_path) : ''
    return projectorPath
      ? models.find(model => normalizeModelPath(model.path) === projectorPath) ?? null
      : null
  }, [local?.mmproj_path, models])

  const workload = useMemo(() => (
    local ? detectModelWorkload(currentModel, local.model_path, local) : 'inference'
  ), [currentModel, local])
  const isEmbedding = workload !== 'inference'
  const modelWorkloadLocked = useMemo(() => (
    local ? isModelWorkloadLocked(currentModel, local.model_path) : false
  ), [currentModel, local])
  const labels = useMemo(() => getConfigPageLabels(lang), [lang])
  const quickTemplates = useMemo(() => getConfigTemplates(lang), [lang])
  const currentEngine = useMemo(() => {
    return local ? resolveEffectiveEngine(local, engines, defaultEngineId) : null
  }, [defaultEngineId, engines, local])
  const compatibilityConfig = useMemo(() => local
    ? configForPreflight(local, currentModel, committedModelPathRef.current)
    : null, [committedModelPathRef, currentModel, local])
  const trustedEngineId = local?.engine_id || defaultEngineId || ''
  const { unsupportedEngineFlags, setUnsupportedEngineFlags, commandPreview, commandPreviewKey, previewingCommand, probingEngineCompatibility, capabilityProbeRequired } = useEngineCompatibility({ local: compatibilityConfig, currentEngine, trustedEngineId })
  const savedBaseline = baseline ?? defaultInstanceConfig()
  const vectorCleanupKeys = new Set(
    vectorCleanupChanges
      .filter(change => local && isEqualValue(local[change.key], change.after))
      .map(change => change.key),
  )
  const configChanges = local ? getConfigChanges(local, savedBaseline, t, labels)
    .filter(change => !vectorCleanupKeys.has(change.key)) : []
  const guideConfigPending = saving || configChanges.length > 0
  useEffect(() => {
    useGuideTourStore.setState({ configPending: guideConfigPending })
    return () => { useGuideTourStore.setState({ configPending: false }) }
  }, [guideConfigPending])

  saveShortcutRef.current = async () => {}
  if (!local) return <div className="console-page space-y-4"><BackToInstancesButton /><EmptyState icon={<Settings className="h-10 w-10" />} title={t.configPage.title} description={t.configPage.noInstance} /></div>

  const overrideKeys = explicitOverrideKeys(local)
  const reviewOverrideKeys = canonicalConfigFields(overrideKeys)
  const inheritAllKeys = [...new Set(reviewOverrideKeys.flatMap(reviewFieldKeys))]
  const generatedParams = getActiveParams(local, isEmbedding)
  const fallbackEmittedKeys = canonicalConfigFields(generatedParams)
    .filter(key => reviewOverrideKeys.includes(key))
  const emittedOverrideKeys = local.launch_mode === 'manual'
    ? [...generatedParams]
    : (commandPreview?.emittedOverrideKeys ?? fallbackEmittedKeys)
  const emittedParams = new Set<keyof InstanceConfig>(emittedOverrideKeys)
  const explicitParams = new Set<keyof InstanceConfig>(reviewOverrideKeys)
  const unsupportedFlags = new Set(unsupportedEngineFlags)
  const primaryModelPath = currentModel?.path || local.model_path || ''
  const draftModelPath = local.draft_model_path || ''
  const endpoint = formatHostPort(local.host || '127.0.0.1', local.port)

  const applyPrimaryModelPath = (modelPath: string) => {
    const normalizedPath = normalizeModelPath(modelPath)
    if (normalizedPath === committedModelPathRef.current) return

    const selectedModel = models.find(model => normalizeModelPath(model.path) === normalizedPath)
    const mmproj = selectedModel ? findMatchingProjector(selectedModel, models) : null
    const withModel = markExplicitOverride(local, 'model_path', modelPath)
    const candidate = markExplicitOverride(withModel, 'mmproj_path', mmproj?.path ?? '')
    const normalized = normalizeConfigForSelectedModel(candidate, selectedModel)
    editRevisionRef.current += 1
    committedModelPathRef.current = normalizedPath
    setAppliedTemplateId(null)
    setLastTemplateSnapshot(null)
    setSaved(false)
    setSaveWarnings([])
    setLocal(normalized.config)
    setVectorCleanupChanges(normalized.vectorMode ? normalized.changes : [])
  }

  const pickModel = (modelPath: string) => {
    if (pickerTarget === 'model') {
      applyPrimaryModelPath(modelPath)
    } else if (pickerTarget === 'draft') {
      set('draft_model_path', modelPath)
    } else {
      set('mmproj_path', modelPath)
    }
    setShowPicker(false)
  }

  const manualMode = local.launch_mode === 'manual'
  const save = async () => {
    if (!inst || saveInFlightRef.current || (!manualMode && (probingEngineCompatibility || capabilityProbeRequired))) {
      return
    }

    saveInFlightRef.current = true
    setSaved(false)
    setSaveFailedFor(null)
    setSaveStage('validating')
    const timing = beginOperationTiming('config.save')
    let outcome: OperationOutcome = 'failure'
    if (saveFeedbackTimerRef.current !== null) {
      clearTimeout(saveFeedbackTimerRef.current)
      saveFeedbackTimerRef.current = null
    }
    const targetInstanceId = inst.id
    const saveRevision = editRevisionRef.current
    const previousSave = { config: inst.config, committedModelPath: committedModelPathRef.current }
    const targetIsActive = () => mountedRef.current && useAppStore.getState().activeConfigInstanceId === targetInstanceId
    const saveIsCurrent = () => targetIsActive() && editRevisionRef.current === saveRevision
    const localSnapshot = local
    const engine = resolveEffectiveEngine(localSnapshot, engines, defaultEngineId)
    const modelPathChanged = normalizeModelPath(localSnapshot.model_path) !== committedModelPathRef.current
    const normalized = manualMode
      ? { config: localSnapshot, workload: 'inference' as const, vectorMode: false, changes: [] as VectorCleanupChange[] }
      : modelPathChanged
        ? normalizeConfigForSelectedModel(localSnapshot, currentModel)
        : normalizeInstanceConfig(localSnapshot, currentModel)
    timing.mark('normalize')

    try {
      if (engine) {
        const expectedPreflightKey = createConfigPreflightKey(normalized.config, engine.exe)
        const cachedPreflight = commandPreview && canReuseConfigPreflight(commandPreviewKey, expectedPreflightKey) ? commandPreview : null
        const preflight = cachedPreflight
          ? { stale: false as const, value: cachedPreflight }
          : await runRevisionGuarded(saveRevision, () => editRevisionRef.current, () => generateCommand(normalized.config, engine.exe))
        timing.mark(cachedPreflight ? 'preflight-reused' : 'preflight')
        if (preflight.stale || !saveIsCurrent()) {
          outcome = 'cancelled'
          return
        }
        const unsupported = preflight.value.unsupportedFlags
        setUnsupportedEngineFlags(unsupported)
        if (unsupported.length > 0) {
          outcome = 'cancelled'
          return
        }
      }

      if (!saveIsCurrent()) {
        outcome = 'cancelled'
        return
      }

      setSaveStage('persisting')
      committedModelPathRef.current = normalizeModelPath(normalized.config.model_path)
      setLocal(normalized.config)
      updateInstance(targetInstanceId, { config: normalized.config })
      await saveConfig()
      timing.mark('persist')
      const persistedConfig = useAppStore.getState().instances
        .find(item => item.id === targetInstanceId)?.config ?? normalized.config
      setBaseline(persistedConfig, normalized.config)
      if (editRevisionRef.current === saveRevision) {
        committedModelPathRef.current = normalizeModelPath(persistedConfig.model_path)
        setLocal(persistedConfig)
      }
      outcome = 'success'
      if (!targetIsActive()) return
      if (editRevisionRef.current === saveRevision) {
        setSaved(true)
        const persistedModelPath = normalizeModelPath(persistedConfig.model_path)
        const persistedModel = models.find(model => normalizeModelPath(model.path) === persistedModelPath) ?? null
        const persistedProjectorPath = normalizeModelPath(persistedConfig.mmproj_path)
        const persistedProjector = models.find(model => normalizeModelPath(model.path) === persistedProjectorPath) ?? null
        setSaveWarnings(validateConfig(persistedConfig, persistedModel, engine, persistedProjector))
        setVectorCleanupChanges([])
      } else {
        setSaved(false)
        setSaveWarnings([])
      }

      saveFeedbackTimerRef.current = setTimeout(() => {
        if (mountedRef.current) {
          setSaved(false)
          setSaveWarnings([])
        }
        saveFeedbackTimerRef.current = null
      }, 6000)
      outcome = 'success'
    } catch (error) {
      if (targetIsActive()) setSaveFailedFor(targetInstanceId)
      updateInstance(targetInstanceId, { config: previousSave.config })
      if (saveIsCurrent()) { committedModelPathRef.current = previousSave.committedModelPath; setLocal(localSnapshot) }
      if (manualMode) {
        addRuntimeWarning(`${labels.manualValidationFailed}: ${String(error)}`)
      } else if (error && typeof error === 'object' && String((error as { code?: string }).code || '').startsWith('ENGINE_')) {
        addRuntimeWarning(`配置保存前的引擎兼容性检查失败：${String(error)}`)
      }
      return
    } finally {
      saveInFlightRef.current = false
      setSaveStage(null)
      timing.finish(outcome)
    }
  }

  saveShortcutRef.current = save

  const vectorCleanupGroups: Array<{ group: VectorCleanupChange['group']; label: string }> = [
    { group: 'speculative', label: t.configPage.vectorCleanupSpeculative },
    { group: 'generation', label: t.configPage.vectorCleanupGeneration },
    { group: 'chat', label: t.configPage.vectorCleanupChat },
    { group: 'multimodal', label: t.configPage.vectorCleanupMultimodal },
    { group: 'custom', label: t.configPage.vectorCleanupCustom },
    { group: 'runtime', label: t.configPage.vectorCleanupRuntime },
  ]
  const visibleVectorCleanupGroups = vectorCleanupGroups
    .map(group => ({
      ...group,
      count: vectorCleanupChanges.filter(change => change.group === group.group).length,
    }))
    .filter(group => group.count > 0)

  const changedParams = new Set<keyof InstanceConfig>(configChanges.map(change => change.key))
  const baselineOverrideKeys = canonicalConfigFields(explicitOverrideKeys(savedBaseline))
  const statusLabels = {
    changedMarker: labels.changedMarker,
    emittedMarker: labels.changeWillEmit,
    inheritedValue: labels.valueModeInherited,
    fixedValue: labels.valueModeFixed,
  }
  const locateParameter = (key: keyof InstanceConfig) => {
    setSearchQuery(fieldLabel(key, t))
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        document.querySelectorAll<HTMLElement>('[data-config-search-current="true"]')
          .forEach(element => element.removeAttribute('data-config-search-current'))
        const target = document.querySelector<HTMLElement>(`[data-config-field="${String(key)}"]`)
        if (!target) return
        target.dataset.configSearchCurrent = 'true'
        target.scrollIntoView({ behavior: 'smooth', block: 'center' })
        target.querySelector<HTMLElement>('input, select, textarea, button')?.focus({ preventScroll: true })
      })
    })
  }
  const undoChange = (key: keyof InstanceConfig) => {
    editRevisionRef.current += 1
    setAppliedTemplateId(null)
    setLastTemplateSnapshot(null)
    setSaved(false)
    setSaveWarnings([])
    setLocal(current => current ? restoreReviewField(current, savedBaseline, key) : current)
  }
  const sectionProps = {
    local,
    set,
    inherit,
    t,
    isEmbedding,
    workload,
    modelWorkloadLocked,
    onCommitModelPath: applyPrimaryModelPath,
    onShowPicker: () => {
      setPickerTarget('model')
      setShowPicker(true)
    },
    onShowDraftPicker: () => {
      setPickerTarget('draft')
      setShowPicker(true)
    },
    onShowMmprojPicker: () => {
      setPickerTarget('mmproj')
      setShowPicker(true)
    },
    emittedParams,
    changedParams,
    statusLabels,
    searchQuery,
  }
  const liveWarnings = manualMode ? [] : validateConfig(local, currentModel, currentEngine, currentProjector)
  const engineCompatibilityMode = getEngineCompatibilityMode(currentEngine?.capabilities)
  const engineVersionStatus = normalizeEngineVersionStatus(currentEngine?.capabilities)
  const visibleWarnings = saved ? saveWarnings : liveWarnings
  const warningCounts = { high: liveWarnings.filter(warning => warning.severity === 'high').length, medium: liveWarnings.filter(warning => warning.severity === 'medium').length, low: liveWarnings.filter(warning => warning.severity === 'low').length }
  const warningTone = warningCounts.high > 0 ? 'red' : warningCounts.medium > 0 ? 'amber' : warningCounts.low > 0 ? 'sky' : 'emerald'
  const checkMessages = [
    ...(!manualMode && !primaryModelPath ? [{ tone: 'red', text: labels.missingModel }] : []),
    ...(!currentEngine ? [{ tone: 'amber', text: labels.missingEngine }] : []),
    ...(!manualMode && unsupportedEngineFlags.length > 0 ? [{ tone: 'red', text: labels.engineCompatibilityBlocked }] : []),
    ...(!manualMode && currentEngine && engineCompatibilityMode !== 'full' ? [{ tone: 'amber', text: labels.engineCompatibilityLimitedCheck }] : []),
    ...(!manualMode && currentEngine && engineVersionStatus === 'unknown' ? [{ tone: 'amber', text: labels.engineVersionUnknownCheck }] : []),
  ]
  const selectedTemplate = quickTemplates.find(template => template.id === selectedTemplateId) ?? quickTemplates[0]
  const selectedTemplateChanges = selectedTemplate ? getTemplateChanges(local, selectedTemplate.changes, t, labels) : []
  const selectedTemplateGroups = groupTemplateChanges(selectedTemplateChanges, [
    { id: 'performance', title: labels.templateGroupsPerformance, keys: ['batch_size', 'ubatch_size', 'parallel', 'cont_batching', 'threads', 'threads_batch', 'threads_http'] },
    { id: 'context', title: labels.templateGroupsContext, keys: ['ctx_size_auto', 'ctx_size', 'cache_prompt', 'cache_ram', 'cache_reuse', 'ctx_checkpoints', 'checkpoint_min_step', 'context_shift'] },
    { id: 'hardware', title: labels.templateGroupsHardware, keys: ['gpu_layers_auto', 'gpu_layers', 'flash_attn', 'load_mode', 'lazy_mode', 'no_kv_offload', 'kv_unified', 'device', 'split_mode', 'tensor_split', 'main_gpu'] },
    { id: 'observability', title: labels.templateGroupsObservability, keys: ['metrics', 'props', 'slots_enabled', 'perf', 'verbose'] },
    { id: 'speculative', title: labels.templateGroupsSpeculative, keys: ['spec_type', 'draft_model_path', 'draft_tokens', 'spec_draft_n_min', 'spec_draft_p_min', 'spec_draft_p_split', 'draft_gpu_layers', 'spec_default'] },
    { id: 'generation', title: labels.templateGroupsGeneration, keys: ['n_predict', 'temp', 'top_k', 'top_p', 'repeat_penalty', 'min_p', 'ignore_eos'] },
  ], labels.templateGroupsOther)
  const overwrittenChanges = selectedTemplateChanges.filter(change => !isEqualValue(local[change.key], savedBaseline[change.key]))

  const applyTemplate = (template: ConfigTemplate) => {
    if (!local) {
      return
    }
    editRevisionRef.current += 1
    setLastTemplateSnapshot({ templateId: template.id, templateTitle: template.title, config: { ...local } })
    setLocal(applyExplicitOverrides(local, template.changes))
    setAppliedTemplateId(template.id)
    setShowPresetAssistant(false)
    setSaved(false)
    setSaveWarnings([])
  }

  const undoTemplate = () => {
    if (!lastTemplateSnapshot) {
      return
    }
    editRevisionRef.current += 1
    setLocal({ ...lastTemplateSnapshot.config })
    setAppliedTemplateId(null)
    setLastTemplateSnapshot(null)
    setSaved(false)
    setSaveWarnings([])
  }

  const advancedDirectoryGroups = [
    ...(!isEmbedding ? [
      { id: 'config-advanced-reasoning', title: t.configPage.subAdvReasoning },
      { id: 'config-advanced-model', title: t.configPage.subAdvModelAdapt },
      { id: 'config-advanced-sampling', title: t.configPage.subAdvSampling },
      { id: 'config-advanced-sampling-ext', title: t.configPage.subAdvSamplingExt },
      { id: 'config-advanced-spec', title: t.configPage.subAdvSpec },
    ] : [
      { id: 'config-advanced-vector', title: t.configPage.subEmbedding },
    ]),
    { id: 'config-advanced-rope', title: t.configPage.subAdvRope },
    { id: 'config-advanced-kv', title: t.configPage.subAdvKvCache },
    { id: 'config-advanced-context', title: t.configPage.subAdvContextMgmt },
    { id: 'config-advanced-hardware', title: t.configPage.subAdvHardware },
    { id: 'config-advanced-server', title: t.configPage.subAdvServer },
    { id: 'config-advanced-server-ext', title: t.configPage.subAdvServerExt },
    ...(!isEmbedding ? [{ id: 'config-advanced-multi', title: t.configPage.subAdvMulti }] : []),
    { id: 'config-advanced-custom', title: t.configPage.customArgs },
  ]
  const directoryGroups = [
    { id: 'config-basic', title: t.configPage.basic },
    ...(!isEmbedding ? [{ id: 'config-reasoning', title: t.configPage.reasoning }] : []),
    { id: 'config-performance', title: t.configPage.performance },
    {
      id: 'config-advanced',
      title: t.configPage.advSectionTitle,
      children: advancedDirectoryGroups,
    },
    { id: 'config-checkpoint', title: t.checkpoint.configTitle },
  ]
  const saveDisabled = !inst || saving || (!manualMode && (probingEngineCompatibility || capabilityProbeRequired || unsupportedEngineFlags.length > 0))
  const savingLabel = saveStage === 'validating' ? t.configPage.validating : saveStage === 'persisting' ? t.configPage.persisting : t.configPage.saving

  return (
    <div className="console-page space-y-4" data-workspace="config">
      <header id="config-page-actions" className="console-heading-row">
        <div className="min-w-0">
          <h2 className="console-title break-words">{inst?.name || t.configPage.title}</h2>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-sm text-ui-muted">
            {currentEngine && <span>{currentEngine.name}</span>}
            {currentModel && <span className="max-w-full truncate" title={currentModel.path}>{pathBasename(currentModel.path)}</span>}
            {isEmbedding && <Badge>{labels.embeddingMode}</Badge>}
          </div>
        </div>
        <div className="console-action-row">
          {configChanges.length > 0 && <span role="status" className="text-xs font-medium text-ui-warn">{labels.unsavedChanges} {configChanges.length}</span>}
          {liveWarnings.length > 0 && <button type="button" className="console-review-link" data-tone={warningTone} onClick={() => document.getElementById('config-validation')?.focus()}>
            <AlertTriangle size={14} />{labels.warnings} {liveWarnings.length}
          </button>}
          <BackToInstancesButton disabled={saving} />
          {!manualMode && !isEmbedding && <Button size="sm" data-guide="config-presets" icon={<Sparkles size={14} />}
            onClick={() => setShowPresetAssistant(true)} disabled={engineCompatibilityMode !== 'full'}
            title={engineCompatibilityMode !== 'full' ? labels.templateCompatibilityDisabled : labels.quickTemplatesDesc}>
            {labels.openPresetAssistant}
          </Button>}
          <Button onClick={save} disabled={saveDisabled} variant="primary" data-guide="config-save"
            icon={saving ? <LoaderCircle size={15} className="animate-spin" /> : saved ? <CheckCircle2 size={15} /> : <Save size={15} />}>
            {saving ? savingLabel : saved ? t.configPage.saved : t.configPage.save}
          </Button>
        </div>
      </header>

      {!manualMode && <div className="config-workspace-tools">
        <ParameterSearch query={searchQuery} onQueryChange={setSearchQuery} labels={labels} />
        <ConfigDirectory title={labels.parameterGroups} groups={directoryGroups} />
      </div>}
      <div className="config-workspace-grid">
        <div className="space-y-4">
          {saved && (
            <div className="flex items-start gap-3 rounded-lg border border-ui-line bg-ui-selected px-4 py-3 text-sm text-ui-positive">
              <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0" />
              <span>
                {t.configPage.savedMsg}
                {' "'}
                {inst?.name}
                {'" '}
                {t.configPage.savedHint}
              </span>
            </div>
          )}

          {!manualMode && isEmbedding && (
            <div className="rounded-lg border border-ui-line bg-ui-info-bg px-4 py-3 text-sm text-ui-info">
              {t.configPage.embeddingBanner}
            </div>
          )}

          <LaunchModePanel
            config={local}
            engine={currentEngine}
            labels={labels}
            overrideKeys={reviewOverrideKeys}
            inheritKeys={inheritAllKeys}
            set={set}
            inherit={inherit}
          />

          {!manualMode && (
            <EngineCompatibilityNotice
              engine={currentEngine}
              unsupportedFlags={unsupportedEngineFlags}
              probing={probingEngineCompatibility}
              labels={labels}
            />
          )}

          {!manualMode && visibleVectorCleanupGroups.length > 0 && (
            <div className="flex items-start gap-3 rounded-lg border border-ui-line bg-ui-selected px-4 py-3 text-sm text-ui-positive">
              <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0" />
              <div className="min-w-0">
                <p className="font-medium">{t.configPage.vectorCleanupTitle}</p>
                <p className="mt-1 text-ui-positive">{t.configPage.vectorCleanupDescription}</p>
                <div className="mt-2 flex flex-wrap gap-2">
                  {visibleVectorCleanupGroups.map(group => (
                    <span key={group.group} className="rounded-md border border-ui-line bg-ui-surface px-2 py-1 text-xs">
                      {group.label}: {group.count} {t.configPage.vectorCleanupItems}
                    </span>
                  ))}
                </div>
              </div>
            </div>
          )}

          {!manualMode && !isEmbedding && lastTemplateSnapshot && (
            <div className="flex flex-col gap-3 rounded-lg border border-ui-line bg-ui-info-bg px-4 py-3 text-sm text-ui-info sm:flex-row sm:items-center sm:justify-between">
              <span className="inline-flex min-w-0 items-center gap-2">
                <Sparkles className="h-4 w-4 shrink-0" />
                <span className="min-w-0">
                  {labels.appliedTemplate} {lastTemplateSnapshot.templateTitle}。{labels.templateAppliedMessage}
                </span>
              </span>
              <Button onClick={undoTemplate} variant="secondary" size="sm" icon={<RotateCcw className="h-4 w-4" />} className="shrink-0">
                {labels.undoTemplate}
              </Button>
            </div>
          )}

          {!manualMode && (
            <FieldRuntimeProvider
              config={local}
              isEmbedding={isEmbedding}
              explicitKeys={explicitParams}
              emittedKeys={emittedParams}
              unsupportedFlags={unsupportedFlags}
              capabilities={currentEngine?.capabilities}
              engineVersion={currentEngine?.version}
              lang={lang}
              labels={{
                inherited: labels.sourceInherited,
                explicit: labels.sourceExplicit,
                managed: labels.sourceManaged,
                inactive: labels.sourceInactive,
                unsupported: labels.sourceUnsupported,
                unsaved: labels.sourceUnsaved,
                currentState: labels.helpCurrentState,
                engineDefault: labels.helpEngineDefault,
                commandFlags: labels.helpCommandFlags,
                restoreInheritance: labels.inheritOne,
                help: labels.parameterHelp,
              }}
              onInherit={inherit}
            >
              <BasicSection {...sectionProps} />
              {!isEmbedding && <ReasoningSection {...sectionProps} />}
              <PerformanceSection {...sectionProps} />
              <AdvancedSection {...sectionProps} />
            </FieldRuntimeProvider>
          )}
          <CheckpointPanel config={local} engineExe={currentEngine?.exe} instanceId={configInstanceId || local.id} labels={t.checkpoint}
            onCheckpointChange={next => set('kv_checkpoint', next)}
            onApplyRequirements={reasons => applyCheckpointRequirements(local, reasons, set)} />
        </div>

        <Surface as="aside" id="config-context" className="config-context min-w-0 p-4">
          <h3 className="mb-3 text-base font-semibold text-ui-ink">{labels.configContext}</h3>
          <div className="grid min-w-0 gap-4">
            <div className="space-y-2 text-sm">
              {[
                { label: labels.primaryModel, value: primaryModelPath || '--', path: !!primaryModelPath },
                { label: labels.draftModel, value: draftModelPath || '--', path: !!draftModelPath },
                { label: labels.engine, value: currentEngine?.name || '--' },
                { label: labels.enginePath, value: currentEngine?.dir || '--', path: !!currentEngine?.dir },
                { label: labels.endpoint, value: endpoint },
                { label: labels.embeddingMode, value: isEmbedding ? labels.on : labels.off },
                { label: labels.modifiedParams, value: String(configChanges.length) },
              ].map(row => (
                <div key={row.label} className="grid min-w-0 grid-cols-[96px_minmax(0,1fr)] items-start gap-3">
                  <span className="truncate text-sm text-ui-muted" title={row.label}>{row.label}</span>
                  {row.path ? (
                    <PathText value={row.value} maxLength={44} className="text-right text-ui-body" />
                  ) : (
                    <span className="min-w-0 truncate text-right text-sm text-ui-body" title={row.value}>
                      {row.value}
                    </span>
                  )}
                </div>
              ))}
            </div>

            <div className="border-t border-ui-line pt-3">
              <h4 id="config-validation" tabIndex={-1} className="text-sm font-semibold text-ui-ink">{labels.validationSummary}</h4>
              <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-xs text-ui-muted">
                {warningCounts.high > 0 && <span className="text-ui-danger">{labels.high} {warningCounts.high}</span>}
                {warningCounts.medium > 0 && <span className="text-ui-warn">{labels.medium} {warningCounts.medium}</span>}
                {warningCounts.low > 0 && <span>{labels.low} {warningCounts.low}</span>}
              </div>
              <div className="mt-4 space-y-2">
                {checkMessages.length === 0 && visibleWarnings.length === 0 ? (
                  <div className="rounded-lg border border-ui-line bg-ui-selected px-3 py-2 text-sm text-ui-positive">
                    {labels.checkPassed}
                  </div>
                ) : checkMessages.map((message, index) => (
                  <div
                    key={`${message.text}-${index}`}
                    className={`rounded-lg px-3 py-2 text-sm ${
                      message.tone === 'red' ? 'bg-ui-danger-bg text-ui-danger'
                        : message.tone === 'amber' ? 'bg-ui-warn-bg text-ui-warn'
                          : 'bg-ui-info-bg text-ui-info'
                    }`}
                  >
                    {message.text}
                  </div>
                ))}
              </div>

              {visibleWarnings.length > 0 && (
                <div className="mt-4 space-y-2">
                  {visibleWarnings.slice(0, 6).map((warning, index) => (
                    <div
                      key={`${warning.key}-${index}`}
                      className={`rounded-lg px-3 py-2 text-sm ${
                        warning.severity === 'high'
                          ? 'bg-ui-danger-bg text-ui-danger'
                          : warning.severity === 'medium'
                            ? 'bg-ui-warn-bg text-ui-warn'
                            : 'bg-ui-info-bg text-ui-info'
                      }`}
                    >
                      {t.configPage[warning.key] || warning.key}
                    </div>
                  ))}
                </div>
              )}
            </div>

            {!manualMode && (
              <ConfigChangePanel
                changes={configChanges}
                emittedKeys={[...emittedParams]}
                currentOverrideKeys={reviewOverrideKeys}
                baselineOverrideKeys={baselineOverrideKeys}
                previewing={previewingCommand}
                labels={labels}
                t={t}
                onLocate={locateParameter}
                onUndo={undoChange}
              />
            )}

            <InsetSurface className="p-4">
              <div className="flex items-start gap-3">
                <div className="rounded-lg border border-ui-line bg-ui-info-bg p-3 text-ui-info">
                  <Activity className="h-5 w-5" />
                </div>
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium text-ui-ink">{labels.performanceLink}</p>
                  <p className="mt-1 text-sm leading-5 text-ui-muted">{labels.performanceLinkDesc}</p>
                  <Button onClick={() => setActiveTab('perf')} variant="secondary" className="mt-3 w-full" icon={<Activity className="h-4 w-4" />}>
                    {labels.openPerformance}
                  </Button>
                </div>
              </div>
            </InsetSurface>
          </div>
        </Surface>
      </div>

      <ConfigFloatingActions
        topTargetId="config-page-actions"
        saveLabel={t.configPage.save}
        floatingSaveLabel={labels.floatingSave}
        savingLabel={savingLabel}
        savedLabel={t.configPage.saved}
        backToTopLabel={labels.backToTop}
        saving={saving}
        saved={saved}
        disabled={saveDisabled}
        hasChanges={configChanges.length > 0}
        error={saveFailedFor === configInstanceId ? labels.saveFailed : null}
        onSave={save}
      />

      {showPresetAssistant && !isEmbedding && selectedTemplate && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-ui-scrim p-4 backdrop-blur-sm">
          <div role="dialog" aria-modal="true" aria-label={labels.quickTemplates} className="console-surface console-popover flex max-h-[88vh] w-full max-w-6xl flex-col overflow-hidden">
            <div className="flex items-start justify-between gap-4 border-b border-ui-line bg-ui-inset px-5 py-4">
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <Sparkles className="h-5 w-5 text-ui-info" />
                  <h3 className="text-lg font-semibold text-ui-ink">{labels.presetAssistant}</h3>
                </div>
                <p className="mt-1 text-sm text-ui-muted">{labels.presetAssistantDesc}</p>
              </div>
              <Button onClick={() => setShowPresetAssistant(false)} variant="subtle" size="icon" aria-label="Close">
                <X className="h-5 w-5" />
              </Button>
            </div>

            <div className="grid min-h-0 flex-1 overflow-hidden lg:grid-cols-[300px_minmax(0,1fr)]">
              <aside className="min-h-0 overflow-y-auto border-b border-ui-line bg-ui-inset p-4 lg:border-b-0 lg:border-r">
                <div className="mb-3 rounded-lg border border-ui-line bg-ui-selected px-3 py-2 text-sm text-ui-positive">
                  {labels.presetSafeHint}
                </div>
                <div className="space-y-2">
                  {quickTemplates.map(template => {
                    const changeCount = getTemplateChanges(local, template.changes, t, labels).length
                    const isSelected = template.id === selectedTemplate.id
                    const isApplied = template.id === appliedTemplateId
                    return (
                      <button
                        key={template.id}
                        type="button"
                        aria-pressed={isSelected}
                        data-selected={isSelected}
                        onClick={() => setSelectedTemplateId(template.id)}
                        className="console-list-row w-full rounded-lg border border-ui-line p-3 text-left"
                      >
                        <span className="flex items-start justify-between gap-3">
                          <span className="min-w-0">
                            <span className="block text-sm font-semibold text-ui-ink">{template.title}</span>
                            <span className="mt-1 block text-xs leading-5 text-ui-muted">{template.subtitle}</span>
                          </span>
                          <span className={`shrink-0 rounded-full border px-2 py-0.5 text-[11px] font-medium ${template.tone}`}>
                            {isApplied ? labels.appliedTemplate : changeCount}
                          </span>
                        </span>
                      </button>
                    )
                  })}
                </div>
              </aside>

              <div className="min-h-0 overflow-y-auto p-5">
                <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <h4 className="text-xl font-semibold text-ui-ink">{selectedTemplate.title}</h4>
                      <span className={`rounded-full border px-2.5 py-1 text-xs font-medium ${selectedTemplate.tone}`}>
                        {selectedTemplate.subtitle}
                      </span>
                    </div>
                    <p className="mt-2 max-w-3xl text-sm leading-6 text-ui-muted">{selectedTemplate.description}</p>
                  </div>
                  <div className="flex shrink-0 items-center gap-2 rounded-lg border border-ui-line bg-ui-inset px-3 py-2 text-sm text-ui-muted">
                    <ListChecks className="h-4 w-4 text-ui-info" />
                    <span>{selectedTemplateChanges.length} {labels.templateChangeCount}</span>
                  </div>
                </div>

                <div className="mt-5 grid gap-4 lg:grid-cols-2">
                  <InsetSurface className="p-4">
                    <p className="text-sm font-medium text-ui-ink">{labels.templateBestFor}</p>
                    <div className="mt-3 flex flex-wrap gap-2">
                      {selectedTemplate.bestFor.map(item => (
                        <Badge key={item} tone="blue" className="px-2.5 py-1 text-xs">{item}</Badge>
                      ))}
                    </div>
                  </InsetSurface>
                  <InsetSurface className="p-4">
                    <p className="text-sm font-medium text-ui-ink">{labels.templateHighlights}</p>
                    <ul className="mt-3 space-y-2 text-sm text-ui-muted">
                      {selectedTemplate.highlights.map(item => (
                        <li key={item} className="flex gap-2">
                          <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-ui-positive" />
                          <span>{item}</span>
                        </li>
                      ))}
                    </ul>
                  </InsetSurface>
                </div>

                {(inst?.status === 'running' || overwrittenChanges.length > 0) && (
                  <div className="mt-4 space-y-2">
                    {inst?.status === 'running' && (
                      <div className="flex gap-2 rounded-lg border border-ui-line bg-ui-warn-bg px-3 py-2 text-sm text-ui-warn">
                        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                        <span>{labels.templateRunningWarning}</span>
                      </div>
                    )}
                    {overwrittenChanges.length > 0 && (
                      <div className="flex gap-2 rounded-lg border border-ui-line bg-ui-warn-bg px-3 py-2 text-sm text-ui-warn">
                        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                        <span>{labels.templateOverwriteWarning}</span>
                      </div>
                    )}
                  </div>
                )}

                <div className="mt-5 grid gap-4 xl:grid-cols-[minmax(0,1fr)_320px]">
                  <InsetSurface className="p-4">
                    <div className="mb-4">
                      <p className="text-sm font-medium text-ui-ink">{labels.templateDiff}</p>
                      <p className="mt-1 text-sm text-ui-muted">{labels.templateDiffDesc}</p>
                    </div>
                    {selectedTemplateChanges.length === 0 ? (
                      <div className="rounded-lg border border-ui-line bg-ui-inset px-3 py-3 text-sm text-ui-muted">
                        {labels.templateNoDiff}
                      </div>
                    ) : (
                      <div className="space-y-4">
                        {selectedTemplateGroups.map(group => (
                          <div key={group.id} className="rounded-lg border border-ui-line bg-ui-surface">
                            <div className="border-b border-ui-line px-3 py-2 text-sm font-medium text-ui-ink">
                              {group.title}
                            </div>
                            <div className="divide-y divide-ui-line">
                              {group.changes.map(change => (
                                <div key={change.key} className="grid min-w-0 gap-2 px-3 py-2 text-sm md:grid-cols-[140px_minmax(0,1fr)_minmax(0,1fr)]">
                                  <span className="min-w-0 truncate font-medium text-ui-body" title={change.label}>{change.label}</span>
                                  <span className="min-w-0 truncate text-ui-muted" title={change.before}>{change.before}</span>
                                  <span className="min-w-0 truncate text-ui-positive" title={change.after}>{change.after}</span>
                                </div>
                              ))}
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </InsetSurface>

                  <InsetSurface className="h-fit p-4">
                    <p className="text-sm font-medium text-ui-ink">{labels.templateRisks}</p>
                    <div className="mt-3 space-y-2">
                      {selectedTemplate.risks.map(risk => (
                        <div key={risk} className="flex gap-2 rounded-lg border border-ui-line bg-ui-inset px-3 py-2 text-sm text-ui-muted">
                          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-ui-warn" />
                          <span>{risk}</span>
                        </div>
                      ))}
                    </div>
                  </InsetSurface>
                </div>
              </div>
            </div>

            <div className="flex flex-col gap-3 border-t border-ui-line bg-ui-inset px-5 py-4 sm:flex-row sm:items-center sm:justify-between">
              <div className="text-sm text-ui-muted">
                {selectedTemplateChanges.length === 0 ? labels.templateNoDiff : `${selectedTemplateChanges.length} ${labels.templateChangeCount}`}
              </div>
              <div className="flex gap-2">
                <Button onClick={() => setShowPresetAssistant(false)} variant="secondary">
                  {labels.templateCancel}
                </Button>
                <Button onClick={() => applyTemplate(selectedTemplate)} disabled={selectedTemplateChanges.length === 0} icon={<Sparkles className="h-4 w-4" />}>
                  {labels.applyTemplate}
                </Button>
              </div>
            </div>
          </div>
        </div>
      )}

      {showPicker && (
        <ModelAssetPicker
          target={pickerTarget}
          models={models}
          modelDirs={modelDirs}
          collapsed={pickerCollapsed}
          description={`${labels.pickDesc}${labels.pickSeparator}${pickerTarget === 'model' ? labels.pickPrimary : pickerTarget === 'draft' ? labels.pickDraft : labels.pickMmproj}${labels.pickSuffix}`}
          emptyLabel={pickerTarget === 'mmproj' ? labels.noProjectors : t.modelRepo.noModels}
          onToggle={path => {
            const next = new Set(pickerCollapsed)
            if (next.has(path)) next.delete(path)
            else next.add(path)
            setPickerCollapsed(next)
          }}
          onPick={pickModel}
          onClose={() => setShowPicker(false)}
        />
      )}
    </div>
  )
}

export default ConfigPage
