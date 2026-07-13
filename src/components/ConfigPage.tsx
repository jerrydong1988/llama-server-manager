import { useEffect, useMemo, useRef, useState } from 'react'
import { Activity, AlertTriangle, CheckCircle2, ChevronDown, ChevronRight, Cpu, File, FolderOpen, Image, ListChecks, RotateCcw, Search, Settings, ShieldCheck, SlidersHorizontal, Sparkles, X } from 'lucide-react'
import { useAppStore, type InstanceConfig, type ModelInfo, defaultInstanceConfig } from '../store'
import { useI18n } from '../i18n'
import { validateConfig, type Warning } from '../validators'
import {
  BasicSection,
  ReasoningSection,
  PerformanceSection,
  AdvancedSection,
  BASIC_CONFIG_KEYS,
  REASONING_CONFIG_KEYS,
  PERFORMANCE_CONFIG_KEYS,
  ADVANCED_CONFIG_KEYS,
  ADVANCED_GROUP_CONFIG_KEYS,
} from './ConfigPage/sections'
import { getActiveParams } from './ConfigPage/activeParams'
import { isPathWithinRoot, normalizePath, pathBasename, pathDirname, pathJoin } from '../utils/path'
import { formatHostPort } from '../utils/network'
import { detectModelWorkload, normalizeInstanceConfig, type VectorCleanupChange } from '../modelPolicy'
import { normalizeModelPath } from '../store/bootstrap'
import { _matchedElements } from './ConfigPage/shared'
import { Badge, Button, EmptyState, InsetSurface, MetricCard, PathText, SectionHeader, Surface, TextInput } from './ui'

interface PickerNode {
  name: string
  path: string
  isDir: boolean
  children?: Map<string, PickerNode>
  model?: ModelInfo
}

const buildPickerTree = (rootDir: string, models: ModelInfo[]): PickerNode => {
  const normalizedRoot = normalizePath(rootDir)
  const root: PickerNode = { name: rootDir, path: normalizedRoot, isDir: true, children: new Map() }

  for (const model of models) {
    const normalizedPath = normalizePath(model.path)
    if (!isPathWithinRoot(normalizedPath, normalizedRoot)) {
      continue
    }

    const relative = normalizedPath.slice(normalizedRoot.length).replace(/^\/+/, '')
    if (!relative) {
      continue
    }

    const parts = relative.split('/')
    let cursor = root

    for (let index = 0; index < parts.length; index += 1) {
      const part = parts[index]
      if (index === parts.length - 1) {
        cursor.children!.set(part, { name: part, path: model.path, isDir: false, model })
      } else {
        if (!cursor.children!.has(part)) {
          cursor.children!.set(part, {
            name: part,
            path: pathJoin(cursor.path, part),
            isDir: true,
            children: new Map(),
          })
        }
        cursor = cursor.children!.get(part)!
      }
    }
  }

  return root
}

const countActive = (activeParams: Set<keyof InstanceConfig>, keys: Array<keyof InstanceConfig>) =>
  keys.filter(key => activeParams.has(key)).length

type ConfigChange = {
  key: keyof InstanceConfig
  label: string
  before: string
  after: string
}

type ConfigTemplate = {
  id: string
  title: string
  subtitle: string
  description: string
  bestFor: string[]
  highlights: string[]
  tone: string
  changes: Partial<InstanceConfig>
  risks: string[]
}

type TemplateSnapshot = {
  templateId: string
  templateTitle: string
  config: InstanceConfig
}

type ChangeGroup = {
  id: string
  title: string
  keys: Array<keyof InstanceConfig>
}

const isEqualValue = (left: unknown, right: unknown) => {
  if (Array.isArray(left) || Array.isArray(right)) {
    return JSON.stringify(left ?? []) === JSON.stringify(right ?? [])
  }
  return left === right
}

const formatValue = (value: unknown, labels: Record<string, string>) => {
  if (Array.isArray(value)) {
    return value.length > 0 ? value.join(' ') : labels.emptyValue
  }
  if (typeof value === 'boolean') {
    return value ? labels.on : labels.off
  }
  if (value === '' || value === null || value === undefined) {
    return labels.emptyValue
  }
  return String(value)
}

const formatConfigValue = (key: keyof InstanceConfig, value: unknown, labels: Record<string, string>, t: any) => (
  key === 'custom_args'
    ? `${Array.isArray(value) ? value.length : 0} ${t.configPage.vectorCleanupItems}`
    : formatValue(value, labels)
)

const fieldLabel = (key: keyof InstanceConfig, t: any) => {
  const labelMap: Partial<Record<keyof InstanceConfig, string>> = {
    model_path: t.configPage.modelPath,
    alias: t.configPage.alias,
    chat_template: t.configPage.chatTemplate,
    host: t.configPage.host,
    port: t.configPage.portLabel,
    gpu_layers: t.configPage.gpuLayers,
    gpu_layers_auto: t.configPage.gpuLayersAuto,
    ctx_size: t.configPage.ctxSize,
    ctx_size_auto: t.configPage.ctxAuto,
    embedding: t.configPage.embedding,
    pooling: t.configPage.pooling,
    reasoning: t.configPage.reasoningSwitch,
    reasoning_format: t.configPage.reasoningFormat,
    reasoning_effort: t.configPage.reasoningEffort,
    reasoning_budget: t.configPage.reasoningBudget,
    reasoning_budget_message: t.configPage.reasoningBudgetMsg,
    draft_model_path: t.configPage.draftModel,
    draft_tokens: t.configPage.draftTokens,
    spec_type: t.configPage.specType,
    spec_draft_n_min: t.configPage.specDraftNMin,
    temp: t.configPage.temp,
    top_k: t.configPage.topK,
    top_p: t.configPage.topP,
    repeat_penalty: t.configPage.repeatPenalty,
    n_predict: t.configPage.nPredict,
    ignore_eos: t.configPage.ignoreEos,
    reverse_prompt: t.configPage.reversePrompt,
    threads: t.configPage.threads,
    threads_batch: t.configPage.threadsBatch,
    batch_size: t.configPage.batchSize,
    ubatch_size: t.configPage.ubatchSize,
    parallel: t.configPage.parallel,
    cont_batching: t.configPage.contBatching,
    flash_attn: t.configPage.flashAttn,
    mlock: t.configPage.mlock,
    no_mmap: t.configPage.noMmap,
    no_repack: t.configPage.noRepack,
    numa: t.configPage.numa,
    cache_ram: t.configPage.cacheRam,
    metrics: t.configPage.metrics,
    props: t.configPage.props,
    perf: t.configPage.perf,
    verbose: t.configPage.verbose,
    custom_args: t.configPage.customArgs,
  }
  return labelMap[key] || String(key).replace(/_/g, ' ')
}

const getConfigChanges = (local: InstanceConfig, baseline: InstanceConfig, t: any, labels: Record<string, string>): ConfigChange[] =>
  (Object.keys(local) as Array<keyof InstanceConfig>)
    .filter(key => !isEqualValue(local[key], baseline[key]))
    .map(key => ({
      key,
      label: fieldLabel(key, t),
      before: formatConfigValue(key, baseline[key], labels, t),
      after: formatConfigValue(key, local[key], labels, t),
    }))

const getTemplateChanges = (local: InstanceConfig, changes: Partial<InstanceConfig>, t: any, labels: Record<string, string>): ConfigChange[] =>
  (Object.keys(changes) as Array<keyof InstanceConfig>)
    .filter(key => !isEqualValue(local[key], changes[key]))
    .map(key => ({
      key,
      label: fieldLabel(key, t),
      before: formatConfigValue(key, local[key], labels, t),
      after: formatConfigValue(key, changes[key], labels, t),
    }))

const groupTemplateChanges = (changes: ConfigChange[], groups: ChangeGroup[], otherTitle: string) => {
  const grouped = groups
    .map(group => ({
      ...group,
      changes: changes.filter(change => group.keys.includes(change.key)),
    }))
    .filter(group => group.changes.length > 0)

  const groupedKeys = new Set(grouped.flatMap(group => group.changes.map(change => change.key)))
  const otherChanges = changes.filter(change => !groupedKeys.has(change.key))

  return otherChanges.length > 0
    ? [...grouped, { id: 'other', title: otherTitle, keys: [], changes: otherChanges }]
    : grouped
}

const ConfigPage = () => {
  const { instances, activeConfigInstanceId, updateInstance, saveConfig, models, modelDirs, engines, defaultEngineId, setActiveTab } = useAppStore()
  const { t, lang } = useI18n()
  const zh = lang === 'zh-CN'
  const inst = instances.find(instance => instance.id === activeConfigInstanceId)

  const [local, setLocal] = useState<InstanceConfig | null>(null)
  const [saved, setSaved] = useState(false)
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

  useEffect(() => {
    return () => {
      mountedRef.current = false
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
    if (inst) {
      setLocal({ ...defaultInstanceConfig(), ...inst.config })
    } else {
      setLocal(null)
    }
  }, [activeConfigInstanceId, inst?.config])

  useEffect(() => {
    setAppliedTemplateId(null)
    setLastTemplateSnapshot(null)
    setShowPresetAssistant(false)
    setVectorCleanupChanges([])
  }, [activeConfigInstanceId])

  const set = (key: keyof InstanceConfig, value: any) => {
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

  const isEmbedding = useMemo(() => (
    local ? detectModelWorkload(currentModel, local.model_path, local) !== 'inference' : false
  ), [currentModel, local])

  if (!local) {
    return (
      <div className="space-y-5">
        <EmptyState icon={<Settings className="h-10 w-10" />} title={t.configPage.title} description={t.configPage.noInstance} />
      </div>
    )
  }

  const activeParams = getActiveParams(local, isEmbedding)
  const currentEngine = engines.find(engine => engine.id === (local.engine_id || defaultEngineId || '')) ?? engines[0] ?? null
  const primaryModelPath = currentModel?.path || local.model_path || ''
  const draftModelPath = local.draft_model_path || ''
  const endpoint = formatHostPort(local.host || '127.0.0.1', local.port)

  const pickModel = (modelPath: string) => {
    if (pickerTarget === 'model') {
      const selectedModel = models.find(model => normalizeModelPath(model.path) === normalizeModelPath(modelPath))
      const directory = pathDirname(modelPath)
      const mmproj = models.find(model => pathDirname(model.path) === directory && model.file_type === 'mmproj')
      const candidate = { ...local, model_path: modelPath, mmproj_path: mmproj?.path ?? '' }
      const normalized = normalizeInstanceConfig(candidate, selectedModel)
      setAppliedTemplateId(null)
      setLastTemplateSnapshot(null)
      setLocal(normalized.config)
      setVectorCleanupChanges(normalized.vectorMode ? normalized.changes : [])
    } else {
      set('draft_model_path', modelPath)
    }
    setShowPicker(false)
  }

  const save = async () => {
    if (!inst) {
      return
    }

    const engine = engines.find(item => item.id === (local.engine_id || defaultEngineId || '')) || engines[0]
    const warnings = validateConfig(local, currentModel, engine)

    updateInstance(inst.id, { config: local })
    try {
      await saveConfig()
    } catch {
      return
    }
    setSaved(true)
    setSaveWarnings(warnings)
    setVectorCleanupChanges([])

    setTimeout(() => {
      if (mountedRef.current) {
        setSaved(false)
        setSaveWarnings([])
      }
    }, 6000)
  }

  const sectionProps = {
    local,
    set,
    t,
    isEmbedding,
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
  const labels = {
    subtitle: zh
      ? '\u96c6\u4e2d\u8c03\u6574\u542f\u52a8\u53c2\u6570\u3001\u6a21\u578b\u884c\u4e3a\u548c\u8fd0\u884c\u786c\u4ef6\uff0c\u5e76\u4fdd\u7559\u5f53\u524d\u751f\u6548\u914d\u7f6e\u7684\u53ef\u89c1\u6027\u3002'
      : 'Tune server startup, model behavior, and runtime hardware in one place without losing visibility into what is actually active.',
    activeParams: zh ? '\u5df2\u542f\u7528\u53c2\u6570' : 'Active Params',
    warnings: zh ? '\u544a\u8b66' : 'Warnings',
    model: zh ? '\u6a21\u578b' : 'Model',
    engine: zh ? '\u5f15\u64ce' : 'Engine',
    parameterSearch: zh ? '\u53c2\u6570\u641c\u7d22' : 'Parameter Search',
    parameterSearchDesc: zh
      ? '\u8f93\u5165\u53c2\u6570\u540d\u3001\u6807\u7b7e\u6216\u5173\u952e\u8bcd\uff0c\u76f4\u63a5\u8df3\u8f6c\u5230\u5bf9\u5e94\u5b57\u6bb5\u3002'
      : 'Jump directly to fields by typing a flag name, label, or keyword.',
    configContext: zh ? '\u914d\u7f6e\u4e0a\u4e0b\u6587' : 'Config Context',
    configContextDesc: zh ? '\u5feb\u901f\u67e5\u770b\u6b64\u5b9e\u4f8b\u5f53\u524d\u5c06\u5982\u4f55\u542f\u52a8\u3002' : 'A quick read on what this instance will launch with right now.',
    primaryModel: zh ? '\u4e3b\u6a21\u578b' : 'Primary model',
    draftModel: zh ? '\u8349\u7a3f\u6a21\u578b' : 'Draft model',
    enginePath: zh ? '\u5f15\u64ce\u8def\u5f84' : 'Engine path',
    endpoint: zh ? '\u7aef\u70b9' : 'Endpoint',
    embeddingMode: zh ? '\u5d4c\u5165\u6a21\u5f0f' : 'Embedding mode',
    modifiedParams: zh ? '\u5df2\u4fee\u6539\u53c2\u6570' : 'Modified params',
    validationSummary: zh ? '\u914d\u7f6e\u68c0\u67e5' : 'Configuration Check',
    configDiff: zh ? '\u672a\u4fdd\u5b58\u53d8\u66f4' : 'Unsaved Changes',
    configDiffDesc: zh ? '\u5bf9\u6bd4\u5df2\u4fdd\u5b58\u914d\u7f6e\uff0c\u663e\u793a\u672c\u6b21\u8c03\u6574\u7684\u5177\u4f53\u53c2\u6570\u3002' : 'Compared with the saved config, these are the fields changed in this editing session.',
    noConfigDiff: zh ? '\u5f53\u524d\u6ca1\u6709\u672a\u4fdd\u5b58\u7684\u53c2\u6570\u53d8\u66f4\u3002' : 'No unsaved parameter changes.',
    before: zh ? '\u539f\u503c' : 'Before',
    after: zh ? '\u65b0\u503c' : 'After',
    emptyValue: zh ? '\u672a\u8bbe\u7f6e' : 'Not set',
    moreChanges: zh ? '\u8fd8\u6709' : 'plus',
    moreChangesSuffix: zh ? '\u9879\u53d8\u66f4' : 'more changes',
    quickTemplates: zh ? '\u914d\u7f6e\u9884\u8bbe' : 'Config Presets',
    quickTemplatesDesc: zh ? '\u9884\u8bbe\u4e0d\u518d\u76f4\u63a5\u6539\u52a8\u914d\u7f6e\uff1b\u6253\u5f00\u52a9\u624b\u540e\u5148\u770b\u573a\u666f\u3001\u5dee\u5f02\u548c\u98ce\u9669\uff0c\u518d\u660e\u786e\u5e94\u7528\u5230\u8349\u7a3f\u3002' : 'Presets no longer change config directly. Open the assistant, review the scenario, diff, and risks, then explicitly apply to the draft.',
    openPresetAssistant: zh ? '\u6253\u5f00\u9884\u8bbe\u52a9\u624b' : 'Open Preset Assistant',
    presetAssistant: zh ? '\u914d\u7f6e\u9884\u8bbe\u52a9\u624b' : 'Config Preset Assistant',
    presetAssistantDesc: zh ? '\u9009\u4e2d\u9884\u8bbe\u53ea\u4f1a\u9884\u89c8\uff0c\u4e0d\u4f1a\u7acb\u5373\u6539\u52a8\u5f53\u524d\u914d\u7f6e\u3002' : 'Selecting a preset only previews it; the current config is not changed until you apply it.',
    presetSafeHint: zh ? '\u5b89\u5168\u6d41\u7a0b\uff1a\u9009\u62e9 -> \u9884\u89c8\u5dee\u5f02 -> \u786e\u8ba4\u5e94\u7528 -> \u4fdd\u5b58\u914d\u7f6e\u3002' : 'Safe flow: select, preview diff, apply, then save the config.',
    presetNoDirectApply: zh ? '\u6b64\u533a\u57df\u4e0d\u4f1a\u76f4\u63a5\u8986\u76d6\u53c2\u6570' : 'This area does not directly overwrite parameters',
    presetRecommended: zh ? '\u63a8\u8350\u8d77\u70b9' : 'Recommended starting point',
    presetCurrent: zh ? '\u5f53\u524d\u9009\u4e2d' : 'Selected',
    applyTemplate: zh ? '\u5e94\u7528\u5230\u8349\u7a3f' : 'Apply to Draft',
    appliedTemplate: zh ? '\u5df2\u5e94\u7528' : 'Applied',
    templateAppliedMessage: zh ? '\u5df2\u5e94\u7528\u5230\u672c\u5730\u8349\u7a3f\uff0c\u5c1a\u672a\u4fdd\u5b58\u5230\u5b9e\u4f8b\u3002' : 'Applied to the local draft, not saved to the instance yet.',
    undoTemplate: zh ? '\u64a4\u9500\u9884\u8bbe' : 'Undo Preset',
    templateBestFor: zh ? '\u9002\u7528\u573a\u666f' : 'Best For',
    templateHighlights: zh ? '\u4e3b\u8981\u8c03\u6574' : 'Main Adjustments',
    templateDiff: zh ? '\u5c06\u4fee\u6539' : 'Changes',
    templateChangeCount: zh ? '\u9879\u5c06\u4fee\u6539' : 'changes',
    templateDiffDesc: zh ? '\u53ea\u5217\u51fa\u4e0e\u5f53\u524d\u8349\u7a3f\u4e0d\u540c\u7684\u5b57\u6bb5\u3002' : 'Only fields that differ from the current draft are listed.',
    templateNoDiff: zh ? '\u5f53\u524d\u8349\u7a3f\u5df2\u7b26\u5408\u8be5\u9884\u8bbe\u3002' : 'The draft already matches this preset.',
    templateRisks: zh ? '\u98ce\u9669' : 'Risks',
    templateOverwriteWarning: zh ? '\u5f53\u524d\u8349\u7a3f\u4e2d\u5df2\u6709\u672a\u4fdd\u5b58\u53d8\u66f4\uff0c\u5e94\u7528\u9884\u8bbe\u53ef\u80fd\u8986\u76d6\u5176\u4e2d\u4e00\u90e8\u5206\u3002' : 'The current draft has unsaved changes. Applying this preset may overwrite part of them.',
    templateRunningWarning: zh ? '\u6b64\u5b9e\u4f8b\u6b63\u5728\u8fd0\u884c\uff1b\u9884\u8bbe\u53ea\u4f1a\u6539\u8349\u7a3f\uff0c\u4fdd\u5b58\u540e\u901a\u5e38\u9700\u8981\u91cd\u542f\u624d\u4f1a\u751f\u6548\u3002' : 'This instance is running. The preset only changes the draft; after saving, restart is usually required.',
    templateCancel: zh ? '\u53d6\u6d88' : 'Cancel',
    templateGroupsPerformance: zh ? '\u6027\u80fd\u4e0e\u5e76\u53d1' : 'Performance and Concurrency',
    templateGroupsContext: zh ? '\u4e0a\u4e0b\u6587\u4e0e\u7f13\u5b58' : 'Context and Cache',
    templateGroupsHardware: zh ? '\u786c\u4ef6\u4e0e\u5185\u5b58' : 'Hardware and Memory',
    templateGroupsObservability: zh ? '\u89c2\u6d4b\u4e0e API' : 'Observability and API',
    templateGroupsSpeculative: zh ? '\u63a8\u6d4b\u89e3\u7801' : 'Speculative Decoding',
    templateGroupsGeneration: zh ? '\u751f\u6210\u884c\u4e3a' : 'Generation Behavior',
    templateGroupsOther: zh ? '\u5176\u4ed6' : 'Other',
    checkPassed: zh ? '\u672a\u53d1\u73b0\u660e\u663e\u914d\u7f6e\u51b2\u7a81\u3002' : 'No obvious configuration conflicts found.',
    missingModel: zh ? '\u8bf7\u5148\u9009\u62e9\u4e3b\u6a21\u578b\uff0c\u5426\u5219\u5b9e\u4f8b\u65e0\u6cd5\u6b63\u5e38\u542f\u52a8\u3002' : 'Select a primary model before starting this instance.',
    missingEngine: zh ? '\u672a\u5339\u914d\u5230\u53ef\u7528\u5f15\u64ce\uff0c\u8bf7\u786e\u8ba4\u5f15\u64ce\u626b\u63cf\u7ed3\u679c\u3002' : 'No usable engine is matched. Check engine scan results.',
    liveWarnings: zh ? '\u6761\u53c2\u6570\u98ce\u9669\u9700\u590d\u6838' : 'parameter risks need review',
    performanceLink: zh ? '\u6027\u80fd\u8bca\u65ad' : 'Performance Diagnostics',
    performanceLinkDesc: zh
      ? '\u4fdd\u5b58\u5e76\u542f\u52a8\u5b9e\u4f8b\u540e\uff0c\u53ef\u5728\u6027\u80fd\u76d1\u63a7\u9875\u67e5\u770b\u541e\u5410\u3001\u663e\u5b58\u548c slot \u8bca\u65ad\u3002'
      : 'After saving and starting the instance, inspect throughput, VRAM, and slot diagnostics from Performance.',
    openPerformance: zh ? '\u6253\u5f00\u6027\u80fd\u76d1\u63a7' : 'Open Performance',
    high: zh ? '\u9ad8' : 'High',
    medium: zh ? '\u4e2d' : 'Medium',
    low: zh ? '\u4f4e' : 'Low',
    on: zh ? '\u5f00' : 'On',
    off: zh ? '\u5173' : 'Off',
    pickPrimary: zh ? '\u4e3b\u6a21\u578b' : 'the primary model',
    pickDraft: zh ? '\u8349\u7a3f\u6a21\u578b' : 'the draft model',
    pickDesc: zh ? '\u4ece\u8d44\u6e90\u5e93\u4e2d\u9009\u62e9\u6587\u4ef6\uff1a' : 'Choose a repository asset for',
    parameterGroups: zh ? '\u53c2\u6570\u5206\u7ec4' : 'Parameter Groups',
  }

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

  const savedBaseline = { ...defaultInstanceConfig(), ...(inst?.config ?? {}) }
  const vectorCleanupKeys = new Set(vectorCleanupChanges.map(change => change.key))
  const configChanges = getConfigChanges(local, savedBaseline, t, labels)
    .filter(change => !vectorCleanupKeys.has(change.key))
  const liveWarnings = validateConfig(local, currentModel, currentEngine)
  const visibleWarnings = saved ? saveWarnings : liveWarnings
  const warningCounts = {
    high: liveWarnings.filter(warning => warning.severity === 'high').length,
    medium: liveWarnings.filter(warning => warning.severity === 'medium').length,
    low: liveWarnings.filter(warning => warning.severity === 'low').length,
  }
  const checkMessages = [
    ...(!primaryModelPath ? [{ tone: 'red', text: labels.missingModel }] : []),
    ...(!currentEngine ? [{ tone: 'amber', text: labels.missingEngine }] : []),
    ...(liveWarnings.length > 0 ? [{ tone: liveWarnings.some(warning => warning.severity === 'high') ? 'red' : 'amber', text: `${liveWarnings.length} ${labels.liveWarnings}` }] : []),
  ]
  const quickTemplates: ConfigTemplate[] = [
    {
      id: 'safe-start',
      title: zh ? '稳妥启动' : 'Safe Start',
      subtitle: zh ? '先跑起来，再逐步加压' : 'Start reliably, then tune upward',
      description: zh ? '优先保证启动成功、日志可观察和资源压力可控，适合新模型或新引擎的第一次验证。' : 'Prioritizes successful startup, observability, and controlled resource pressure for first validation of a model or engine.',
      bestFor: zh ? ['新模型首测', '不确定显存余量', '排查启动失败'] : ['First model test', 'Unknown VRAM headroom', 'Startup troubleshooting'],
      highlights: zh ? ['固定 4K 上下文', '降低 batch 与并发', '开启 metrics / props / slots'] : ['Fixed 4K context', 'Lower batch and concurrency', 'Enable metrics / props / slots'],
      tone: 'border-emerald-500/25 bg-emerald-500/10 text-emerald-200',
      changes: { ctx_size_auto: false, ctx_size: 4096, gpu_layers_auto: true, batch_size: 1024, ubatch_size: 256, parallel: 1, flash_attn: 'auto', cont_batching: true, cache_ram: 4096, metrics: true, props: true, slots_enabled: true },
      risks: zh ? ['吞吐会低于并发服务型配置。', '固定 4K 上下文不适合长文档任务。'] : ['Throughput will be lower than service-oriented configs.', 'Fixed 4K context is not ideal for long-document tasks.'],
    },
    {
      id: 'single-chat',
      title: zh ? '单用户聊天' : 'Single User Chat',
      subtitle: zh ? '低延迟、长会话、交互优先' : 'Low-latency interactive chat',
      description: zh ? '面向本机单人交互，保持自动上下文和较温和的批处理，减少并发争抢。' : 'For local single-user interaction with automatic context and moderate batching, reducing concurrency contention.',
      bestFor: zh ? ['本机聊天', '长时间单会话', '稳定低延迟'] : ['Local chat', 'Long single sessions', 'Stable latency'],
      highlights: zh ? ['使用模型原生上下文', 'parallel 设为 1', '保留连续批处理'] : ['Use native model context', 'Set parallel to 1', 'Keep continuous batching'],
      tone: 'border-sky-500/25 bg-sky-500/10 text-sky-200',
      changes: { ctx_size_auto: true, gpu_layers_auto: true, batch_size: 2048, ubatch_size: 512, parallel: 1, cont_batching: true, flash_attn: 'auto', cache_ram: 8192, metrics: true, props: true, slots_enabled: true, n_predict: -1 },
      risks: zh ? ['多客户端并发时响应会排队。', '自动上下文可能占用较多 KV cache。'] : ['Multiple clients may queue behind each other.', 'Automatic context can consume more KV cache.'],
    },
    {
      id: 'api-service',
      title: zh ? 'API 并发服务' : 'Concurrent API Service',
      subtitle: zh ? '多请求吞吐优先' : 'Throughput for multiple clients',
      description: zh ? '提升 batch、ubatch 和 parallel，适合作为 OpenAI 兼容 API 的本地服务端。' : 'Raises batch, ubatch, and parallel slots for local OpenAI-compatible API service use.',
      bestFor: zh ? ['多客户端 API', '工具链调用', '请求吞吐测试'] : ['Multi-client API', 'Tool integrations', 'Throughput testing'],
      highlights: zh ? ['parallel 设为 4', 'batch 提高到 4096', '优先启用 Flash Attention'] : ['Set parallel to 4', 'Raise batch to 4096', 'Prefer Flash Attention'],
      tone: 'border-blue-500/25 bg-blue-500/10 text-blue-200',
      changes: { ctx_size_auto: true, gpu_layers_auto: true, batch_size: 4096, ubatch_size: 1024, parallel: 4, cont_batching: true, flash_attn: 'on', cache_ram: 8192, metrics: true, props: true, slots_enabled: true },
      risks: zh ? ['需要更多显存和 KV cache 空间。', '不支持 Flash Attention 的后端可能出现警告或降速。'] : ['Needs more VRAM and KV cache headroom.', 'Backends without Flash Attention support may warn or slow down.'],
    },
    {
      id: 'long-context',
      title: zh ? '长上下文阅读' : 'Long Context Reading',
      subtitle: zh ? '文档、代码库、长会话' : 'Documents, codebases, long sessions',
      description: zh ? '扩大上下文和缓存预留，适合长文档摘要、代码阅读或长多轮对话。' : 'Expands context and cache headroom for long-document summarization, code reading, or extended conversations.',
      bestFor: zh ? ['长文档摘要', '代码库阅读', '长多轮对话'] : ['Document summarization', 'Codebase reading', 'Long conversations'],
      highlights: zh ? ['固定 32K 上下文', '提高 cache_ram', '启用 context shift'] : ['Fixed 32K context', 'Increase cache_ram', 'Enable context shift'],
      tone: 'border-violet-500/25 bg-violet-500/10 text-violet-200',
      changes: { ctx_size_auto: false, ctx_size: 32768, batch_size: 2048, ubatch_size: 512, parallel: 1, cache_ram: 16384, ctx_checkpoints: 64, flash_attn: 'auto', context_shift: true, metrics: true, props: true },
      risks: zh ? ['KV cache 会明显增加内存或显存压力。', '超过模型原生上下文时，质量可能下降。'] : ['KV cache can substantially increase RAM or VRAM pressure.', 'Quality may degrade beyond the model native context.'],
    },
    {
      id: 'low-vram',
      title: zh ? '低显存保命' : 'Low VRAM Rescue',
      subtitle: zh ? '保留自动 GPU，降低压力' : 'Keep automatic GPU, reduce pressure',
      description: zh ? '降低批处理、并发和缓存压力，但不强制切换纯 CPU，适合显存紧张时保守启动。' : 'Reduces batching, concurrency, and cache pressure without forcing CPU-only mode, useful when VRAM is tight.',
      bestFor: zh ? ['显存接近上限', '模型较大', '先启动后调优'] : ['VRAM near limit', 'Large models', 'Start before tuning'],
      highlights: zh ? ['保留自动 GPU 层数', '缩小 batch / ubatch', '限制 4K 上下文'] : ['Keep automatic GPU layers', 'Shrink batch / ubatch', 'Limit context to 4K'],
      tone: 'border-amber-500/25 bg-amber-500/10 text-amber-200',
      changes: { ctx_size_auto: false, ctx_size: 4096, gpu_layers_auto: true, batch_size: 512, ubatch_size: 128, parallel: 1, cache_ram: 2048, mlock: false, flash_attn: 'auto', no_kv_offload: false, metrics: true, props: true, slots_enabled: true },
      risks: zh ? ['长输入和高并发性能会明显受限。', '如果仍然 OOM，需要再手动降低 GPU 层数或上下文。'] : ['Long prompts and high concurrency will be limited.', 'If OOM persists, manually lower GPU layers or context size.'],
    },
    {
      id: 'spec-draft-mtp',
      title: zh ? '推测解码实验' : 'Speculative Decoding Lab',
      subtitle: zh ? '用于内置 MTP 或草稿模型验证' : 'Validate built-in MTP or draft models',
      description: zh ? '打开 draft-mtp 类型并设置温和的草稿 token，用于对比推测解码收益。' : 'Enables draft-mtp with moderate draft tokens for comparing speculative decoding benefits.',
      bestFor: zh ? ['内置 MTP 模型', '草稿模型实验', '吞吐对比'] : ['Built-in MTP models', 'Draft model experiments', 'Throughput comparison'],
      highlights: zh ? ['spec_type 设为 draft-mtp', '草稿 token 设为 3', '保留观测指标'] : ['Set spec_type to draft-mtp', 'Use 3 draft tokens', 'Keep observability enabled'],
      tone: 'border-fuchsia-500/25 bg-fuchsia-500/10 text-fuchsia-200',
      changes: { spec_type: 'draft-mtp', draft_tokens: 3, spec_draft_n_min: 1, batch_size: 4096, ubatch_size: 1024, cont_batching: true, metrics: true, props: true, slots_enabled: true },
      risks: zh ? ['非 MTP 模型可能需要外部草稿模型。', '推测接受率过低时可能没有加速收益。'] : ['Non-MTP models may need an external draft model.', 'Low draft acceptance can remove the speed benefit.'],
    },
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
    setLocal({ ...lastTemplateSnapshot.config })
    setAppliedTemplateId(null)
    setLastTemplateSnapshot(null)
    setSaved(false)
    setSaveWarnings([])
  }

  const directoryGroups = [
    { id: 'config-basic', title: t.configPage.basic, count: countActive(activeParams, BASIC_CONFIG_KEYS) },
    { id: 'config-reasoning', title: t.configPage.reasoning, count: countActive(activeParams, REASONING_CONFIG_KEYS) },
    { id: 'config-performance', title: t.configPage.performance, count: countActive(activeParams, PERFORMANCE_CONFIG_KEYS) },
    {
      id: 'config-advanced',
      title: t.configPage.advSectionTitle,
      count: countActive(activeParams, ADVANCED_CONFIG_KEYS),
      children: [
        { id: 'config-advanced-reasoning', title: t.configPage.subAdvReasoning, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedReasoningConfig) },
        { id: 'config-advanced-model', title: t.configPage.subAdvModelAdapt, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedModelAdapt) },
        { id: 'config-advanced-sampling', title: t.configPage.subAdvSampling, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedSampling) },
        { id: 'config-advanced-sampling-ext', title: t.configPage.subAdvSamplingExt, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedSamplingExt) },
        { id: 'config-advanced-spec', title: t.configPage.subAdvSpec, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedSpec) },
        { id: 'config-advanced-rope', title: t.configPage.subAdvRope, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedRope) },
        { id: 'config-advanced-kv', title: t.configPage.subAdvKvCache, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedKvCache) },
        { id: 'config-advanced-context', title: t.configPage.subAdvContextMgmt, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedContextMgmt) },
        { id: 'config-advanced-hardware', title: t.configPage.subAdvHardware, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedHardware) },
        { id: 'config-advanced-server', title: t.configPage.subAdvServer, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedServerBasic) },
        { id: 'config-advanced-server-ext', title: t.configPage.subAdvServerExt, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedServerExt) },
        { id: 'config-advanced-multi', title: t.configPage.subAdvMulti, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.advancedMulti) },
        { id: 'config-advanced-custom', title: t.configPage.customArgs, count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.customArgs) },
      ],
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
            disabled={!inst}
            variant="primary"
            data-guide="config-save"
            icon={saved ? <CheckCircle2 className="h-4 w-4" /> : <Sparkles className="h-4 w-4" />}
            className="shrink-0"
          >
            {saved ? t.configPage.saved : t.configPage.save}
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

          {lastTemplateSnapshot && (
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

          <Surface className="p-5" data-guide="config-presets">
            <div className="flex flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
              <SectionHeader title={labels.quickTemplates} description={labels.quickTemplatesDesc} />
              <Button onClick={() => setShowPresetAssistant(true)} icon={<Sparkles className="h-4 w-4" />} className="shrink-0">
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
          <ReasoningSection {...sectionProps} />
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
                      {zh ? `${labels.moreChanges} ${configChanges.length - 8} ${labels.moreChangesSuffix}` : `${labels.moreChanges} ${configChanges.length - 8} ${labels.moreChangesSuffix}`}
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

      {showPresetAssistant && selectedTemplate && (
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
                  {zh ? `${labels.pickDesc}${pickerTarget === 'model' ? labels.pickPrimary : labels.pickDraft}` : `${labels.pickDesc} ${pickerTarget === 'model' ? labels.pickPrimary : labels.pickDraft}.`}
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
