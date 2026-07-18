import { useEffect, useMemo, useRef, useState } from 'react'
import { Activity, AlertTriangle, CheckCircle2, ChevronDown, ChevronRight, Cpu, File, FolderOpen, Image, ListChecks, LoaderCircle, RotateCcw, Search, Settings, ShieldCheck, SlidersHorizontal, Sparkles, X } from 'lucide-react'
import { useAppStore, type InstanceConfig, defaultInstanceConfig } from '../store'
import { useI18n } from '../i18n'
import { getConfigPageLabels, getConfigTemplates, type ConfigTemplate } from '../i18n/configPageCopy'
import { validateConfig, type Warning } from '../validators'
import {
  BasicSection, ReasoningSection, PerformanceSection, AdvancedSection,
  BASIC_CONFIG_KEYS, REASONING_CONFIG_KEYS, PERFORMANCE_CONFIG_KEYS,
  ADVANCED_CONFIG_KEYS, ADVANCED_GROUP_CONFIG_KEYS,
} from './ConfigPage/sections'
import { getActiveParams } from './ConfigPage/activeParams'
import { pathBasename } from '../utils/path'
import { formatHostPort } from '../utils/network'
import { detectModelWorkload, isModelWorkloadLocked, normalizeConfigForSelectedModel, normalizeInstanceConfig, type VectorCleanupChange } from '../modelPolicy'
import { normalizeModelPath } from '../store/bootstrap'
import { getEngineCompatibilityMode, normalizeEngineVersionStatus } from '../engineCapabilities'
import { _matchedElements } from './ConfigPage/shared'
import { buildPickerTree, countActive, getConfigChanges, getTemplateChanges, groupTemplateChanges, isEqualValue, type PickerNode, type TemplateSnapshot } from './ConfigPage/configWorkspace'
import { useEngineCompatibility } from './ConfigPage/useEngineCompatibility'
import { EngineCompatibilityNotice } from './ConfigPage/EngineCompatibilityNotice'
import { runRevisionGuarded } from './ConfigPage/configSaveGuard'
import { resolveEffectiveEngine } from '../store/engineResolution'
import { findMatchingProjector } from '../modelProjector'
import { Badge, Button, EmptyState, InsetSurface, MetricCard, PathText, SectionHeader, Surface, TextInput } from './ui'

const ConfigPage = () => {
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

  const [local, setLocal] = useState<InstanceConfig | null>(null)
  const [baseline, setBaseline] = useState<InstanceConfig | null>(null)
  const [saved, setSaved] = useState(false)
  const [saving, setSaving] = useState(false)
  const [showPicker, setShowPicker] = useState(false)
  const [pickerTarget, setPickerTarget] = useState<'model' | 'draft'>('model')
  const [pickerCollapsed, setPickerCollapsed] = useState<Set<string>>(new Set())
  const [saveWarnings, setSaveWarnings] = useState<Warning[]>([])
  const [vectorCleanupChanges, setVectorCleanupChanges] = useState<VectorCleanupChange[]>([])
  const [searchQuery, setSearchQuery] = useState('')
  const [appliedTemplateId, setAppliedTemplateId] = useState<string | null>(null)
  const [showPresetAssistant, setShowPresetAssistant] = useState(false)
  const [selectedTemplateId, setSelectedTemplateId] = useState('safe-start')
  const [lastTemplateSnapshot, setLastTemplateSnapshot] = useState<TemplateSnapshot | null>(null)
  const mountedRef = useRef(true)
  const prevQuery = useRef('')
  const committedModelPathRef = useRef('')
  const editRevisionRef = useRef(0)
  const saveInFlightRef = useRef(false)
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
    if (searchQuery !== prevQuery.current) {
      _matchedElements.clear()
      prevQuery.current = searchQuery
    }
  }, [searchQuery])

  useEffect(() => {
    if (!searchQuery) {
      return
    }

    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        const firstMatch = [..._matchedElements][0]
        firstMatch?.scrollIntoView({ behavior: 'smooth', block: 'center' })
      })
    })
  }, [searchQuery])

  useEffect(() => {
    const selected = useAppStore.getState().instances.find(instance => instance.id === configInstanceId)
    if (selected) {
      const next = { ...defaultInstanceConfig(), ...selected.config }
      setLocal(next)
      setBaseline(next)
      editRevisionRef.current = 0
      committedModelPathRef.current = normalizeModelPath(next.model_path)
    } else {
      setLocal(null)
      setBaseline(null)
      editRevisionRef.current = 0
      committedModelPathRef.current = ''
    }
  }, [activeConfigInstanceId, configInstanceId])

  useEffect(() => {
    setAppliedTemplateId(null)
    setLastTemplateSnapshot(null)
    setShowPresetAssistant(false)
    setVectorCleanupChanges([])
  }, [activeConfigInstanceId])

  const set = (key: keyof InstanceConfig, value: any) => {
    editRevisionRef.current += 1
    setAppliedTemplateId(null)
    setLastTemplateSnapshot(null)
    setLocal(current => (current ? { ...current, [key]: value } : current))
  }

  const currentModel = useMemo(() => {
    const modelPath = local?.model_path ? normalizeModelPath(local.model_path) : ''
    return modelPath
      ? models.find(model => normalizeModelPath(model.path) === modelPath) ?? null
      : null
  }, [local?.model_path, models])

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
  const trustedEngineId = local?.engine_id || defaultEngineId || ''
  const { unsupportedEngineFlags, setUnsupportedEngineFlags, probingEngineCompatibility, capabilityProbeRequired } = useEngineCompatibility({ local, currentEngine, trustedEngineId })

  if (!local) {
    return (
      <div className="space-y-5">
        <EmptyState icon={<Settings className="h-10 w-10" />} title={t.configPage.title} description={t.configPage.noInstance} />
      </div>
    )
  }

  const activeParams = getActiveParams(local, isEmbedding)
  const primaryModelPath = currentModel?.path || local.model_path || ''
  const draftModelPath = local.draft_model_path || ''
  const endpoint = formatHostPort(local.host || '127.0.0.1', local.port)

  const applyPrimaryModelPath = (modelPath: string) => {
    const normalizedPath = normalizeModelPath(modelPath)
    if (normalizedPath === committedModelPathRef.current) return

    const selectedModel = models.find(model => normalizeModelPath(model.path) === normalizedPath)
    const mmproj = selectedModel ? findMatchingProjector(selectedModel, models) : null
    const candidate = { ...local, model_path: modelPath, mmproj_path: mmproj?.path ?? '' }
    const normalized = normalizeConfigForSelectedModel(candidate, selectedModel)
    editRevisionRef.current += 1
    committedModelPathRef.current = normalizedPath
    setAppliedTemplateId(null)
    setLastTemplateSnapshot(null)
    setLocal(normalized.config)
    setVectorCleanupChanges(normalized.vectorMode ? normalized.changes : [])
  }

  const pickModel = (modelPath: string) => {
    if (pickerTarget === 'model') {
      applyPrimaryModelPath(modelPath)
    } else {
      set('draft_model_path', modelPath)
    }
    setShowPicker(false)
  }

  const save = async () => {
    if (!inst || saveInFlightRef.current || probingEngineCompatibility || capabilityProbeRequired) {
      return
    }

    saveInFlightRef.current = true
    setSaving(true); setSaved(false)
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
    const normalized = modelPathChanged ? normalizeConfigForSelectedModel(localSnapshot, currentModel) : normalizeInstanceConfig(localSnapshot, currentModel)

    try {
      if (engine) {
        const preflight = await runRevisionGuarded(saveRevision, () => editRevisionRef.current, () => generateCommand(normalized.config, engine.exe))
        if (preflight.stale || !saveIsCurrent()) return
        const unsupported = preflight.value.unsupportedFlags
        setUnsupportedEngineFlags(unsupported)
        if (unsupported.length > 0) {
          return
        }
      }

      if (!saveIsCurrent()) return

      committedModelPathRef.current = normalizeModelPath(normalized.config.model_path)
      setLocal(normalized.config)
      updateInstance(targetInstanceId, { config: normalized.config })
      await saveConfig()
      if (!targetIsActive()) return
      const persistedConfig = useAppStore.getState().instances
        .find(item => item.id === targetInstanceId)?.config ?? normalized.config
      setBaseline(persistedConfig)
      if (editRevisionRef.current === saveRevision) {
        committedModelPathRef.current = normalizeModelPath(persistedConfig.model_path)
        setLocal(persistedConfig)
        setSaved(true)
        setSaveWarnings(validateConfig(persistedConfig, currentModel, engine))
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
    } catch (error) {
      updateInstance(targetInstanceId, { config: previousSave.config })
      if (saveIsCurrent()) { committedModelPathRef.current = previousSave.committedModelPath; setLocal(localSnapshot) }
      if (error && typeof error === 'object' && String((error as { code?: string }).code || '').startsWith('ENGINE_')) {
        addRuntimeWarning(`配置保存前的引擎兼容性检查失败：${String(error)}`)
      }
      return
    } finally {
      saveInFlightRef.current = false
      if (mountedRef.current) setSaving(false)
    }
  }

  const sectionProps = {
    local,
    set,
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
    activeParams,
    searchQuery,
  }

  const pickerTrees = modelDirs.map(dir => buildPickerTree(dir, models))

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

  const savedBaseline = baseline ?? defaultInstanceConfig()
  const vectorCleanupKeys = new Set(
    vectorCleanupChanges
      .filter(change => isEqualValue(local[change.key], change.after))
      .map(change => change.key),
  )
  const configChanges = getConfigChanges(local, savedBaseline, t, labels)
    .filter(change => !vectorCleanupKeys.has(change.key))
  const liveWarnings = validateConfig(local, currentModel, currentEngine)
  const engineCompatibilityMode = getEngineCompatibilityMode(currentEngine?.capabilities)
  const engineVersionStatus = normalizeEngineVersionStatus(currentEngine?.capabilities)
  const visibleWarnings = saved ? saveWarnings : liveWarnings
  const warningCounts = {
    high: liveWarnings.filter(warning => warning.severity === 'high').length,
    medium: liveWarnings.filter(warning => warning.severity === 'medium').length,
    low: liveWarnings.filter(warning => warning.severity === 'low').length,
  }
  const checkMessages = [
    ...(!primaryModelPath ? [{ tone: 'red', text: labels.missingModel }] : []),
    ...(!currentEngine ? [{ tone: 'amber', text: labels.missingEngine }] : []),
    ...(unsupportedEngineFlags.length > 0 ? [{ tone: 'red', text: labels.engineCompatibilityBlocked }] : []),
    ...(currentEngine && engineCompatibilityMode !== 'full' ? [{ tone: 'amber', text: labels.engineCompatibilityLimitedCheck }] : []),
    ...(currentEngine && engineVersionStatus === 'unknown' ? [{ tone: 'amber', text: labels.engineVersionUnknownCheck }] : []),
    ...(liveWarnings.length > 0 ? [{ tone: liveWarnings.some(warning => warning.severity === 'high') ? 'red' : 'amber', text: `${liveWarnings.length} ${labels.liveWarnings}` }] : []),
  ]
  const selectedTemplate = quickTemplates.find(template => template.id === selectedTemplateId) ?? quickTemplates[0]
  const selectedTemplateChanges = selectedTemplate ? getTemplateChanges(local, selectedTemplate.changes, t, labels) : []
  const selectedTemplateGroups = groupTemplateChanges(selectedTemplateChanges, [
    { id: 'performance', title: labels.templateGroupsPerformance, keys: ['batch_size', 'ubatch_size', 'parallel', 'cont_batching', 'threads', 'threads_batch', 'threads_http'] },
    { id: 'context', title: labels.templateGroupsContext, keys: ['ctx_size_auto', 'ctx_size', 'cache_prompt', 'cache_ram', 'cache_reuse', 'ctx_checkpoints', 'checkpoint_min_step', 'context_shift'] },
    { id: 'hardware', title: labels.templateGroupsHardware, keys: ['gpu_layers_auto', 'gpu_layers', 'flash_attn', 'mlock', 'no_mmap', 'no_kv_offload', 'kv_unified', 'device', 'split_mode', 'tensor_split', 'main_gpu'] },
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
    setLocal({ ...local, ...template.changes })
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
      { id: 'config-advanced-reasoning', title: t.configPage.subAdvReasoning, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedReasoningConfig) },
      { id: 'config-advanced-model', title: t.configPage.subAdvModelAdapt, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedModelAdapt) },
      { id: 'config-advanced-sampling', title: t.configPage.subAdvSampling, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedSampling) },
      { id: 'config-advanced-sampling-ext', title: t.configPage.subAdvSamplingExt, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedSamplingExt) },
      { id: 'config-advanced-spec', title: t.configPage.subAdvSpec, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedSpec) },
    ] : [
      { id: 'config-advanced-vector', title: t.configPage.subEmbedding, count: countActive(activeParams, ['embd_normalize', 'reranking']) },
    ]),
    { id: 'config-advanced-rope', title: t.configPage.subAdvRope, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedRope) },
    { id: 'config-advanced-kv', title: t.configPage.subAdvKvCache, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedKvCache) },
    { id: 'config-advanced-context', title: t.configPage.subAdvContextMgmt, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedContextMgmt) },
    { id: 'config-advanced-hardware', title: t.configPage.subAdvHardware, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedHardware) },
    { id: 'config-advanced-server', title: t.configPage.subAdvServer, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedServerBasic) },
    { id: 'config-advanced-server-ext', title: t.configPage.subAdvServerExt, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedServerExt) },
    ...(!isEmbedding ? [
      { id: 'config-advanced-multi', title: t.configPage.subAdvMulti, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedMulti) },
      { id: 'config-advanced-custom', title: t.configPage.customArgs, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.customArgs) },
    ] : []),
  ]
  const directoryGroups = [
    { id: 'config-basic', title: t.configPage.basic, count: countActive(activeParams, BASIC_CONFIG_KEYS) },
    ...(!isEmbedding ? [{ id: 'config-reasoning', title: t.configPage.reasoning, count: countActive(activeParams, REASONING_CONFIG_KEYS) }] : []),
    { id: 'config-performance', title: t.configPage.performance, count: countActive(activeParams, PERFORMANCE_CONFIG_KEYS) },
    {
      id: 'config-advanced',
      title: t.configPage.advSectionTitle,
      count: countActive(activeParams, ADVANCED_CONFIG_KEYS),
      children: advancedDirectoryGroups,
    },
  ]

  const scrollToSection = (id: string) => {
    document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  return (
    <div className="space-y-5">
      <Surface as="section">
        <div className="flex flex-col gap-4 px-5 py-4 lg:flex-row lg:items-center lg:justify-between">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-blue-500/20 bg-blue-500/10 text-blue-300">
              <SlidersHorizontal className="h-5 w-5" />
            </div>
            <div className="min-w-0">
              <div className="flex min-w-0 flex-wrap items-center gap-2">
                <h1 className="truncate text-xl font-semibold text-slate-950 dark:text-slate-50">{t.configPage.title}</h1>
                <Badge tone="slate" className="max-w-[220px] truncate">
                  {inst?.name}
                </Badge>
                {isEmbedding && <Badge tone="blue">{labels.embeddingMode}</Badge>}
              </div>
              <p className="mt-1 max-w-3xl truncate text-sm text-slate-500 dark:text-slate-400">{labels.subtitle}</p>
            </div>
          </div>

          <Button
            onClick={save}
            disabled={!inst || saving || probingEngineCompatibility || capabilityProbeRequired || unsupportedEngineFlags.length > 0}
            variant="primary"
            data-guide="config-save"
            icon={saving ? <LoaderCircle className="h-4 w-4 animate-spin" /> : saved ? <CheckCircle2 className="h-4 w-4" /> : <Sparkles className="h-4 w-4" />}
            className="shrink-0"
          >
            {saving ? t.configPage.saving : saved ? t.configPage.saved : t.configPage.save}
          </Button>
        </div>
      </Surface>

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          { label: labels.activeParams, value: activeParams.size, icon: SlidersHorizontal, tone: 'text-sky-300 bg-sky-500/10 border-sky-500/20' },
          { label: labels.warnings, value: liveWarnings.length, icon: AlertTriangle, tone: 'text-amber-300 bg-amber-500/10 border-amber-500/20' },
          { label: labels.model, value: currentModel ? pathBasename(currentModel.path) : '--', icon: File, tone: 'text-fuchsia-300 bg-fuchsia-500/10 border-fuchsia-500/20' },
          { label: labels.engine, value: currentEngine?.name || '--', icon: Cpu, tone: 'text-emerald-300 bg-emerald-500/10 border-emerald-500/20' },
        ].map(card => (
          <MetricCard key={card.label} label={card.label} value={card.value} icon={<card.icon className="h-5 w-5" />} tone={card.tone} valueClassName="text-xl leading-7" />
        ))}
      </div>

      <div className="grid gap-5 xl:grid-cols-[220px,minmax(0,1fr)] 2xl:grid-cols-[220px,minmax(0,1fr)_320px]">
        <Surface as="aside" className="h-fit p-4 xl:sticky xl:top-4">
          <SectionHeader title={labels.parameterGroups} />
          <nav className="mt-4 space-y-1">
            {directoryGroups.map(group => (
              <div key={group.id}>
                <button
                  type="button"
                  onClick={() => scrollToSection(group.id)}
                  className="flex w-full items-center justify-between gap-3 rounded-lg px-3 py-2 text-left text-sm text-slate-700 transition hover:bg-slate-100 dark:text-slate-200 dark:hover:bg-slate-800"
                >
                  <span className="min-w-0 truncate">{group.title}</span>
                  {group.count > 0 && <Badge tone="emerald" className="shrink-0 px-2 py-0.5">{group.count}</Badge>}
                </button>
                {'children' in group && group.children && (
                  <div className="mt-1 space-y-1 border-l border-slate-200 pl-3 dark:border-slate-800">
                    {group.children.map(child => (
                      <button
                        key={child.id}
                        type="button"
                        onClick={() => scrollToSection(child.id)}
                        className="flex w-full items-center justify-between gap-2 rounded-lg px-2 py-1.5 text-left text-xs text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 dark:hover:bg-slate-800 dark:hover:text-slate-200"
                      >
                        <span className="min-w-0 truncate">{child.title}</span>
                        {child.count > 0 && <span className="shrink-0 rounded-md bg-emerald-500/10 px-1.5 py-0.5 text-[11px] text-emerald-300">{child.count}</span>}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </nav>
        </Surface>

        <div className="space-y-4">
          {saved && (
            <div className="flex items-start gap-3 rounded-lg border border-emerald-200 bg-emerald-50 px-4 py-3 text-sm text-emerald-700 dark:border-emerald-500/20 dark:bg-emerald-500/10 dark:text-emerald-200">
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

          {isEmbedding && (
            <div className="rounded-lg border border-blue-200 bg-blue-50 px-4 py-3 text-sm text-blue-700 dark:border-blue-500/20 dark:bg-blue-500/10 dark:text-blue-200">
              {t.configPage.embeddingBanner}
            </div>
          )}

          <EngineCompatibilityNotice
            engine={currentEngine}
            unsupportedFlags={unsupportedEngineFlags}
            probing={probingEngineCompatibility}
            labels={labels}
          />

          {visibleVectorCleanupGroups.length > 0 && (
            <div className="flex items-start gap-3 rounded-lg border border-emerald-200 bg-emerald-50 px-4 py-3 text-sm text-emerald-700 dark:border-emerald-500/20 dark:bg-emerald-500/10 dark:text-emerald-200">
              <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0" />
              <div className="min-w-0">
                <p className="font-medium">{t.configPage.vectorCleanupTitle}</p>
                <p className="mt-1 text-emerald-700/80 dark:text-emerald-200/80">{t.configPage.vectorCleanupDescription}</p>
                <div className="mt-2 flex flex-wrap gap-2">
                  {visibleVectorCleanupGroups.map(group => (
                    <span key={group.group} className="rounded-md border border-emerald-200 bg-white/70 px-2 py-1 text-xs dark:border-emerald-500/20 dark:bg-slate-950/30">
                      {group.label}: {group.count} {t.configPage.vectorCleanupItems}
                    </span>
                  ))}
                </div>
              </div>
            </div>
          )}

          {!isEmbedding && lastTemplateSnapshot && (
            <div className="flex flex-col gap-3 rounded-lg border border-blue-200 bg-blue-50 px-4 py-3 text-sm text-blue-700 dark:border-blue-500/20 dark:bg-blue-500/10 dark:text-blue-200 sm:flex-row sm:items-center sm:justify-between">
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

          {!isEmbedding && (
          <Surface className="p-5" data-guide="config-presets">
            <div className="flex flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
              <SectionHeader title={labels.quickTemplates} description={labels.quickTemplatesDesc} />
              <Button
                onClick={() => setShowPresetAssistant(true)}
                disabled={engineCompatibilityMode !== 'full'}
                title={engineCompatibilityMode !== 'full' ? labels.templateCompatibilityDisabled : undefined}
                icon={<Sparkles className="h-4 w-4" />}
                className="shrink-0"
              >
                {labels.openPresetAssistant}
              </Button>
            </div>
            <div className="mt-4 grid gap-3 md:grid-cols-3">
              <InsetSurface className="flex items-center gap-3 p-3">
                <ShieldCheck className="h-5 w-5 shrink-0 text-emerald-400" />
                <span className="min-w-0 text-sm text-slate-600 dark:text-slate-300">{labels.presetNoDirectApply}</span>
              </InsetSurface>
              <InsetSurface className="flex items-center gap-3 p-3">
                <ListChecks className="h-5 w-5 shrink-0 text-blue-400" />
                <span className="min-w-0 text-sm text-slate-600 dark:text-slate-300">{labels.presetSafeHint}</span>
              </InsetSurface>
              <InsetSurface className="flex items-center gap-3 p-3">
                <Sparkles className="h-5 w-5 shrink-0 text-violet-400" />
                <span className="min-w-0 text-sm text-slate-600 dark:text-slate-300">{labels.presetRecommended}: {quickTemplates[0]?.title}</span>
              </InsetSurface>
            </div>
          </Surface>
          )}

          <Surface className="p-5">
            <div className="mb-4">
              <SectionHeader title={labels.parameterSearch} description={labels.parameterSearchDesc} />
            </div>
            <TextInput
              type="text"
              value={searchQuery}
              onChange={event => setSearchQuery(event.target.value)}
              placeholder={(t.configPage as any).searchParams || t.perfBlock.searchParams}
              leadingIcon={<Search className="h-4 w-4" />}
            />
          </Surface>

          <BasicSection {...sectionProps} />
          {!isEmbedding && <ReasoningSection {...sectionProps} />}
          <PerformanceSection {...sectionProps} />
          <AdvancedSection {...sectionProps} />
        </div>

        <Surface as="aside" className="h-fit p-5 xl:col-span-2 2xl:sticky 2xl:top-4 2xl:col-span-1">
          <div className="mb-5">
            <SectionHeader title={labels.configContext} description={labels.configContextDesc} />
          </div>

          <div className="grid gap-4 lg:grid-cols-3 2xl:block 2xl:space-y-4">
            <InsetSurface className="p-4">
              <div className="flex items-start gap-3">
                <div className="rounded-lg border border-slate-200 bg-white p-3 text-slate-700 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-300">
                  <Settings className="h-5 w-5" />
                </div>
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium text-slate-900 dark:text-slate-100" title={inst?.name}>{inst?.name}</p>
                  <PathText value={endpoint} maxLength={36} className="mt-1 text-slate-500" />
                </div>
              </div>
            </InsetSurface>

            <InsetSurface className="space-y-3 p-4">
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
                  <span className="truncate text-sm text-slate-500" title={row.label}>{row.label}</span>
                  {row.path ? (
                    <PathText value={row.value} maxLength={44} className="text-right text-slate-700 dark:text-slate-200" />
                  ) : (
                    <span className="min-w-0 truncate text-right text-sm text-slate-700 dark:text-slate-200" title={row.value}>
                      {row.value}
                    </span>
                  )}
                </div>
              ))}
            </InsetSurface>

            <InsetSurface className="p-4">
              <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.validationSummary}</p>
              <div className="mt-3 grid grid-cols-3 gap-2 text-center">
                {[
                  [labels.high, warningCounts.high, 'text-red-300 border-red-500/20 bg-red-500/10'],
                  [labels.medium, warningCounts.medium, 'text-amber-300 border-amber-500/20 bg-amber-500/10'],
                  [labels.low, warningCounts.low, 'text-sky-300 border-sky-500/20 bg-sky-500/10'],
                ].map(([label, count, tone]) => (
                  <div key={label} className={`rounded-lg border px-2 py-3 ${tone}`}>
                    <p className="text-lg font-semibold">{count}</p>
                    <p className="mt-1 text-[11px] uppercase tracking-[0.14em]">{label}</p>
                  </div>
                ))}
              </div>

              <div className="mt-4 space-y-2">
                {checkMessages.length === 0 ? (
                  <div className="rounded-lg border border-emerald-200 bg-emerald-50 px-3 py-2 text-sm text-emerald-700 dark:border-emerald-500/20 dark:bg-emerald-500/10 dark:text-emerald-200">
                    {labels.checkPassed}
                  </div>
                ) : checkMessages.map((message, index) => (
                  <div
                    key={`${message.text}-${index}`}
                    className={`rounded-lg px-3 py-2 text-sm ${
                      message.tone === 'red'
                        ? 'bg-red-50 text-red-700 dark:bg-red-500/10 dark:text-red-200'
                        : 'bg-amber-50 text-amber-700 dark:bg-amber-500/10 dark:text-amber-200'
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
                          ? 'bg-red-50 text-red-700 dark:bg-red-500/10 dark:text-red-200'
                          : warning.severity === 'medium'
                            ? 'bg-amber-50 text-amber-700 dark:bg-amber-500/10 dark:text-amber-200'
                            : 'bg-sky-500/10 text-sky-200'
                      }`}
                    >
                      {(t.configPage as any)[warning.key] || warning.key}
                    </div>
                  ))}
                </div>
              )}
            </InsetSurface>

            <InsetSurface className="p-4">
              <div className="mb-3">
                <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.configDiff}</p>
                <p className="mt-1 text-sm text-slate-500">{labels.configDiffDesc}</p>
              </div>
              {configChanges.length === 0 ? (
                <div className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-sm text-slate-500 dark:border-slate-800 dark:bg-slate-950/40">
                  {labels.noConfigDiff}
                </div>
              ) : (
                <div className="space-y-2">
                  {configChanges.slice(0, 8).map(change => (
                    <div key={change.key} className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 dark:border-slate-800 dark:bg-slate-950/40">
                      <div className="flex items-center justify-between gap-3">
                        <span className="min-w-0 truncate text-sm font-medium text-slate-800 dark:text-slate-200" title={change.label}>{change.label}</span>
                        <Badge tone="blue" className="shrink-0 px-2 py-0.5 text-[11px]">{change.key}</Badge>
                      </div>
                      <div className="mt-2 grid min-w-0 grid-cols-[48px_minmax(0,1fr)] gap-x-2 gap-y-1 text-xs">
                        <span className="text-slate-500">{labels.before}</span>
                        <span className="min-w-0 truncate text-slate-500" title={change.before}>{change.before}</span>
                        <span className="text-slate-500">{labels.after}</span>
                        <span className="min-w-0 truncate text-emerald-700 dark:text-emerald-200" title={change.after}>{change.after}</span>
                      </div>
                    </div>
                  ))}
                  {configChanges.length > 8 && (
                    <div className="rounded-lg border border-slate-200 px-3 py-2 text-sm text-slate-500 dark:border-slate-800 dark:text-slate-400">
                      {`${labels.moreChanges} ${configChanges.length - 8} ${labels.moreChangesSuffix}`}
                    </div>
                  )}
                </div>
              )}
            </InsetSurface>

            <InsetSurface className="p-4">
              <div className="flex items-start gap-3">
                <div className="rounded-lg border border-sky-500/20 bg-sky-500/10 p-3 text-sky-300">
                  <Activity className="h-5 w-5" />
                </div>
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.performanceLink}</p>
                  <p className="mt-1 text-sm leading-5 text-slate-500">{labels.performanceLinkDesc}</p>
                  <Button onClick={() => setActiveTab('perf')} variant="secondary" className="mt-3 w-full" icon={<Activity className="h-4 w-4" />}>
                    {labels.openPerformance}
                  </Button>
                </div>
              </div>
            </InsetSurface>
          </div>
        </Surface>
      </div>

      {showPresetAssistant && !isEmbedding && selectedTemplate && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm">
          <div className="flex max-h-[88vh] w-full max-w-6xl flex-col overflow-hidden rounded-lg border border-slate-200 bg-white shadow-[0_30px_80px_rgba(15,23,42,0.25)] dark:border-slate-800 dark:bg-slate-900 dark:shadow-[0_30px_80px_rgba(2,6,23,0.7)]">
            <div className="flex items-start justify-between gap-4 border-b border-slate-200 bg-slate-50 px-5 py-4 dark:border-slate-800 dark:bg-slate-950/90">
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <Sparkles className="h-5 w-5 text-blue-500" />
                  <h3 className="text-lg font-semibold text-slate-950 dark:text-slate-50">{labels.presetAssistant}</h3>
                </div>
                <p className="mt-1 text-sm text-slate-500">{labels.presetAssistantDesc}</p>
              </div>
              <Button onClick={() => setShowPresetAssistant(false)} variant="subtle" size="icon" aria-label="Close">
                <X className="h-5 w-5" />
              </Button>
            </div>

            <div className="grid min-h-0 flex-1 overflow-hidden lg:grid-cols-[300px_minmax(0,1fr)]">
              <aside className="min-h-0 overflow-y-auto border-b border-slate-200 bg-slate-50 p-4 dark:border-slate-800 dark:bg-slate-950/50 lg:border-b-0 lg:border-r">
                <div className="mb-3 rounded-lg border border-emerald-200 bg-emerald-50 px-3 py-2 text-sm text-emerald-700 dark:border-emerald-500/20 dark:bg-emerald-500/10 dark:text-emerald-200">
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
                        onClick={() => setSelectedTemplateId(template.id)}
                        className={`w-full rounded-lg border p-3 text-left transition ${
                          isSelected
                            ? 'border-blue-400 bg-blue-50 ring-1 ring-blue-400/30 dark:border-blue-500/60 dark:bg-blue-500/10'
                            : 'border-slate-200 bg-white hover:border-slate-300 hover:bg-slate-100 dark:border-slate-800 dark:bg-slate-950/40 dark:hover:border-slate-700 dark:hover:bg-slate-800/70'
                        }`}
                      >
                        <span className="flex items-start justify-between gap-3">
                          <span className="min-w-0">
                            <span className="block text-sm font-semibold text-slate-900 dark:text-slate-100">{template.title}</span>
                            <span className="mt-1 block text-xs leading-5 text-slate-500">{template.subtitle}</span>
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
                      <h4 className="text-xl font-semibold text-slate-950 dark:text-slate-50">{selectedTemplate.title}</h4>
                      <span className={`rounded-full border px-2.5 py-1 text-xs font-medium ${selectedTemplate.tone}`}>
                        {selectedTemplate.subtitle}
                      </span>
                    </div>
                    <p className="mt-2 max-w-3xl text-sm leading-6 text-slate-500">{selectedTemplate.description}</p>
                  </div>
                  <div className="flex shrink-0 items-center gap-2 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-sm text-slate-600 dark:border-slate-800 dark:bg-slate-950/50 dark:text-slate-300">
                    <ListChecks className="h-4 w-4 text-blue-400" />
                    <span>{selectedTemplateChanges.length} {labels.templateChangeCount}</span>
                  </div>
                </div>

                <div className="mt-5 grid gap-4 lg:grid-cols-2">
                  <InsetSurface className="p-4">
                    <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.templateBestFor}</p>
                    <div className="mt-3 flex flex-wrap gap-2">
                      {selectedTemplate.bestFor.map(item => (
                        <Badge key={item} tone="blue" className="px-2.5 py-1 text-xs">{item}</Badge>
                      ))}
                    </div>
                  </InsetSurface>
                  <InsetSurface className="p-4">
                    <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.templateHighlights}</p>
                    <ul className="mt-3 space-y-2 text-sm text-slate-600 dark:text-slate-300">
                      {selectedTemplate.highlights.map(item => (
                        <li key={item} className="flex gap-2">
                          <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-emerald-400" />
                          <span>{item}</span>
                        </li>
                      ))}
                    </ul>
                  </InsetSurface>
                </div>

                {(inst?.status === 'running' || overwrittenChanges.length > 0) && (
                  <div className="mt-4 space-y-2">
                    {inst?.status === 'running' && (
                      <div className="flex gap-2 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-700 dark:border-amber-500/20 dark:bg-amber-500/10 dark:text-amber-200">
                        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                        <span>{labels.templateRunningWarning}</span>
                      </div>
                    )}
                    {overwrittenChanges.length > 0 && (
                      <div className="flex gap-2 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-700 dark:border-amber-500/20 dark:bg-amber-500/10 dark:text-amber-200">
                        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                        <span>{labels.templateOverwriteWarning}</span>
                      </div>
                    )}
                  </div>
                )}

                <div className="mt-5 grid gap-4 xl:grid-cols-[minmax(0,1fr)_320px]">
                  <InsetSurface className="p-4">
                    <div className="mb-4">
                      <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.templateDiff}</p>
                      <p className="mt-1 text-sm text-slate-500">{labels.templateDiffDesc}</p>
                    </div>
                    {selectedTemplateChanges.length === 0 ? (
                      <div className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-3 text-sm text-slate-500 dark:border-slate-800 dark:bg-slate-950/40">
                        {labels.templateNoDiff}
                      </div>
                    ) : (
                      <div className="space-y-4">
                        {selectedTemplateGroups.map(group => (
                          <div key={group.id} className="rounded-lg border border-slate-200 bg-white dark:border-slate-800 dark:bg-slate-950/40">
                            <div className="border-b border-slate-200 px-3 py-2 text-sm font-medium text-slate-800 dark:border-slate-800 dark:text-slate-200">
                              {group.title}
                            </div>
                            <div className="divide-y divide-slate-200 dark:divide-slate-800">
                              {group.changes.map(change => (
                                <div key={change.key} className="grid min-w-0 gap-2 px-3 py-2 text-sm md:grid-cols-[140px_minmax(0,1fr)_minmax(0,1fr)]">
                                  <span className="min-w-0 truncate font-medium text-slate-700 dark:text-slate-200" title={change.label}>{change.label}</span>
                                  <span className="min-w-0 truncate text-slate-500" title={change.before}>{change.before}</span>
                                  <span className="min-w-0 truncate text-emerald-700 dark:text-emerald-200" title={change.after}>{change.after}</span>
                                </div>
                              ))}
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </InsetSurface>

                  <InsetSurface className="h-fit p-4">
                    <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.templateRisks}</p>
                    <div className="mt-3 space-y-2">
                      {selectedTemplate.risks.map(risk => (
                        <div key={risk} className="flex gap-2 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-sm text-slate-600 dark:border-slate-800 dark:bg-slate-950/40 dark:text-slate-300">
                          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-400" />
                          <span>{risk}</span>
                        </div>
                      ))}
                    </div>
                  </InsetSurface>
                </div>
              </div>
            </div>

            <div className="flex flex-col gap-3 border-t border-slate-200 bg-slate-50 px-5 py-4 dark:border-slate-800 dark:bg-slate-950/90 sm:flex-row sm:items-center sm:justify-between">
              <div className="text-sm text-slate-500">
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
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm">
          <div className="flex max-h-[85vh] w-full max-w-3xl flex-col overflow-hidden rounded-lg border border-slate-800 bg-slate-900 shadow-[0_30px_80px_rgba(2,6,23,0.7)]">
            <div className="flex items-center justify-between gap-4 border-b border-slate-800 bg-slate-950/90 px-5 py-4">
              <div className="min-w-0">
                <h3 className="text-lg font-semibold text-slate-50">{t.modelRepo.selectFromRepo}</h3>
                <p className="mt-1 truncate text-sm text-slate-400">
                  {`${labels.pickDesc}${labels.pickSeparator}${pickerTarget === 'model' ? labels.pickPrimary : labels.pickDraft}${labels.pickSuffix}`}
                </p>
              </div>
              <Button
                onClick={() => setShowPicker(false)}
                variant="subtle"
                size="icon"
                aria-label="Close"
              >
                <X className="h-5 w-5" />
              </Button>
            </div>

            <div className="min-h-0 flex-1 overflow-y-auto p-4">
              <div className="min-w-0 space-y-2 rounded-lg border border-slate-800 bg-slate-950/40 p-3">
                {pickerTrees.map(tree => {
                  const renderNode = (node: PickerNode, depth: number): JSX.Element => {
                    if (node.isDir) {
                      const isCollapsed = pickerCollapsed.has(node.path)
                      return (
                        <div key={node.path}>
                          <button
                            onClick={() => {
                              const next = new Set(pickerCollapsed)
                              if (next.has(node.path)) {
                                next.delete(node.path)
                              } else {
                                next.add(node.path)
                              }
                              setPickerCollapsed(next)
                            }}
                            style={{ paddingLeft: `${depth * 14 + 8}px` }}
                            className="flex w-full items-center gap-2 rounded-lg py-2 pr-3 text-left text-sm text-slate-200 transition hover:bg-slate-800/80"
                          >
                            {isCollapsed ? <ChevronRight className="h-3.5 w-3.5 shrink-0 text-slate-500" /> : <ChevronDown className="h-3.5 w-3.5 shrink-0 text-slate-500" />}
                            <FolderOpen className="h-4 w-4 shrink-0 text-amber-400" />
                            {depth === 0 ? (
                              <PathText value={node.path} maxLength={78} className="flex-1 text-slate-200" />
                            ) : (
                              <span className="min-w-0 flex-1 truncate" title={node.name}>{node.name}</span>
                            )}
                          </button>
                          {!isCollapsed && node.children && [...node.children.values()]
                            .sort((left, right) => {
                              if (left.isDir !== right.isDir) {
                                return left.isDir ? -1 : 1
                              }
                              return left.name.localeCompare(right.name)
                            })
                            .map(child => renderNode(child, depth + 1))}
                        </div>
                      )
                    }

                    const model = node.model!
                    if (model.file_type === 'mmproj') {
                      return (
                        <div
                          key={node.path}
                          style={{ paddingLeft: `${depth * 14 + 32}px` }}
                          className="flex items-center gap-2 py-2 pr-3 text-sm text-slate-500"
                        >
                          <Image className="h-4 w-4 shrink-0 text-fuchsia-400" />
                          <span className="min-w-0 flex-1 truncate">{model.name}</span>
                          <span className="shrink-0 text-xs text-fuchsia-300">{t.modelRepo.typeMmprojShort}</span>
                        </div>
                      )
                    }

                    return (
                      <button
                        key={node.path}
                        onClick={() => pickModel(model.path)}
                        style={{ paddingLeft: `${depth * 14 + 32}px` }}
                        className="flex w-full items-center gap-2 rounded-lg py-2 pr-3 text-left text-sm text-slate-100 transition hover:bg-blue-500/10"
                      >
                        <File className="h-4 w-4 shrink-0 text-sky-400" />
                        <span className="min-w-0 flex-1 truncate">{model.name}</span>
                        <span className="shrink-0 text-xs text-slate-500">{model.quant_type || ''}</span>
                      </button>
                    )
                  }

                  return renderNode(tree, 0)
                })}
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

export default ConfigPage
