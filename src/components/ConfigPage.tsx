import { useEffect, useMemo, useRef, useState } from 'react'
import { AlertTriangle, CheckCircle2, ChevronDown, ChevronRight, Cpu, File, FolderOpen, Image, Search, Settings, SlidersHorizontal, Sparkles, X } from 'lucide-react'
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
import { Badge, Button, EmptyState, InsetSurface, MetricCard, SectionHeader, Surface, TextInput } from './ui'

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

const ConfigPage = () => {
  const { instances, activeConfigInstanceId, updateInstance, saveConfig, models, modelDirs, engines, defaultEngineId } = useAppStore()
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
      <div className="flex-1 overflow-y-auto p-6">
        <EmptyState icon={<Settings className="h-10 w-10" />} title={t.configPage.title} description={t.configPage.noInstance} />
      </div>
    )
  }

  const activeParams = getActiveParams(local, isEmbedding)
  const currentModel = models.find(model => model.path === local.model_path) ?? null
  const currentEngine = engines.find(engine => engine.id === (local.engine_id || defaultEngineId || '')) ?? engines[0] ?? null
  const warningCounts = {
    high: saveWarnings.filter(warning => warning.severity === 'high').length,
    medium: saveWarnings.filter(warning => warning.severity === 'medium').length,
    low: saveWarnings.filter(warning => warning.severity === 'low').length,
  }

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
    embeddingMode: zh ? '\u5d4c\u5165\u6a21\u5f0f' : 'Embedding mode',
    modifiedParams: zh ? '\u5df2\u4fee\u6539\u53c2\u6570' : 'Modified params',
    validationSummary: zh ? '\u6821\u9a8c\u6458\u8981' : 'Validation Summary',
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
        { id: 'config-advanced-custom', title: (t.configPage as any).customArgs || 'Custom arguments', count: countActive(activeParams, ADVANCED_GROUP_CONFIG_KEYS.customArgs) },
      ],
    },
  ]

  const scrollToSection = (id: string) => {
    document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="mb-6 flex flex-col gap-5 xl:flex-row xl:items-end xl:justify-between">
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <div className="rounded-lg border border-blue-500/20 bg-blue-500/10 p-3 text-blue-300">
              <SlidersHorizontal className="h-5 w-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-3xl font-semibold tracking-tight text-slate-50">{t.configPage.title}</h1>
                <Badge tone="slate">
                  {inst?.name}
                </Badge>
                {isEmbedding && (
                  <Badge tone="blue">
                    Embedding
                  </Badge>
                )}
              </div>
              <p className="text-sm text-slate-400">{labels.subtitle}</p>
            </div>
          </div>
        </div>

        <Button
          onClick={save}
          disabled={!inst}
          variant="primary"
          size="lg"
          data-guide="config-save"
          icon={saved ? <CheckCircle2 className="h-4 w-4" /> : <Sparkles className="h-4 w-4" />}
        >
          {saved ? t.configPage.saved : t.configPage.save}
        </Button>
      </div>

      <div className="mb-6 grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          { label: labels.activeParams, value: activeParams.size, icon: SlidersHorizontal, tone: 'text-sky-300 bg-sky-500/10 border-sky-500/20' },
          { label: labels.warnings, value: saveWarnings.length, icon: AlertTriangle, tone: 'text-amber-300 bg-amber-500/10 border-amber-500/20' },
          { label: labels.model, value: currentModel ? pathBasename(currentModel.path) : '--', icon: File, tone: 'text-fuchsia-300 bg-fuchsia-500/10 border-fuchsia-500/20' },
          { label: labels.engine, value: currentEngine?.name || '--', icon: Cpu, tone: 'text-emerald-300 bg-emerald-500/10 border-emerald-500/20' },
        ].map(card => (
          <MetricCard key={card.label} label={card.label} value={card.value} icon={<card.icon className="h-5 w-5" />} tone={card.tone} valueClassName="text-2xl" />
        ))}
      </div>

      <div className="grid gap-6 xl:grid-cols-[240px,minmax(0,1fr),320px]">
        <Surface as="aside" className="h-fit p-4 xl:sticky xl:top-6">
          <SectionHeader title={labels.parameterGroups} />
          <nav className="mt-4 space-y-1">
            {directoryGroups.map(group => (
              <div key={group.id}>
                <button
                  type="button"
                  onClick={() => scrollToSection(group.id)}
                  className="flex w-full items-center justify-between gap-3 rounded-lg px-3 py-2 text-left text-sm text-slate-200 transition hover:bg-slate-800"
                >
                  <span className="min-w-0 truncate">{group.title}</span>
                  {group.count > 0 && <Badge tone="emerald" className="shrink-0 px-2 py-0.5">{group.count}</Badge>}
                </button>
                {'children' in group && group.children && (
                  <div className="mt-1 space-y-1 border-l border-slate-800 pl-3">
                    {group.children.map(child => (
                      <button
                        key={child.id}
                        type="button"
                        onClick={() => scrollToSection(child.id)}
                        className="flex w-full items-center justify-between gap-2 rounded-lg px-2 py-1.5 text-left text-xs text-slate-500 transition hover:bg-slate-800 hover:text-slate-200"
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
            <div className="flex items-start gap-3 rounded-lg border border-emerald-500/20 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-200">
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

        <Surface as="aside" className="h-fit p-5 xl:sticky xl:top-6">
          <div className="mb-5">
            <SectionHeader title={labels.configContext} description={labels.configContextDesc} />
          </div>

          <div className="space-y-4">
            <InsetSurface className="p-4">
              <div className="flex items-start gap-3">
                <div className="rounded-lg border border-slate-700 bg-slate-900 p-3 text-slate-300">
                  <Settings className="h-5 w-5" />
                </div>
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium text-slate-100">{inst?.name}</p>
                  <p className="mt-1 text-xs text-slate-500">Port {local.port} - {local.host || '127.0.0.1'}</p>
                </div>
              </div>
            </InsetSurface>

            <InsetSurface className="space-y-3 p-4">
              {[
                [labels.primaryModel, currentModel ? pathBasename(currentModel.path) : '--'],
                [labels.draftModel, local.draft_model_path ? pathBasename(local.draft_model_path) : '--'],
                [labels.engine, currentEngine?.name || '--'],
                [labels.embeddingMode, isEmbedding ? labels.on : labels.off],
                [labels.modifiedParams, String(activeParams.size)],
              ].map(([label, value]) => (
                <div key={label} className="flex items-center justify-between gap-3">
                  <span className="text-sm text-slate-500">{label}</span>
                  <span className="max-w-[180px] truncate text-right text-sm text-slate-200" title={value}>
                    {value}
                  </span>
                </div>
              ))}
            </InsetSurface>

            <InsetSurface className="p-4">
              <p className="text-sm font-medium text-slate-100">{labels.validationSummary}</p>
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

              {saveWarnings.length > 0 && (
                <div className="mt-4 space-y-2">
                  {saveWarnings.slice(0, 6).map((warning, index) => (
                    <div
                      key={`${warning.key}-${index}`}
                      className={`rounded-lg px-3 py-2 text-sm ${
                        warning.severity === 'high'
                          ? 'bg-red-500/10 text-red-200'
                          : warning.severity === 'medium'
                            ? 'bg-amber-500/10 text-amber-200'
                            : 'bg-sky-500/10 text-sky-200'
                      }`}
                    >
                      {(t.configPage as any)[warning.key] || warning.key}
                    </div>
                  ))}
                </div>
              )}
            </InsetSurface>
          </div>
        </Surface>
      </div>

      {showPicker && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
          <div className="flex max-h-[85vh] w-full max-w-3xl flex-col overflow-hidden rounded-lg border border-slate-800 bg-slate-900 shadow-[0_30px_80px_rgba(2,6,23,0.7)]">
            <div className="flex items-center justify-between border-b border-slate-800 bg-slate-950/90 px-6 py-4">
              <div>
                <h3 className="text-lg font-semibold text-slate-50">{t.modelRepo.selectFromRepo}</h3>
                <p className="mt-1 text-sm text-slate-400">
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

            <div className="flex-1 overflow-y-auto p-4">
              <div className="space-y-2 rounded-lg border border-slate-800 bg-slate-950/40 p-3">
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
                            <span className="truncate">{node.name}</span>
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
