import { useEffect, useMemo, useRef, useState } from 'react'
import { Activity, AlertTriangle, CheckCircle2, ChevronDown, ChevronRight, Cpu, File, FolderOpen, Image, Search, Settings, SlidersHorizontal, Sparkles, X } from 'lucide-react'
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
import { normalizePath, pathBasename, pathDirname, pathJoin } from '../utils/path'
import { _matchedElements } from './ConfigPage/shared'
import { Badge, Button, EmptyState, InsetSurface, MetricCard, PathText, SectionHeader, Surface, TextInput } from './ui'

const EMBED_ARCHS = ['bge', 'gte', 'e5', 'text-embedding', 'sentence-bert', 'sentence-t5', 'instructor', 'bert', 'nomic', 'jina']

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
  const normalizedRootLower = normalizedRoot.toLowerCase()

  for (const model of models) {
    const normalizedPath = normalizePath(model.path).toLowerCase()
    if (!normalizedPath.startsWith(normalizedRootLower)) {
      continue
    }

    const relative = normalizePath(model.path.substring(rootDir.length)).replace(/^\/+/, '')
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
  description: string
  tone: string
  changes: Partial<InstanceConfig>
  risks: string[]
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
      before: formatValue(baseline[key], labels),
      after: formatValue(local[key], labels),
    }))

const getTemplateChanges = (local: InstanceConfig, changes: Partial<InstanceConfig>, t: any, labels: Record<string, string>): ConfigChange[] =>
  (Object.keys(changes) as Array<keyof InstanceConfig>)
    .filter(key => !isEqualValue(local[key], changes[key]))
    .map(key => ({
      key,
      label: fieldLabel(key, t),
      before: formatValue(local[key], labels),
      after: formatValue(changes[key], labels),
    }))

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
  const [searchQuery, setSearchQuery] = useState('')
  const [appliedTemplateId, setAppliedTemplateId] = useState<string | null>(null)
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
  }, [activeConfigInstanceId, inst])

  const set = (key: keyof InstanceConfig, value: any) => {
    setAppliedTemplateId(null)
    setLocal(current => (current ? { ...current, [key]: value } : current))
  }

  const isEmbedding = useMemo(() => {
    if (!local?.model_path) {
      return false
    }

    const fileName = pathBasename(local.model_path)
    if (fileName.toLowerCase().includes('embed')) {
      return true
    }

    const model = models.find(item => item.path === local.model_path)
    return !!(model?.architecture && EMBED_ARCHS.some(arch => model.architecture!.toLowerCase().includes(arch)))
  }, [local?.model_path, models])

  useEffect(() => {
    if (isEmbedding && local) {
      if (!local.embedding) {
        set('embedding', true)
      }
      if (!local.pooling) {
        set('pooling', 'mean')
      }
    }
  }, [isEmbedding, local?.embedding, local?.model_path, local?.pooling])

  if (!local) {
    return (
      <div className="space-y-5">
        <EmptyState icon={<Settings className="h-10 w-10" />} title={t.configPage.title} description={t.configPage.noInstance} />
      </div>
    )
  }

  const activeParams = getActiveParams(local, isEmbedding)
  const currentModel = models.find(model => model.path === local.model_path) ?? null
  const currentEngine = engines.find(engine => engine.id === (local.engine_id || defaultEngineId || '')) ?? engines[0] ?? null
  const primaryModelPath = currentModel?.path || local.model_path || ''
  const draftModelPath = local.draft_model_path || ''
  const endpoint = `${local.host || '127.0.0.1'}:${local.port}`

  const pickModel = (modelPath: string) => {
    if (pickerTarget === 'model') {
      set('model_path', modelPath)
      const directory = pathDirname(modelPath)
      const mmproj = models.find(model => pathDirname(model.path) === directory && model.file_type === 'mmproj')
      set('mmproj_path', mmproj?.path ?? '')
    } else {
      set('draft_model_path', modelPath)
    }
    setShowPicker(false)
  }

  const save = async () => {
    if (!inst) {
      return
    }

    const model = models.find(item => item.path === local.model_path)
    const engine = engines.find(item => item.id === (local.engine_id || defaultEngineId || '')) || engines[0]
    const warnings = validateConfig(local, model, engine)

    updateInstance(inst.id, { config: local })
    await saveConfig()
    setSaved(true)
    setSaveWarnings(warnings)

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
    quickTemplatesDesc: zh ? '\u5e94\u7528\u5230\u672c\u5730\u8349\u7a3f\uff0c\u5148\u5bf9\u6bd4\u5dee\u5f02\u548c\u98ce\u9669\uff0c\u786e\u8ba4\u540e\u518d\u4fdd\u5b58\u5230\u5b9e\u4f8b\u3002' : 'Apply to the local draft, review the diff and risk notes, then save when ready.',
    applyTemplate: zh ? '\u5e94\u7528\u5230\u8349\u7a3f' : 'Apply to Draft',
    appliedTemplate: zh ? '\u5df2\u5e94\u7528' : 'Applied',
    templateDiff: zh ? '\u5c06\u4fee\u6539' : 'Changes',
    templateNoDiff: zh ? '\u5f53\u524d\u8349\u7a3f\u5df2\u7b26\u5408\u8be5\u9884\u8bbe\u3002' : 'The draft already matches this preset.',
    templateRisks: zh ? '\u98ce\u9669' : 'Risks',
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

  const savedBaseline = { ...defaultInstanceConfig(), ...(inst?.config ?? {}) }
  const configChanges = getConfigChanges(local, savedBaseline, t, labels)
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
      id: 'conservative',
      title: zh ? '\u4fdd\u5b88\u7a33\u5b9a' : 'Conservative',
      description: zh ? '\u4f18\u5148\u542f\u52a8\u6210\u529f\u548c\u7a33\u5b9a\u54cd\u5e94\uff0c\u964d\u4f4e\u6279\u5904\u7406\u4e0e\u5e76\u53d1\u538b\u529b\u3002' : 'Prioritizes reliable startup and steady responses with modest batching and concurrency.',
      tone: 'border-emerald-500/25 bg-emerald-500/10 text-emerald-200',
      changes: { ctx_size_auto: false, ctx_size: 4096, gpu_layers_auto: true, batch_size: 1024, ubatch_size: 256, parallel: 1, flash_attn: 'auto', cont_batching: true, cache_ram: 4096, metrics: true, props: true, slots_enabled: true },
      risks: zh
        ? ['\u541e\u5410\u4f1a\u4f4e\u4e8e\u6fc0\u8fdb\u5e76\u53d1\u914d\u7f6e\u3002', '\u56fa\u5b9a 4K \u4e0a\u4e0b\u6587\u53ef\u80fd\u4e0d\u9002\u5408\u957f\u6587\u6863\u4efb\u52a1\u3002']
        : ['Throughput will be lower than aggressive concurrent configs.', 'A fixed 4K context may not fit long-document workloads.'],
    },
    {
      id: 'throughput',
      title: zh ? '\u9ad8\u541e\u5410' : 'Throughput',
      description: zh ? '\u63d0\u9ad8 batch\u3001parallel \u548c Flash Attention\uff0c\u9002\u5408\u591a\u8bf7\u6c42 API \u670d\u52a1\u3002' : 'Raises batch size, parallel slots, and Flash Attention for heavier API concurrency.',
      tone: 'border-blue-500/25 bg-blue-500/10 text-blue-200',
      changes: { ctx_size_auto: true, gpu_layers_auto: true, batch_size: 4096, ubatch_size: 1024, parallel: 4, cont_batching: true, flash_attn: 'on', cache_ram: 8192, metrics: true, props: true, slots_enabled: true },
      risks: zh
        ? ['\u9700\u8981\u66f4\u591a\u663e\u5b58\u548c KV cache \u7a7a\u95f4\u3002', 'CPU \u6216\u4e0d\u652f\u6301 Flash Attention \u7684\u540e\u7aef\u53ef\u80fd\u51fa\u73b0\u8b66\u544a\u6216\u964d\u901f\u3002']
        : ['Needs more VRAM and KV cache headroom.', 'CPU or non-Flash-Attention backends may warn or slow down.'],
    },
    {
      id: 'long-context',
      title: zh ? '\u957f\u4e0a\u4e0b\u6587' : 'Long Context',
      description: zh ? '\u6269\u5927\u4e0a\u4e0b\u6587\u548c\u7f13\u5b58\u9884\u7559\uff0c\u9762\u5411\u957f\u6587\u6863\u6216\u591a\u8f6e\u5bf9\u8bdd\u3002' : 'Expands context and cache headroom for long documents or extended conversations.',
      tone: 'border-violet-500/25 bg-violet-500/10 text-violet-200',
      changes: { ctx_size_auto: false, ctx_size: 32768, batch_size: 2048, ubatch_size: 512, parallel: 1, cache_ram: 16384, ctx_checkpoints: 64, flash_attn: 'auto', context_shift: true, metrics: true, props: true },
      risks: zh
        ? ['KV cache \u4f1a\u663e\u8457\u589e\u52a0\u5185\u5b58 / \u663e\u5b58\u538b\u529b\u3002', '\u8d85\u8fc7\u6a21\u578b\u539f\u751f\u4e0a\u4e0b\u6587\u65f6\uff0c\u8d28\u91cf\u53ef\u80fd\u4e0b\u964d\u3002']
        : ['KV cache can substantially increase RAM or VRAM pressure.', 'Quality may degrade beyond the model native context.'],
    },
    {
      id: 'low-vram',
      title: zh ? '\u4f4e\u663e\u5b58' : 'Low VRAM',
      description: zh ? '\u5c3d\u91cf\u964d\u4f4e GPU \u5360\u7528\u548c\u6279\u5904\u7406\u89c4\u6a21\uff0c\u4fbf\u4e8e\u8d44\u6e90\u7d27\u5f20\u65f6\u5148\u8dd1\u8d77\u6765\u3002' : 'Reduces GPU pressure and batch size so constrained machines can start first.',
      tone: 'border-amber-500/25 bg-amber-500/10 text-amber-200',
      changes: { ctx_size_auto: false, ctx_size: 4096, gpu_layers_auto: false, gpu_layers: 0, batch_size: 512, ubatch_size: 128, parallel: 1, cache_ram: 0, mlock: false, flash_attn: 'off', no_kv_offload: false, metrics: true },
      risks: zh
        ? ['CPU \u8d1f\u8f7d\u4f1a\u589e\u52a0\uff0c\u751f\u6210\u901f\u5ea6\u53ef\u80fd\u660e\u663e\u53d8\u6162\u3002', '\u4ec5\u4f7f\u7528 4K \u4e0a\u4e0b\u6587\uff0c\u4e0d\u9002\u5408\u957f\u8f93\u5165\u3002']
        : ['CPU load will rise and generation can become much slower.', 'Uses only a 4K context, so long prompts are not a good fit.'],
    },
  ]

  const applyTemplate = (template: ConfigTemplate) => {
    setLocal(current => (current ? { ...current, ...template.changes } : current))
    setAppliedTemplateId(template.id)
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
            <div className="rounded-lg border border-blue-500/20 bg-blue-500/10 px-4 py-3 text-sm text-blue-200">
              {t.configPage.embeddingBanner}
            </div>
          )}

          <Surface className="p-5" data-guide="config-presets">
            <div className="mb-4">
              <SectionHeader title={labels.quickTemplates} description={labels.quickTemplatesDesc} />
            </div>
            <div className="grid gap-3 xl:grid-cols-2">
              {quickTemplates.map(template => {
                const previewChanges = getTemplateChanges(local, template.changes, t, labels)
                const isApplied = appliedTemplateId === template.id
                return (
                  <button
                    key={template.id}
                    type="button"
                    onClick={() => applyTemplate(template)}
                    className={`flex min-h-[220px] flex-col justify-between rounded-lg border bg-white p-4 text-left transition hover:border-blue-300 hover:bg-blue-50 dark:bg-slate-950/40 dark:hover:border-blue-500/40 dark:hover:bg-blue-500/10 ${
                      isApplied ? 'border-blue-400 ring-1 ring-blue-400/40 dark:border-blue-500/60' : 'border-slate-200 dark:border-slate-800'
                    }`}
                  >
                    <span>
                      <span className="flex items-start justify-between gap-3">
                        <span className="min-w-0">
                          <span className="block text-sm font-medium text-slate-900 dark:text-slate-100">{template.title}</span>
                          <span className="mt-2 block text-sm leading-5 text-slate-500 dark:text-slate-400">{template.description}</span>
                        </span>
                        <span className={`shrink-0 rounded-full border px-2.5 py-1 text-[11px] font-medium ${template.tone}`}>
                          {isApplied ? labels.appliedTemplate : labels.applyTemplate}
                        </span>
                      </span>

                      <span className="mt-4 block rounded-lg border border-slate-200 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-950/70">
                        <span className="mb-2 block text-xs font-medium uppercase tracking-[0.12em] text-slate-500">{labels.templateDiff}</span>
                        {previewChanges.length === 0 ? (
                          <span className="block text-xs leading-5 text-slate-500">{labels.templateNoDiff}</span>
                        ) : (
                          <span className="block space-y-1">
                            {previewChanges.slice(0, 4).map(change => (
                              <span key={change.key} className="grid min-w-0 grid-cols-[92px_minmax(0,1fr)] gap-2 text-xs">
                                <span className="truncate text-slate-500" title={change.label}>{change.label}</span>
                                <span className="min-w-0 truncate text-slate-700 dark:text-slate-200" title={`${change.before} -> ${change.after}`}>
                                  {change.before} {'->'} {change.after}
                                </span>
                              </span>
                            ))}
                            {previewChanges.length > 4 && (
                              <span className="block text-xs text-slate-500">
                                {zh ? `${labels.moreChanges} ${previewChanges.length - 4} ${labels.moreChangesSuffix}` : `${labels.moreChanges} ${previewChanges.length - 4} ${labels.moreChangesSuffix}`}
                              </span>
                            )}
                          </span>
                        )}
                      </span>

                      <span className="mt-3 block">
                        <span className="mb-1 block text-xs font-medium uppercase tracking-[0.12em] text-slate-500">{labels.templateRisks}</span>
                        <span className="block space-y-1">
                          {template.risks.map(risk => (
                            <span key={risk} className="block text-xs leading-5 text-slate-500 dark:text-slate-400">
                              {risk}
                            </span>
                          ))}
                        </span>
                      </span>
                    </span>

                    <span className="mt-4 inline-flex items-center gap-2 text-xs font-medium text-blue-300">
                      <Sparkles className="h-3.5 w-3.5" />
                      {labels.applyTemplate}
                    </span>
                  </button>
                )
              })}
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
