import type { InstanceConfig } from '../../store'
import { useState, useEffect } from 'react'
import { FolderOpen, Plus, X } from 'lucide-react'
import { Section, Input, Num, Switch, Select, SearchTarget, CollapsibleGroup, ResetButton, RESET_MAP, chatTemplates, specTypes, cacheTypes } from './shared'
import { KNOWN_FLAGS } from '../../validators'
import WorkerSelector from './WorkerSelector'
import { Button, TextInput } from '../ui'
import { getResettableFields, type ModelWorkload } from '../../modelPolicy'

const formGridClassName = 'config-form-grid'
const wideGridClassName = 'config-form-grid'

export const BASIC_CONFIG_KEYS: Array<keyof InstanceConfig> = [
  'model_path',
  'alias',
  'chat_template',
  'host',
  'port',
  'gpu_layers',
  'gpu_layers_auto',
  'ctx_size',
  'ctx_size_auto',
  'embedding',
  'pooling',
]

export const REASONING_CONFIG_KEYS: Array<keyof InstanceConfig> = [
  'reasoning',
  'spec_type',
  'draft_tokens',
  'spec_draft_n_min',
  'temp',
  'top_k',
  'top_p',
  'repeat_penalty',
  'n_predict',
  'ignore_eos',
  'reverse_prompt',
]

export const PERFORMANCE_CONFIG_KEYS: Array<keyof InstanceConfig> = [
  'threads',
  'threads_batch',
  'batch_size',
  'ubatch_size',
  'parallel',
  'cont_batching',
  'flash_attn',
  'mlock',
  'no_mmap',
  'no_repack',
  'numa_mode',
]

export const ADVANCED_GROUP_CONFIG_KEYS: Record<string, Array<keyof InstanceConfig>> = {
  advancedReasoningConfig: Object.keys(RESET_MAP.advancedReasoningConfig) as Array<keyof InstanceConfig>,
  advancedModelAdapt: [
    ...Object.keys(RESET_MAP.advancedModelAdapt),
    'lora_scaled',
  ] as Array<keyof InstanceConfig>,
  advancedSampling: Object.keys(RESET_MAP.advancedSampling) as Array<keyof InstanceConfig>,
  advancedSamplingExt: Object.keys(RESET_MAP.advancedSamplingExt) as Array<keyof InstanceConfig>,
  advancedSpec: Object.keys(RESET_MAP.advancedSpec) as Array<keyof InstanceConfig>,
  advancedRope: [
    ...Object.keys(RESET_MAP.advancedRope),
    'yarn_orig_ctx',
  ] as Array<keyof InstanceConfig>,
  advancedKvCache: [
    ...Object.keys(RESET_MAP.advancedKvCache),
    'no_kv_offload',
  ] as Array<keyof InstanceConfig>,
  advancedContextMgmt: Object.keys(RESET_MAP.advancedContextMgmt) as Array<keyof InstanceConfig>,
  advancedHardware: Object.keys(RESET_MAP.advancedHardware) as Array<keyof InstanceConfig>,
  advancedServerBasic: [
    ...Object.keys(RESET_MAP.advancedServerBasic),
    'offline',
  ] as Array<keyof InstanceConfig>,
  advancedServerExt: Object.keys(RESET_MAP.advancedServerExt) as Array<keyof InstanceConfig>,
  advancedMulti: [
    ...Object.keys(RESET_MAP.advancedMulti),
    'no_mmproj',
    'no_mmproj_offload',
    'mtmd_batch_max_tokens',
  ] as Array<keyof InstanceConfig>,
  customArgs: ['custom_args'],
}

export const ADVANCED_CONFIG_KEYS = Array.from(new Set(Object.values(ADVANCED_GROUP_CONFIG_KEYS).flat())) as Array<keyof InstanceConfig>

const countActive = (emittedParams: Set<keyof InstanceConfig>, keys: Array<keyof InstanceConfig>) =>
  keys.filter(key => emittedParams.has(key)).length

const countSummary = (emitted: number, changed: number, labels: Props['statusLabels']) =>
  emitted > 0 || changed > 0 ? (
    <span className="inline-flex items-center gap-2 text-[11px] text-slate-500">
      {changed > 0 && <span>{labels.changedMarker} {changed}</span>}
      {emitted > 0 && <span className="text-blue-400">{labels.emittedMarker} {emitted}</span>}
    </span>
  ) : undefined

interface Props {
  local: InstanceConfig
  set: (k: keyof InstanceConfig, v: any) => void
  inherit: (keys: Array<keyof InstanceConfig>) => void
  t: any
  isEmbedding: boolean
  workload: ModelWorkload
  modelWorkloadLocked: boolean
  onShowPicker?: () => void
  onCommitModelPath?: (modelPath: string) => void
  onShowDraftPicker?: () => void
  onShowMmprojPicker?: () => void
  emittedParams: Set<keyof InstanceConfig>
  changedParams: Set<keyof InstanceConfig>
  statusLabels: { changedMarker: string; emittedMarker: string }
  searchQuery: string
}

// ━━━━━━━━━━━━━━━━━━━━━━ COMMON SECTIONS ━━━━━━━━━━━━━━━━━━━━━━

export function BasicSection({ local, set, t, isEmbedding, workload, modelWorkloadLocked, onShowPicker, onCommitModelPath, emittedParams, changedParams, statusLabels, searchQuery }: Props) {
  const a = (k: keyof InstanceConfig) => k
  return (
    <Section id="config-basic" title={t.configPage.basic} defaultOpen={true} searchQuery={searchQuery} changedParams={changedParams} emittedParams={emittedParams} changedLabel={statusLabels.changedMarker} emittedLabel={statusLabels.emittedMarker} summary={countSummary(countActive(emittedParams, BASIC_CONFIG_KEYS), countActive(changedParams, BASIC_CONFIG_KEYS), statusLabels)}>
      <div className={formGridClassName}>
        <SearchTarget label={`${t.configPage.modelPath} (--model, -m)`} fieldKey="model_path" title={t.configPage.modelPathTip}>
          <div className="flex gap-1">
            <TextInput type="text" value={local.model_path} onChange={e => set('model_path', e.target.value)} onBlur={e => onCommitModelPath?.(e.target.value)} className="h-10 flex-1" />
            <Button onClick={onShowPicker} variant="primary" size="icon" title={t.configPage.modelPathBtn}><FolderOpen className="h-4 w-4" /></Button>
          </div>
        </SearchTarget>
        <Input label={`${t.configPage.alias} (--alias, -a)`} value={local.alias} onChange={v => set('alias', v)} title={t.configPage.aliasTip}  fieldKey={a('alias')} />
        {!isEmbedding && <Select label={`${t.configPage.chatTemplate} (--chat-template)`} value={local.chat_template} onChange={v => set('chat_template', v)} options={chatTemplates} title={t.configPage.chatTemplateTip} defaultLabel={t.common.default}  fieldKey={a('chat_template')} />}
        <Input label={`${t.configPage.host} (--host)`} value={local.host} onChange={v => set('host', v)} title={t.configPage.hostTip}  fieldKey={a('host')} />
        <Num label={`${t.configPage.portLabel} (--port)`} value={local.port} onChange={v => set('port', v)} min={1} max={65535} title={t.configPage.portLabelTip}  fieldKey={a('port')} />
        <Num label={`${t.configPage.gpuLayers} (--n-gpu-layers, -ngl)`} value={local.gpu_layers} onChange={v => set('gpu_layers', v)} min={0} title={t.configPage.gpuLayersTip} disabled={local.gpu_layers_auto}  fieldKey={a('gpu_layers')} />
        <Switch label={`${t.configPage.gpuLayersAuto}`} value={local.gpu_layers_auto} onChange={v => set('gpu_layers_auto', v)} title={t.configPage.gpuLayersAutoTip}  fieldKey={a('gpu_layers_auto')} />
        <Num label={`${t.configPage.ctxSize} (--ctx-size, -c)`} value={local.ctx_size} onChange={v => set('ctx_size', v)} min={0} step={1024} title={t.configPage.ctxSizeTip} disabled={local.ctx_size_auto}  fieldKey={a('ctx_size')} />
        <Switch label={`${t.configPage.ctxAuto}`} value={local.ctx_size_auto} onChange={v => set('ctx_size_auto', v)} title={t.configPage.ctxAutoTip}  fieldKey={a('ctx_size_auto')} />
        <Switch label={`${t.configPage.embedding} (--embedding)`} value={local.embedding} onChange={v => {
          set('embedding', v)
          if (!v) {
            set('reranking', false)
            set('pooling', '')
          }
        }} title={t.configPage.embeddingTip} disabled={modelWorkloadLocked} fieldKey={a('embedding')} />
        <Select label={`${t.configPage.pooling} (--pooling)`} value={local.pooling} onChange={v => set('pooling', v)} options={['', 'none', 'mean', 'cls', 'last', 'rank']} title={t.configPage.poolingTip} defaultLabel={t.common.default} disabled={workload === 'reranker'} fieldKey={a('pooling')} />
      </div>
    </Section>
  )
}

export function ReasoningSection({ local, set, t, isEmbedding, emittedParams, changedParams, statusLabels, searchQuery }: Props) {
  const a = (k: keyof InstanceConfig) => k
  return (
    <Section id="config-reasoning" title={t.configPage.reasoning} defaultOpen={true} searchQuery={searchQuery} changedParams={changedParams} emittedParams={emittedParams} changedLabel={statusLabels.changedMarker} emittedLabel={statusLabels.emittedMarker} summary={countSummary(countActive(emittedParams, REASONING_CONFIG_KEYS), countActive(changedParams, REASONING_CONFIG_KEYS), statusLabels)}>
      {(() => { const specActive = local.spec_type && local.spec_type !== 'none' && !isEmbedding; return (<>
      <div className={formGridClassName}>
        <Select label={`${t.configPage.reasoningSwitch} (--reasoning)`} value={local.reasoning} onChange={v => set('reasoning', v)} options={['', 'on', 'off', 'auto']} title={t.configPage.reasoningTip} disabled={isEmbedding} defaultLabel={t.common.default}  fieldKey={a('reasoning')} />
        <Select label={`${t.configPage.specType} (--spec-type)`} value={local.spec_type} onChange={v => set('spec_type', v)} options={specTypes} title={t.configPage.specTypeTip} disabled={isEmbedding} defaultLabel={t.common.default}  fieldKey={a('spec_type')} />
        <Num label={`${t.configPage.draftTokens} (--spec-draft-n-max)`} value={local.draft_tokens} onChange={v => set('draft_tokens', v)} min={0} title={t.configPage.draftTokensTip} disabled={!specActive}  fieldKey={a('draft_tokens')} />
        <Num label={`${t.configPage.specDraftNMin} (--spec-draft-n-min)`} value={local.spec_draft_n_min} onChange={v => set('spec_draft_n_min', v)} min={0} title={t.configPage.specDraftNMinTip} disabled={!specActive}  fieldKey={a('spec_draft_n_min')} />
        <Num label={`${t.configPage.temp} (--temp)`} value={local.temp} onChange={v => set('temp', v)} min={0} max={2} step={0.1} title={t.configPage.tempTip} disabled={isEmbedding}  fieldKey={a('temp')} />
        <Num label={`${t.configPage.topK} (--top-k)`} value={local.top_k} onChange={v => set('top_k', v)} min={0} title={t.configPage.topKTip} disabled={isEmbedding}  fieldKey={a('top_k')} />
        <Num label={`${t.configPage.topP} (--top-p)`} value={local.top_p} onChange={v => set('top_p', v)} min={0} max={1} step={0.1} title={t.configPage.topPTip} disabled={isEmbedding}  fieldKey={a('top_p')} />
        <Num label={`${t.configPage.repeatPenalty} (--repeat-penalty)`} value={local.repeat_penalty} onChange={v => set('repeat_penalty', v)} min={0} max={2} step={0.1} title={t.configPage.repeatPenaltyTip} disabled={isEmbedding}  fieldKey={a('repeat_penalty')} />
        <Num label={`${t.configPage.nPredict} (--n-predict, -n)`} value={local.n_predict} onChange={v => set('n_predict', v)} min={-1} title={t.configPage.nPredictTip} disabled={isEmbedding}  fieldKey={a('n_predict')} />
        <Switch label={`${t.configPage.ignoreEos} (--ignore-eos)`} value={local.ignore_eos} onChange={v => set('ignore_eos', v)} title={t.configPage.ignoreEosTip} disabled={isEmbedding}  fieldKey={a('ignore_eos')} />
        <Input label={`${t.configPage.reversePrompt} (--reverse-prompt, -r)`} value={local.reverse_prompt} onChange={v => set('reverse_prompt', v)} title={t.configPage.reversePromptTip} disabled={isEmbedding}  fieldKey={a('reverse_prompt')} />
      </div>
      </>); })()}
    </Section>
  )
}

export function PerformanceSection({ local, set, t, emittedParams, changedParams, statusLabels, searchQuery }: Props) {
  const a = (k: keyof InstanceConfig) => k
  return (
    <Section id="config-performance" title={t.configPage.performance} defaultOpen={true} searchQuery={searchQuery} changedParams={changedParams} emittedParams={emittedParams} changedLabel={statusLabels.changedMarker} emittedLabel={statusLabels.emittedMarker} summary={countSummary(countActive(emittedParams, PERFORMANCE_CONFIG_KEYS), countActive(changedParams, PERFORMANCE_CONFIG_KEYS), statusLabels)}>
      <div className={formGridClassName}>
        <Num label={`${t.configPage.threads} (--threads, -t)`} value={local.threads} onChange={v => set('threads', v)} min={0} title={t.configPage.threadsTip}  fieldKey={a('threads')} />
        <Num label={`${t.configPage.threadsBatch} (--threads-batch)`} value={local.threads_batch} onChange={v => set('threads_batch', v)} min={0} title={t.configPage.threadsBatchTip}  fieldKey={a('threads_batch')} />
        <Num label={`${t.configPage.batchSize} (--batch-size, -b)`} value={local.batch_size} onChange={v => set('batch_size', v)} min={1} title={t.configPage.batchSizeTip}  fieldKey={a('batch_size')} />
        <Num label={`${t.configPage.ubatchSize} (--ubatch-size, -ub)`} value={local.ubatch_size} onChange={v => set('ubatch_size', v)} min={1} title={t.configPage.ubatchSizeTip}  fieldKey={a('ubatch_size')} />
        <Num label={`${t.configPage.parallel} (--parallel, -np)`} value={local.parallel} onChange={v => set('parallel', v)} min={-1} title={t.configPage.parallelTip}  fieldKey={a('parallel')} />
        <Switch label={`${t.configPage.contBatching} (--cont-batching, -cb)`} value={local.cont_batching} onChange={v => set('cont_batching', v)} title={t.configPage.contBatchingTip}  fieldKey={a('cont_batching')} />
        <Select label={`${t.configPage.flashAttn} (--flash-attn, -fa)`} value={local.flash_attn} onChange={v => set('flash_attn', v)} options={['auto', 'on', 'off']} title={t.configPage.flashAttnTip} defaultLabel={t.common.default}  fieldKey={a('flash_attn')} />
        <Switch label={`${t.configPage.mlock} (--mlock)`} value={local.mlock} onChange={v => set('mlock', v)} title={t.configPage.mlockTip}  fieldKey={a('mlock')} />
        <Switch label={`${t.configPage.noMmap} (--no-mmap)`} value={local.no_mmap} onChange={v => set('no_mmap', v)} title={t.configPage.noMmapTip}  fieldKey={a('no_mmap')} />
      <Switch label={`${t.configPage.noRepack} (--no-repack)`} value={local.no_repack} onChange={v => set('no_repack', v)} title={t.configPage.noRepackTip}  fieldKey={a('no_repack')} />
        <Select label={`${t.configPage.numa} (--numa)`} value={local.numa_mode || (local.numa ? 'distribute' : '')} onChange={v => { set('numa_mode', v); set('numa', v === 'distribute') }} options={['', 'distribute', 'isolate', 'numactl']} title={t.configPage.numaTip} defaultLabel={t.common.default} fieldKey={a('numa_mode')} />
      </div>
    </Section>
  )
}

// ━━━━━━━━━━━━━━━━━━━━━━ ADVANCED CONTAINER ━━━━━━━━━━━━━━━━━━━━━━

export function AdvancedSection({ local, set, inherit, t, isEmbedding, modelWorkloadLocked, onShowDraftPicker, onShowMmprojPicker, emittedParams, changedParams, statusLabels, searchQuery }: Props) {
  const a = (k: keyof InstanceConfig) => k
  const summary = (keys: Array<keyof InstanceConfig>) => countSummary(countActive(emittedParams, keys), countActive(changedParams, keys), statusLabels)
  const resetAll = () => {
    inherit(getResettableFields(ADVANCED_CONFIG_KEYS, isEmbedding, modelWorkloadLocked))
  }

  const resetGroup = (id: string) => {
    const keys = ADVANCED_GROUP_CONFIG_KEYS[id]
    if (!keys) return

    inherit(getResettableFields(keys, isEmbedding, modelWorkloadLocked))
  }

  // ── 自定义参数：结构化键值对编辑器 ──
  // Parse existing custom_args into entries on mount
  const parseArgs = (args: string[]): { name: string; value: string }[] => {
    const entries: { name: string; value: string }[] = []
    for (let i = 0; i < args.length; i++) {
      if (args[i].startsWith('-')) {
        const candidate = args[i + 1] || ''
        const isNegativeNumber = /^-\d+(?:\.\d+)?$/.test(candidate)
        const next = (i + 1 < args.length && (!candidate.startsWith('-') || isNegativeNumber)) ? candidate : ''
        entries.push({ name: args[i], value: next })
        if (next) i++
      }
    }
    return entries
  }
  const serializeArgs = (entries: { name: string; value: string }[]): string[] => {
    const result: string[] = []
    for (const e of entries) {
      if (!e.name) continue
      result.push(e.name)
      if (e.value) result.push(e.value)
    }
    return result
  }
  const [entries, setEntries] = useState<{ name: string; value: string }[]>(() => parseArgs(local.custom_args))
  const [newName, setNewName] = useState('')
  const [newVal, setNewVal] = useState('')
  const customArgAction = t.modelRepo?.remove || 'Remove'

  useEffect(() => {
    setEntries(parseArgs(local.custom_args))
  }, [local.custom_args])

  const addEntry = () => {
    const name = newName.trim()
    if (!name) return
    const next = [...entries, { name, value: newVal.trim() }]
    setEntries(next)
    set('custom_args', serializeArgs(next))
    setNewName('')
    setNewVal('')
  }
  const removeEntry = (i: number) => {
    const next = entries.filter((_, idx) => idx !== i)
    setEntries(next)
    set('custom_args', serializeArgs(next))
  }
  const specActive = local.spec_type && local.spec_type !== 'none' && !isEmbedding

  return (
    <Section id="config-advanced" title={t.configPage.advSectionTitle} defaultOpen={true} searchQuery={searchQuery} changedParams={changedParams} emittedParams={emittedParams} changedLabel={statusLabels.changedMarker} emittedLabel={statusLabels.emittedMarker} summary={summary(ADVANCED_CONFIG_KEYS)}>
      <div className="flex items-center justify-end mb-2">
        <span className="mr-2 text-xs text-slate-500">{t.configPage.advSectionReset}</span>
        <ResetButton onClick={resetAll} title={t.configPage.advSectionReset} />
      </div>
      <div className="space-y-2">
        {/* 推理配置 (6) */}
        {!isEmbedding && (
        <CollapsibleGroup id="config-advanced-reasoning" title={t.configPage.subAdvReasoning} onReset={() => resetGroup('advancedReasoningConfig')} disabled={isEmbedding} summary={summary(ADVANCED_GROUP_CONFIG_KEYS.advancedReasoningConfig)}>
          <div className={formGridClassName}>
            <Select label={`${t.configPage.reasoningFormat} (--reasoning-format)`} value={local.reasoning_format} onChange={v => set('reasoning_format', v)} options={['', 'none', 'deepseek', 'deepseek-legacy']} title={t.configPage.reasoningFormatTip} defaultLabel={t.common.default}  fieldKey={a('reasoning_format')} />
            <Select label={`${t.configPage.reasoningEffort} (--chat-template-kwargs)`} value={local.reasoning_effort} onChange={v => set('reasoning_effort', v)} options={['', 'low', 'medium', 'high']} title={t.configPage.reasoningEffortTip} defaultLabel={t.common.default}  fieldKey={a('reasoning_effort')} />
            <Num label={`${t.configPage.reasoningBudget} (--reasoning-budget)`} value={local.reasoning_budget ? parseInt(local.reasoning_budget) : -1} onChange={v => set('reasoning_budget', v.toString())} min={-1} max={65536} step={256} title={t.configPage.reasoningBudgetTip} fieldKey={a('reasoning_budget')} />
            <Input label={`${t.configPage.reasoningBudgetMsg} (--reasoning-budget-message)`} value={local.reasoning_budget_message} onChange={v => set('reasoning_budget_message', v)} title={t.configPage.reasoningBudgetMsgTip}  fieldKey={a('reasoning_budget_message')} />
            <Select label={`${t.configPage.reasoningPreserve} (--reasoning-preserve)`} value={local.reasoning_preserve} onChange={v => set('reasoning_preserve', v)} options={['', 'on', 'off']} title={t.configPage.reasoningPreserveTip} defaultLabel={t.common.default} fieldKey={a('reasoning_preserve')} />
            <Switch label={`${t.configPage.jinja} (--jinja)`} value={local.jinja} onChange={v => set('jinja', v)} title={t.configPage.jinjaTip}  fieldKey={a('jinja')} />
            <Switch label={`${t.configPage.skipChatParsing} (--skip-chat-parsing)`} value={local.skip_chat_parsing} onChange={v => set('skip_chat_parsing', v)} title={t.configPage.skipChatParsingTip}  fieldKey={a('skip_chat_parsing')} />
          </div>
        </CollapsibleGroup>
        )}

        {/* 模型适配 (8) */}
        {!isEmbedding && (
        <CollapsibleGroup id="config-advanced-model" title={t.configPage.subAdvModelAdapt} onReset={() => resetGroup('advancedModelAdapt')} summary={summary(ADVANCED_GROUP_CONFIG_KEYS.advancedModelAdapt)}>
          <div className={formGridClassName}>
            <Input label={`${t.configPage.chatTemplateFile} (--chat-template-file)`} value={local.chat_template_file} onChange={v => set('chat_template_file', v)} title={t.configPage.chatTemplateFileTip}  fieldKey={a('chat_template_file')} />
            <Input label={`${t.configPage.lora} (--lora)`} value={local.lora_path} onChange={v => set('lora_path', v)} title={t.configPage.loraTip}  fieldKey={a('lora_path')} />
            <Switch label={`${t.configPage.loraInitNoApply} (--lora-init-without-apply)`} value={local.lora_init_without_apply} onChange={v => set('lora_init_without_apply', v)} title={t.configPage.loraInitNoApplyTip}  fieldKey={a('lora_init_without_apply')} />
      <Input label={`${t.configPage.loraScaled} (--lora-scaled)`} value={local.lora_scaled || ''} onChange={v => set('lora_scaled', v)} title={t.configPage.loraScaledTip} disabled={isEmbedding}  fieldKey={a('lora_scaled')} />
            <SearchTarget label={`${t.configPage.mmproj} (--mmproj)`} fieldKey={a('mmproj_path')} title={t.configPage.mmprojTip}>
              <div className="flex gap-1">
                <TextInput type="text" value={local.mmproj_path} onChange={e => set('mmproj_path', e.target.value)} disabled={isEmbedding} className="h-10 flex-1" />
                <Button onClick={onShowMmprojPicker} variant="primary" size="icon" title={t.configPage.mmprojPathBtn} aria-label={t.configPage.mmprojPathBtn} disabled={isEmbedding}>
                  <FolderOpen className="h-4 w-4" />
                </Button>
              </div>
            </SearchTarget>
            <Input label={`${t.configPage.grammarFile} (--grammar-file)`} value={local.grammar_file} onChange={v => set('grammar_file', v)} title={t.configPage.grammarFileTip}  fieldKey={a('grammar_file')} />
            <Input label={`${t.configPage.grammar} (--grammar)`} value={local.grammar} onChange={v => set('grammar', v)} title={t.configPage.grammarTip}  fieldKey={a('grammar')} />
            <Num label={`${t.configPage.embdNormalize} (--embd-normalize)`} value={local.embd_normalize} onChange={v => set('embd_normalize', v)} min={-1} title={t.configPage.embdNormalizeTip}  fieldKey={a('embd_normalize')} />
            <Switch label={`${t.configPage.reranking} (--reranking)`} value={local.reranking} onChange={v => set('reranking', v)} title={t.configPage.rerankingTip}  fieldKey={a('reranking')} />
          </div>
        </CollapsibleGroup>
        )}

        {isEmbedding && (
          <CollapsibleGroup id="config-advanced-vector" title={t.configPage.subEmbedding} defaultOpen={true} summary={summary(['embd_normalize', 'reranking'])}>
            <div className={formGridClassName}>
              <Num label={`${t.configPage.embdNormalize} (--embd-normalize)`} value={local.embd_normalize} onChange={v => set('embd_normalize', v)} min={-1} title={t.configPage.embdNormalizeTip} fieldKey={a('embd_normalize')} />
              <Switch label={`${t.configPage.reranking} (--reranking)`} value={local.reranking} onChange={v => {
                set('reranking', v)
                set('pooling', v ? 'rank' : '')
              }} title={t.configPage.rerankingTip} disabled={modelWorkloadLocked} fieldKey={a('reranking')} />
            </div>
          </CollapsibleGroup>
        )}

        {/* 高级采样 (19) */}
        {!isEmbedding && (
        <CollapsibleGroup id="config-advanced-sampling" title={t.configPage.subAdvSampling} onReset={() => resetGroup('advancedSampling')} disabled={isEmbedding} summary={summary(ADVANCED_GROUP_CONFIG_KEYS.advancedSampling)}>
          <div className={formGridClassName}>
            <Select label={`${t.configPage.mirostat} (--mirostat)`} value={local.mirostat.toString()} onChange={v => set('mirostat', parseInt(v))} options={['0', '1', '2']} title={t.configPage.mirostatTip} defaultLabel={t.common.default} fieldKey={a('mirostat')} />
            <Num label={`${t.configPage.mirostatLr} (--mirostat-lr)`} value={local.mirostat_lr} onChange={v => set('mirostat_lr', v)} min={0} max={1} step={0.001} title={t.configPage.mirostatLrTip}  fieldKey={a('mirostat_lr')} />
            <Num label={`${t.configPage.mirostatEnt} (--mirostat-ent)`} value={local.mirostat_ent} onChange={v => set('mirostat_ent', v)} min={0} max={10} step={0.1} title={t.configPage.mirostatEntTip}  fieldKey={a('mirostat_ent')} />
            <Num label={`${t.configPage.xtcProbability} (--xtc-probability)`} value={local.xtc_probability} onChange={v => set('xtc_probability', v)} min={0} max={1} step={0.05} title={t.configPage.xtcProbabilityTip}  fieldKey={a('xtc_probability')} />
            <Num label={`${t.configPage.xtcThreshold} (--xtc-threshold)`} value={local.xtc_threshold} onChange={v => set('xtc_threshold', v)} min={0} max={1} step={0.05} title={t.configPage.xtcThresholdTip}  fieldKey={a('xtc_threshold')} />
            <Num label={`${t.configPage.dynatempRange} (--dynatemp-range)`} value={local.dynatemp_range} onChange={v => set('dynatemp_range', v)} min={0} max={10} step={0.1} title={t.configPage.dynatempRangeTip}  fieldKey={a('dynatemp_range')} />
            <Num label={`${t.configPage.dynatempExp} (--dynatemp-exp)`} value={local.dynatemp_exp} onChange={v => set('dynatemp_exp', v)} min={0} max={10} step={0.1} title={t.configPage.dynatempExpTip}  fieldKey={a('dynatemp_exp')} />
            <Num label={`${t.configPage.typicalP} (--typical-p)`} value={local.typical_p} onChange={v => set('typical_p', v)} min={0} max={1} step={0.05} title={t.configPage.typicalPTip}  fieldKey={a('typical_p')} />
            <Num label={`${t.configPage.dryMultiplier} (--dry-multiplier)`} value={local.dry_multiplier} onChange={v => set('dry_multiplier', v)} min={0} max={10} step={0.1} title={t.configPage.dryMultiplierTip}  fieldKey={a('dry_multiplier')} />
            <Num label={`${t.configPage.dryBase} (--dry-base)`} value={local.dry_base} onChange={v => set('dry_base', v)} min={0} max={10} step={0.1} title={t.configPage.dryBaseTip}  fieldKey={a('dry_base')} />
            <Num label={`${t.configPage.dryAllowedLength} (--dry-allowed-length)`} value={local.dry_allowed_length} onChange={v => set('dry_allowed_length', v)} min={0} step={1} title={t.configPage.dryAllowedLengthTip}  fieldKey={a('dry_allowed_length')} />
            <Num label={`${t.configPage.dryPenaltyLastN} (--dry-penalty-last-n)`} value={local.dry_penalty_last_n} onChange={v => set('dry_penalty_last_n', v)} min={-1} title={t.configPage.dryPenaltyLastNTip}  fieldKey={a('dry_penalty_last_n')} />
            <Input label={`${t.configPage.drySeqBreaker} (--dry-sequence-breaker)`} value={local.dry_sequence_breaker} onChange={v => set('dry_sequence_breaker', v)} title={t.configPage.drySeqBreakerTip}  fieldKey={a('dry_sequence_breaker')} />
            <Num label={`${t.configPage.adaptiveTarget} (--adaptive-target)`} value={local.adaptive_target} onChange={v => set('adaptive_target', v)} min={-1} max={1} step={0.1} title={t.configPage.adaptiveTargetTip}  fieldKey={a('adaptive_target')} />
            <Num label={`${t.configPage.adaptiveDecay} (--adaptive-decay)`} value={local.adaptive_decay} onChange={v => set('adaptive_decay', v)} min={0} max={1} step={0.01} title={t.configPage.adaptiveDecayTip}  fieldKey={a('adaptive_decay')} />
            <Num label={`${t.configPage.topNSigma} (--top-n-sigma)`} value={local.top_n_sigma} onChange={v => set('top_n_sigma', v)} min={-1} max={10} step={0.1} title={t.configPage.topNSigmaTip}  fieldKey={a('top_n_sigma')} />
            <Input label={`${t.configPage.logitBias} (--logit-bias, -l)`} value={local.logit_bias} onChange={v => set('logit_bias', v)} title={t.configPage.logitBiasTip}  fieldKey={a('logit_bias')} />
            <Input label={`${t.configPage.samplers} (--samplers)`} value={local.samplers} onChange={v => set('samplers', v)} title={t.configPage.samplersTip}  fieldKey={a('samplers')} />
            <Input label={`${t.configPage.samplerSeq} (--sampler-seq)`} value={local.sampler_seq} onChange={v => set('sampler_seq', v)} title={t.configPage.samplerSeqTip}  fieldKey={a('sampler_seq')} />
          </div>
        </CollapsibleGroup>
        )}

        {/* 采样参数扩展 (9) */}
        {!isEmbedding && (
        <CollapsibleGroup id="config-advanced-sampling-ext" title={t.configPage.subAdvSamplingExt} onReset={() => resetGroup('advancedSamplingExt')} disabled={isEmbedding} summary={summary(ADVANCED_GROUP_CONFIG_KEYS.advancedSamplingExt)}>
          <div className={formGridClassName}>
            <Num label={`${t.configPage.seed} (--seed)`} value={local.seed} onChange={v => set('seed', v)} min={-1} title={t.configPage.seedTip}  fieldKey={a('seed')} />
            <Num label={`${t.configPage.minP} (--min-p)`} value={local.min_p} onChange={v => set('min_p', v)} min={0} max={1} step={0.05} title={t.configPage.minPTip}  fieldKey={a('min_p')} />
            <Num label={`${t.configPage.presencePenalty} (--presence-penalty)`} value={local.presence_penalty} onChange={v => set('presence_penalty', v)} min={-2} max={2} step={0.1} title={t.configPage.presencePenaltyTip}  fieldKey={a('presence_penalty')} />
            <Num label={`${t.configPage.frequencyPenalty} (--frequency-penalty)`} value={local.frequency_penalty} onChange={v => set('frequency_penalty', v)} min={-2} max={2} step={0.1} title={t.configPage.frequencyPenaltyTip}  fieldKey={a('frequency_penalty')} />
            <Num label={`${t.configPage.repeatLastN} (--repeat-last-n)`} value={local.repeat_last_n} onChange={v => set('repeat_last_n', v)} min={-1} title={t.configPage.repeatLastNTip}  fieldKey={a('repeat_last_n')} />
            <Switch label={`${t.configPage.special} (--special, -sp)`} value={local.special} onChange={v => set('special', v)} title={t.configPage.specialTip}  fieldKey={a('special')} />
            <Switch label={`${t.configPage.spmInfill} (--spm-infill)`} value={local.spm_infill} onChange={v => set('spm_infill', v)} title={t.configPage.spmInfillTip}  fieldKey={a('spm_infill')} />
            <Switch label={`${t.configPage.backendSampling} (--sampling-backend, -bs)`} value={local.backend_sampling} onChange={v => set('backend_sampling', v)} title={t.configPage.backendSamplingTip}  fieldKey={a('backend_sampling')} />
            <Input label={`${t.configPage.jsonSchema} (--json-schema)`} value={local.json_schema} onChange={v => set('json_schema', v)} title={t.configPage.jsonSchemaTip}  fieldKey={a('json_schema')} />
            <Input label={`${t.configPage.jsonSchemaFile} (--json-schema-file, -jf)`} value={local.json_schema_file} onChange={v => set('json_schema_file', v)} title={t.configPage.jsonSchemaFileTip}  fieldKey={a('json_schema_file')} />
          </div>
        </CollapsibleGroup>
        )}

        {/* 推测解码 (9) */}
        {!isEmbedding && (
        <CollapsibleGroup id="config-advanced-spec" title={t.configPage.subAdvSpec} onReset={() => resetGroup('advancedSpec')} disabled={!specActive} summary={summary(ADVANCED_GROUP_CONFIG_KEYS.advancedSpec)}>
          {!specActive && <div className="mb-2 rounded-lg border border-slate-800 bg-slate-950/60 px-3 py-2 text-xs text-slate-500">{t.configPage.specDisabled}</div>}
          <div className={formGridClassName}>
            <SearchTarget label={`${t.configPage.draftModel} (--draft-model, -md)`} fieldKey="draft_model_path" title={t.configPage.draftModelTip}>
              <div className="flex gap-1">
                <TextInput type="text" value={local.draft_model_path} onChange={e => set('draft_model_path', e.target.value)} disabled={isEmbedding} className="h-10 flex-1" />
                <Button onClick={onShowDraftPicker} disabled={isEmbedding} variant="primary" size="icon" title={t.configPage.draftModelTip}><FolderOpen className="h-4 w-4" /></Button>
              </div>
            </SearchTarget>
            <Num label={`${t.configPage.draftGpu} (--draft-n-gpu-layers, -ngld)`} value={local.draft_gpu_layers} onChange={v => set('draft_gpu_layers', v)} min={0} title={t.configPage.draftGpuTip} disabled={isEmbedding}  fieldKey={a('draft_gpu_layers')} />
            <Num label={`${t.configPage.specDraftPMin} (--spec-draft-p-min)`} value={local.spec_draft_p_min} onChange={v => set('spec_draft_p_min', v)} min={0} max={1} step={0.05} title={t.configPage.specDraftPMinTip} disabled={isEmbedding}  fieldKey={a('spec_draft_p_min')} />
            <Num label={`${t.configPage.specDraftPSplit} (--spec-draft-p-split)`} value={local.spec_draft_p_split} onChange={v => set('spec_draft_p_split', v)} min={0} max={1} step={0.05} title={t.configPage.specDraftPSplitTip} disabled={isEmbedding}  fieldKey={a('spec_draft_p_split')} />
            <Input label={`${t.configPage.specDraftDevice} (--spec-draft-device)`} value={local.spec_draft_device} onChange={v => set('spec_draft_device', v)} title={t.configPage.specDraftDeviceTip} disabled={isEmbedding}  fieldKey={a('spec_draft_device')} />
            <Input label={`${t.configPage.lookupCacheStatic} (--lookup-cache-static, -lcs)`} value={local.lookup_cache_static} onChange={v => set('lookup_cache_static', v)} title={t.configPage.lookupCacheStaticTip} disabled={isEmbedding}  fieldKey={a('lookup_cache_static')} />
            <Input label={`${t.configPage.lookupCacheDynamic} (--lookup-cache-dynamic, -lcd)`} value={local.lookup_cache_dynamic} onChange={v => set('lookup_cache_dynamic', v)} title={t.configPage.lookupCacheDynamicTip} disabled={isEmbedding}  fieldKey={a('lookup_cache_dynamic')} />
            <Select label={`${t.configPage.cacheTypeDraftK} (--cache-type-draft-k, -ctkd)`} value={local.cache_type_draft_k} onChange={v => set('cache_type_draft_k', v)} options={cacheTypes} title={t.configPage.cacheTypeDraftKTip} defaultLabel={t.common.default}  fieldKey={a('cache_type_draft_k')} />
            <Select label={`${t.configPage.cacheTypeDraftV} (--cache-type-draft-v, -ctvd)`} value={local.cache_type_draft_v} onChange={v => set('cache_type_draft_v', v)} options={cacheTypes} title={t.configPage.cacheTypeDraftVTip} defaultLabel={t.common.default}  fieldKey={a('cache_type_draft_v')} />
            <Switch label={`${t.configPage.specDefault} (--spec-default)`} value={local.spec_default} onChange={v => set('spec_default', v)} title={t.configPage.specDefaultTip} disabled={isEmbedding}  fieldKey={a('spec_default')} />
            <Switch label={`${t.configPage.specDraftBackendSampling} (--no-spec-draft-backend-sampling)`} value={!local.spec_draft_backend_sampling} onChange={v => set('spec_draft_backend_sampling', !v)} title={t.configPage.specDraftBackendSamplingTip} disabled={isEmbedding} fieldKey={a('spec_draft_backend_sampling')} />
            <Num label={`${t.configPage.specDraftThreads} (--spec-draft-threads, -td)`} value={local.spec_draft_threads} onChange={v => set('spec_draft_threads', v)} min={0} title={t.configPage.specDraftThreadsTip} disabled={isEmbedding}  fieldKey={a('spec_draft_threads')} />
            <Num label={`${t.configPage.specDraftThreadsBatch} (--spec-draft-threads-batch, -tbd)`} value={local.spec_draft_threads_batch} onChange={v => set('spec_draft_threads_batch', v)} min={0} title={t.configPage.specDraftThreadsBatchTip} disabled={isEmbedding}  fieldKey={a('spec_draft_threads_batch')} />
          </div>
        </CollapsibleGroup>
        )}

        {/* 上下文缩放 / RoPE · YaRN (8) */}
        <CollapsibleGroup id="config-advanced-rope" title={t.configPage.subAdvRope} onReset={() => resetGroup('advancedRope')} summary={summary(ADVANCED_GROUP_CONFIG_KEYS.advancedRope)}>
          <div className={formGridClassName}>
            <Select label={`${t.configPage.ropeScaling} (--rope-scaling)`} value={local.rope_scaling} onChange={v => set('rope_scaling', v)} options={['', 'none', 'linear', 'yarn']} title={t.configPage.ropeScalingTip} defaultLabel={t.common.default}  fieldKey={a('rope_scaling')} />
            <Num label={`${t.configPage.ropeScale} (--rope-scale)`} value={local.rope_scale} onChange={v => set('rope_scale', v)} min={0} max={10} step={0.1} title={t.configPage.ropeScaleTip}  fieldKey={a('rope_scale')} />
            <Num label={`${t.configPage.ropeFreqBase} (--rope-freq-base)`} value={local.rope_freq_base} onChange={v => set('rope_freq_base', v)} min={0} title={t.configPage.ropeFreqBaseTip}  fieldKey={a('rope_freq_base')} />
            <Num label={`${t.configPage.ropeFreqScale} (--rope-freq-scale)`} value={local.rope_freq_scale} onChange={v => set('rope_freq_scale', v)} min={0} max={10} step={0.1} title={t.configPage.ropeFreqScaleTip}  fieldKey={a('rope_freq_scale')} />
            <Num label={`${t.configPage.yarnExtFactor} (--yarn-ext-factor)`} value={local.yarn_ext_factor} onChange={v => set('yarn_ext_factor', v)} min={-1} max={10} step={0.1} title={t.configPage.yarnExtFactorTip}  fieldKey={a('yarn_ext_factor')} />
            <Num label={`${t.configPage.yarnAttnFactor} (--yarn-attn-factor)`} value={local.yarn_attn_factor} onChange={v => set('yarn_attn_factor', v)} min={-1} max={10} step={0.1} title={t.configPage.yarnAttnFactorTip}  fieldKey={a('yarn_attn_factor')} />
            <Num label={`${t.configPage.yarnBetaSlow} (--yarn-beta-slow)`} value={local.yarn_beta_slow} onChange={v => set('yarn_beta_slow', v)} min={0} max={10} step={0.1} title={t.configPage.yarnBetaSlowTip}  fieldKey={a('yarn_beta_slow')} />
            <Num label={`${t.configPage.yarnBetaFast} (--yarn-beta-fast)`} value={local.yarn_beta_fast} onChange={v => set('yarn_beta_fast', v)} min={-1} max={128} title={t.configPage.yarnBetaFastTip}  fieldKey={a('yarn_beta_fast')} />
      <Num label={`${t.configPage.yarnOrigCtx} (--yarn-orig-ctx)`} value={local.yarn_orig_ctx || 0} onChange={v => set('yarn_orig_ctx', v)} min={0} max={1048576} title={t.configPage.yarnOrigCtxTip}  fieldKey={a('yarn_orig_ctx')} />
          </div>
        </CollapsibleGroup>

        {/* KV 缓存 (8) */}
        <CollapsibleGroup id="config-advanced-kv" title={t.configPage.subAdvKvCache} onReset={() => resetGroup('advancedKvCache')} summary={summary(ADVANCED_GROUP_CONFIG_KEYS.advancedKvCache)}>
          <div className={formGridClassName}>
            <Select label={`${t.configPage.cacheTypeK} (--cache-type-k, -ctk)`} value={local.cache_type_k} onChange={v => set('cache_type_k', v)} options={cacheTypes} title={t.configPage.cacheTypeKTip} defaultLabel={t.common.default}  fieldKey={a('cache_type_k')} />
            <Select label={`${t.configPage.cacheTypeV} (--cache-type-v, -ctv)`} value={local.cache_type_v} onChange={v => set('cache_type_v', v)} options={cacheTypes} title={t.configPage.cacheTypeVTip} defaultLabel={t.common.default}  fieldKey={a('cache_type_v')} />
            {!isEmbedding && (<>
            <Switch label={`${t.configPage.cachePrompt} (--no-cache-prompt)`} value={!local.cache_prompt} onChange={v => set('cache_prompt', !v)} title={t.configPage.cachePromptTip} fieldKey={a('cache_prompt')} />
            <Num label={`${t.configPage.cacheReuse} (--cache-reuse)`} value={local.cache_reuse} onChange={v => set('cache_reuse', v)} min={0} title={t.configPage.cacheReuseTip}  fieldKey={a('cache_reuse')} />
            <Num label={`${t.configPage.cacheRam} (--cache-ram, -cram)`} value={local.cache_ram} onChange={v => set('cache_ram', v)} min={-1} step={256} title={t.configPage.cacheRamTip}  fieldKey={a('cache_ram')} />
            </>)}
            <Switch label={`${t.configPage.warmup} (--warmup)`} value={local.warmup} onChange={v => set('warmup', v)} title={t.configPage.warmupTip}  fieldKey={a('warmup')} />
            <Switch label={`${t.configPage.cacheIdleSlots} (--no-cache-idle-slots)`} value={!local.cache_idle_slots} onChange={v => set('cache_idle_slots', !v)} title={t.configPage.cacheIdleSlotsTip} fieldKey={a('cache_idle_slots')} />
            <Select label={`${t.configPage.kvUnified} (--kv-unified)`} value={local.kv_unified_mode || (local.kv_unified ? 'on' : '')} onChange={v => { set('kv_unified_mode', v); set('kv_unified', v === 'on') }} options={['', 'on', 'off']} title={t.configPage.kvUnifiedTip} defaultLabel={t.common.default} fieldKey={a('kv_unified_mode')} />
      <Switch label={`${t.configPage.noKvOffload} (--no-kv-offload)`} value={local.no_kv_offload} onChange={v => set('no_kv_offload', v)} title={t.configPage.noKvOffloadTip}  fieldKey={a('no_kv_offload')} />
          </div>
        </CollapsibleGroup>

        {/* 上下文管理 (6) */}
        <CollapsibleGroup id="config-advanced-context" title={t.configPage.subAdvContextMgmt} onReset={() => resetGroup('advancedContextMgmt')} summary={summary(ADVANCED_GROUP_CONFIG_KEYS.advancedContextMgmt)}>
          <div className={formGridClassName}>
            {!isEmbedding && (<>
            <Num label={`${t.configPage.ctxCheckpoints} (--ctx-checkpoints, -ctxcp)`} value={local.ctx_checkpoints} onChange={v => set('ctx_checkpoints', v)} min={0} title={t.configPage.ctxCheckpointsTip}  fieldKey={a('ctx_checkpoints')} />
            <Num label={`${t.configPage.checkpointMinStep} (--checkpoint-min-step, -cms)`} value={local.checkpoint_min_step} onChange={v => set('checkpoint_min_step', v)} min={0} title={t.configPage.checkpointMinStepTip}  fieldKey={a('checkpoint_min_step')} />
            <Switch label={`${t.configPage.contextShift} (--context-shift)`} value={local.context_shift} onChange={v => set('context_shift', v)} title={t.configPage.contextShiftTip} disabled={isEmbedding}  fieldKey={a('context_shift')} />
            <Switch label={`${t.configPage.swaFull} (--swa-full)`} value={local.swa_full} onChange={v => set('swa_full', v)} title={t.configPage.swaFullTip}  fieldKey={a('swa_full')} />
            <Num label={`${t.configPage.keep} (--keep)`} value={local.keep} onChange={v => set('keep', v)} min={-1} title={t.configPage.keepTip}  fieldKey={a('keep')} />
            </>)}
            <Input label={`${t.configPage.overrideKv} (--override-kv)`} value={local.override_kv} onChange={v => set('override_kv', v)} title={t.configPage.overrideKvTip}  fieldKey={a('override_kv')} />
          </div>
        </CollapsibleGroup>

        {/* 硬件配置 (9) */}
        <CollapsibleGroup id="config-advanced-hardware" title={t.configPage.subAdvHardware} onReset={() => resetGroup('advancedHardware')} summary={summary(ADVANCED_GROUP_CONFIG_KEYS.advancedHardware)}>
          <div className={formGridClassName}>
            <Num label={`${t.configPage.moeCpu} (--n-cpu-moe)`} value={local.moe_cpu_layers} onChange={v => set('moe_cpu_layers', v)} min={0} title={t.configPage.moeCpuTip} fieldKey={a('moe_cpu_layers')} />
            <Switch label={`${t.configPage.cpuMoe} (--cpu-moe)`} value={local.cpu_moe} onChange={v => set('cpu_moe', v)} title={t.configPage.cpuMoeTip}  fieldKey={a('cpu_moe')} />
            <Input label={`${t.configPage.device} (--device, -dev)`} value={local.device} onChange={v => set('device', v)} title={t.configPage.deviceTip}  fieldKey={a('device')} />
            <Select label={`${t.configPage.splitMode} (--split-mode, -sm)`} value={local.split_mode} onChange={v => set('split_mode', v)} options={['', 'none', 'layer', 'row', 'tensor']} title={t.configPage.splitModeTip} defaultLabel={t.common.default}  fieldKey={a('split_mode')} />
            <Input label={`${t.configPage.tensorSplit} (--tensor-split, -ts)`} value={local.tensor_split} onChange={v => set('tensor_split', v)} title={t.configPage.tensorSplitTip}  fieldKey={a('tensor_split')} />
            <Num label={`${t.configPage.mainGpu} (--main-gpu, -mg)`} value={local.main_gpu} onChange={v => set('main_gpu', v)} min={0} title={t.configPage.mainGpuTip}  fieldKey={a('main_gpu')} />
            <Switch label={`${t.configPage.perf} (--perf)`} value={local.perf} onChange={v => set('perf', v)} title={t.configPage.perfTip}  fieldKey={a('perf')} />
            <Switch label={`${t.configPage.checkTensors} (--check-tensors)`} value={local.check_tensors} onChange={v => set('check_tensors', v)} title={t.configPage.checkTensorsTip}  fieldKey={a('check_tensors')} />
            <Switch label={`${t.configPage.directIo} (--direct-io)`} value={local.direct_io} onChange={v => set('direct_io', v)} title={t.configPage.directIoTip} fieldKey={a('direct_io')} />
            <Select label={`${t.configPage.fit} (--fit)`} value={local.fit_mode || (local.fit ? 'on' : '')} onChange={v => { set('fit_mode', v); set('fit', v === 'on') }} options={['', 'on', 'off']} title={t.configPage.fitTip} defaultLabel={t.common.default} fieldKey={a('fit_mode')} />
            <Input label={`${t.configPage.fitTarget} (--fit-target, -fitt)`} value={local.fit_target} onChange={v => set('fit_target', v)} title={t.configPage.fitTargetTip} disabled={(local.fit_mode || (local.fit ? 'on' : '')) === 'off'}  fieldKey={a('fit_target')} />
            <Num label={`${t.configPage.fitCtx} (--fit-ctx, -fitc)`} value={local.fit_ctx} onChange={v => set('fit_ctx', v)} min={0} title={t.configPage.fitCtxTip} disabled={(local.fit_mode || (local.fit ? 'on' : '')) === 'off'}  fieldKey={a('fit_ctx')} />
            <Num label={`${t.configPage.threadsHttp} (--threads-http)`} value={local.threads_http} onChange={v => set('threads_http', v)} min={-1} title={t.configPage.threadsHttpTip}  fieldKey={a('threads_http')} />
          </div>
        </CollapsibleGroup>

        {/* 服务基础 (10) */}
        <CollapsibleGroup id="config-advanced-server" title={t.configPage.subAdvServer} onReset={() => resetGroup('advancedServerBasic')} summary={summary(ADVANCED_GROUP_CONFIG_KEYS.advancedServerBasic)}>
          <div className={formGridClassName}>
            <Input label={`${t.configPage.apiKey} (--api-key)`} value={local.api_key} onChange={v => set('api_key', v)} type="password" title={t.configPage.apiKeyTip}  fieldKey={a('api_key')} />
            <Input label={`${t.configPage.apiKeyFile} (--api-key-file)`} value={local.api_key_file} onChange={v => set('api_key_file', v)} title={t.configPage.apiKeyFileTip}  fieldKey={a('api_key_file')} />
            <Switch label={`${t.configPage.noUi} (--no-ui)`} value={local.no_ui} onChange={v => set('no_ui', v)} title={t.configPage.noUiTip}  fieldKey={a('no_ui')} />
      <Switch label={`${t.configPage.offline} (--offline)`} value={local.offline} onChange={v => set('offline', v)} title={t.configPage.offlineTip}  fieldKey={a('offline')} />
            <Input label={`${t.configPage.pathPrefix} (--path)`} value={local.path_prefix} onChange={v => set('path_prefix', v)} title={t.configPage.pathPrefixTip}  fieldKey={a('path_prefix')} />
            <Input label={`${t.configPage.apiPrefix} (--api-prefix)`} value={local.api_prefix} onChange={v => set('api_prefix', v)} title={t.configPage.apiPrefixTip}  fieldKey={a('api_prefix')} />
            <Input label={`${t.configPage.corsOrigins} (--cors-origins)`} value={local.cors_origins} onChange={v => set('cors_origins', v)} title={t.configPage.corsOriginsTip} fieldKey={a('cors_origins')} />
            <Input label={`${t.configPage.corsMethods} (--cors-methods)`} value={local.cors_methods} onChange={v => set('cors_methods', v)} title={t.configPage.corsMethodsTip} fieldKey={a('cors_methods')} />
            <Input label={`${t.configPage.corsHeaders} (--cors-headers)`} value={local.cors_headers} onChange={v => set('cors_headers', v)} title={t.configPage.corsHeadersTip} fieldKey={a('cors_headers')} />
            <Select label={`${t.configPage.corsCredentials} (--cors-credentials)`} value={local.cors_credentials} onChange={v => set('cors_credentials', v)} options={['', 'on', 'off']} title={t.configPage.corsCredentialsTip} defaultLabel={t.common.default} fieldKey={a('cors_credentials')} />
            <Num label={`${t.configPage.timeout} (--timeout, -to)`} value={local.timeout} onChange={v => set('timeout', v)} min={1} title={t.configPage.timeoutTip}  fieldKey={a('timeout')} />
            <Num label={`${t.configPage.sleepIdle} (--sleep-idle-seconds)`} value={local.sleep_idle} onChange={v => set('sleep_idle', v)} min={-1} title={t.configPage.sleepIdleTip}  fieldKey={a('sleep_idle')} />
            <Switch label={`${t.configPage.verbose} (--verbose, -v)`} value={local.verbose} onChange={v => set('verbose', v)} title={t.configPage.verboseTip}  fieldKey={a('verbose')} />
          </div>
          <div className={`${wideGridClassName} mt-3`}>
            <Input label={`${t.configPage.sslKey} (--ssl-key-file)`} value={local.ssl_key_file} onChange={v => set('ssl_key_file', v)} title={t.configPage.sslKeyTip}  fieldKey={a('ssl_key_file')} />
            <Input label={`${t.configPage.sslCert} (--ssl-cert-file)`} value={local.ssl_cert_file} onChange={v => set('ssl_cert_file', v)} title={t.configPage.sslCertTip}  fieldKey={a('ssl_cert_file')} />
          </div>
        </CollapsibleGroup>

        {/* 服务扩展 (7) */}
        <CollapsibleGroup id="config-advanced-server-ext" title={t.configPage.subAdvServerExt} onReset={() => resetGroup('advancedServerExt')} summary={summary(ADVANCED_GROUP_CONFIG_KEYS.advancedServerExt)}>
          <div className={formGridClassName}>
            <Switch label={`${t.configPage.slotsEnabled} (--no-slots)`} value={!local.slots_enabled} onChange={v => set('slots_enabled', !v)} title={t.configPage.slotsEnabledTip} fieldKey={a('slots_enabled')} />
            <Switch label={`${t.configPage.metrics} (--metrics)`} value={local.metrics} onChange={v => set('metrics', v)} title={t.configPage.metricsTip}  fieldKey={a('metrics')} />
            <Switch label={`${t.configPage.props} (--props)`} value={local.props} onChange={v => set('props', v)} title={t.configPage.propsTip}  fieldKey={a('props')} />
            {!isEmbedding && (<>
            <Input label={`${t.configPage.slotSavePath} (--slot-save-path)`} value={local.slot_save_path} onChange={v => set('slot_save_path', v)} title={t.configPage.slotSavePathTip}  fieldKey={a('slot_save_path')} />
            <Input label={`${t.configPage.logPromptsDir} (--log-prompts-dir)`} value={local.log_prompts_dir} onChange={v => set('log_prompts_dir', v)} title={t.configPage.logPromptsDirTip} fieldKey={a('log_prompts_dir')} />
            <Num label={`${t.configPage.slotPromptSimilarity} (--slot-prompt-similarity, -sps)`} value={local.slot_prompt_similarity} onChange={v => set('slot_prompt_similarity', v)} min={0} max={1} step={0.05} title={t.configPage.slotPromptSimilarityTip}  fieldKey={a('slot_prompt_similarity')} />
            <Switch label={`${t.configPage.prefillAssistant} (--prefill-assistant)`} value={local.prefill_assistant} onChange={v => set('prefill_assistant', v)} title={t.configPage.prefillAssistantTip}  fieldKey={a('prefill_assistant')} />
            <Input label={`${t.configPage.uiConfigFile} (--ui-config-file)`} value={local.ui_config_file} onChange={v => set('ui_config_file', v)} title={t.configPage.uiConfigFileTip}  fieldKey={a('ui_config_file')} />
            <Input label={`${t.configPage.uiConfig} (--ui-config)`} value={local.ui_config} onChange={v => set('ui_config', v)} title={t.configPage.uiConfigTip}  fieldKey={a('ui_config')} />
            <Switch label={`${t.configPage.uiMcpProxy} (--ui-mcp-proxy)`} value={local.ui_mcp_proxy} onChange={v => set('ui_mcp_proxy', v)} title={t.configPage.uiMcpProxyTip}  fieldKey={a('ui_mcp_proxy')} />
            <Switch label={`${t.configPage.agent} (--agent)`} value={local.agent} onChange={v => set('agent', v)} title={t.configPage.agentTip}  fieldKey={a('agent')} />
            </>)}
          </div>
          <div className={`${formGridClassName} mt-3`}>
            <SearchTarget label={`${t.configPage.rpcServers} (--rpc)`} fieldKey="rpc_servers" className="config-grid-span-full">
              <WorkerSelector value={local.rpc_servers} onChange={v => set('rpc_servers', v)} t={t} hideLabel />
            </SearchTarget>
            <Num label={`${t.configPage.ssePingInterval} (--sse-ping-interval)`} value={local.sse_ping_interval} onChange={v => set('sse_ping_interval', v)} min={-1} title={t.configPage.ssePingIntervalTip}  fieldKey={a('sse_ping_interval')} />
            <Switch label={`${t.configPage.reusePort} (--reuse-port)`} value={local.reuse_port} onChange={v => set('reuse_port', v)} title={t.configPage.reusePortTip}  fieldKey={a('reuse_port')} />
          </div>
        </CollapsibleGroup>

        {/* 多模型/专家 (11) */}
        {!isEmbedding && (
        <CollapsibleGroup id="config-advanced-multi" title={t.configPage.subAdvMulti} onReset={() => resetGroup('advancedMulti')} summary={summary(ADVANCED_GROUP_CONFIG_KEYS.advancedMulti)}>
          <div className={formGridClassName}>
            <Input label={`${t.configPage.modelsDir} (--models-dir)`} value={local.models_dir} onChange={v => set('models_dir', v)} title={t.configPage.modelsDirTip}  fieldKey={a('models_dir')} />
            <Input label={`${t.configPage.modelsPreset} (--models-preset)`} value={local.models_preset} onChange={v => set('models_preset', v)} title={t.configPage.modelsPresetTip}  fieldKey={a('models_preset')} />
            <Num label={`${t.configPage.modelsMax} (--models-max)`} value={local.models_max} onChange={v => set('models_max', v)} min={0} title={t.configPage.modelsMaxTip}  fieldKey={a('models_max')} />
            <Switch label={`${t.configPage.modelsAutoload} (--models-autoload)`} value={local.models_autoload} onChange={v => set('models_autoload', v)} title={t.configPage.modelsAutoloadTip}  fieldKey={a('models_autoload')} />
            <Input label={`${t.configPage.mmprojUrl} (--mmproj-url)`} value={local.mmproj_url} onChange={v => set('mmproj_url', v)} title={t.configPage.mmprojUrlTip} disabled={isEmbedding}  fieldKey={a('mmproj_url')} />
            <Select label={`${t.configPage.mmprojAuto} (--mmproj-auto)`} value={local.mmproj_mode || (local.no_mmproj ? 'off' : local.mmproj_auto ? 'on' : '')} onChange={v => { set('mmproj_mode', v); set('mmproj_auto', v === 'on'); set('no_mmproj', v === 'off') }} options={['', 'on', 'off']} title={t.configPage.mmprojAutoTip} defaultLabel={t.common.default} disabled={isEmbedding} fieldKey={a('mmproj_mode')} />
      <Switch label={`${t.configPage.noMmprojOffload} (--no-mmproj-offload)`} value={local.no_mmproj_offload} onChange={v => set('no_mmproj_offload', v)} title={t.configPage.noMmprojOffloadTip} disabled={isEmbedding || (local.mmproj_mode || (local.no_mmproj ? 'off' : local.mmproj_auto ? 'on' : '')) === 'off'}  fieldKey={a('no_mmproj_offload')} />
            <Num label={`${t.configPage.imageMinTokens} (--image-min-tokens)`} value={local.image_min_tokens} onChange={v => set('image_min_tokens', v)} min={0} title={t.configPage.imageMinTokensTip} disabled={isEmbedding}  fieldKey={a('image_min_tokens')} />
            <Num label={`${t.configPage.imageMaxTokens} (--image-max-tokens)`} value={local.image_max_tokens} onChange={v => set('image_max_tokens', v)} min={0} title={t.configPage.imageMaxTokensTip} disabled={isEmbedding}  fieldKey={a('image_max_tokens')} />
            <Num label={`${t.configPage.mtmdBatchMaxTokens} (--mtmd-batch-max-tokens)`} value={local.mtmd_batch_max_tokens} onChange={v => set('mtmd_batch_max_tokens', v)} min={256} step={256} title={t.configPage.mtmdBatchMaxTokensTip} disabled={isEmbedding}  fieldKey={a('mtmd_batch_max_tokens')} />
            <Input label={`${t.configPage.mediaPath} (--media-path)`} value={local.media_path} onChange={v => set('media_path', v)} title={t.configPage.mediaPathTip}  fieldKey={a('media_path')} />
            <Input label={`${t.configPage.tags} (--tags)`} value={local.tags} onChange={v => set('tags', v)} title={t.configPage.tagsTip}  fieldKey={a('tags')} />
            <Input label={`${t.configPage.tools} (--tools)`} value={local.tools} onChange={v => set('tools', v)} title={t.configPage.toolsTip}  fieldKey={a('tools')} />
          </div>
        </CollapsibleGroup>
        )}

        <CollapsibleGroup
          id="config-advanced-custom"
          title={t.configPage.customArgs}
          defaultOpen={entries.length > 0}
          summary={summary(ADVANCED_GROUP_CONFIG_KEYS.customArgs)}
          fieldKey="custom_args"
        >
          <div className="space-y-2">
            <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_36px] gap-2 px-1 text-[11px] font-medium uppercase tracking-[0.12em] text-slate-500">
              <span>{t.configPage.customArgName}</span>
              <span>{t.configPage.customArgValue}</span>
              <span className="text-right">{customArgAction}</span>
            </div>
            {entries.length === 0 && (
              <div className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-xs text-slate-500 dark:border-slate-800 dark:bg-slate-950/50 dark:text-slate-500">
                {t.configPage.customArgEmpty}
              </div>
            )}
            {entries.map((e, i) => (
              <div key={i} className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_28px] items-center gap-2">
                <span className="min-w-0 truncate rounded-lg border border-slate-200 bg-white px-2 py-1 font-mono text-xs text-slate-800 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-200" title={e.name}>{e.name}</span>
                <span className="min-w-0 truncate rounded-lg border border-slate-200 bg-white px-2 py-1 font-mono text-xs text-slate-500 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-400" title={e.value}>
                  {e.value || '-'}
                </span>
                <Button onClick={() => removeEntry(i)} variant="danger" size="icon" className="h-7 w-7 shrink-0" title={customArgAction} aria-label={customArgAction}><X className="w-3.5 h-3.5"/></Button>
              </div>
            ))}
            <div className="flex items-center gap-2">
              <TextInput
                type="text"
                value={newName}
                onChange={e => setNewName(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter') addEntry() }}
                placeholder={t.configPage.customArgName}
                className="h-9 flex-1 text-xs"
              />
              <TextInput
                type="text"
                value={newVal}
                onChange={e => setNewVal(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter') addEntry() }}
                placeholder={t.configPage.customArgValue}
                className="h-9 flex-1 text-xs"
              />
              <Button onClick={addEntry} variant="primary" size="icon" className="h-9 w-9 shrink-0" title={t.configPage.customArgAdd}><Plus className="w-4 h-4"/></Button>
            </div>
            {newName.trim().startsWith('-') && KNOWN_FLAGS.has(newName.trim()) && (
              <div className="rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-700 dark:border-red-500/20 dark:bg-red-500/10 dark:text-red-200">! {t.configPage.warnD1}</div>
            )}
          </div>
        </CollapsibleGroup>
      </div>
    </Section>
  )
}
