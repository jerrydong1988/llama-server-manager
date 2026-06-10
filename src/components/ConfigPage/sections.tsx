import type { InstanceConfig } from '../../store'
import { Section, Input, Num, Switch, Select, CollapsibleGroup, ResetButton, RESET_MAP, chatTemplates, specTypes, cacheTypes } from './shared'
import WorkerSelector from './WorkerSelector'

interface Props {
  local: InstanceConfig
  set: (k: keyof InstanceConfig, v: any) => void
  t: any
  isEmbedding: boolean
  onShowPicker?: () => void
  onShowDraftPicker?: () => void
  activeParams: Set<keyof InstanceConfig>
}

// ━━━━━━━━━━━━━━━━━━━━━━ COMMON SECTIONS ━━━━━━━━━━━━━━━━━━━━━━

export function BasicSection({ local, set, t, onShowPicker, activeParams }: Props) {
  const a = (k: keyof InstanceConfig) => activeParams.has(k)
  return (
    <Section title={t.configPage.basic} defaultOpen={true}>
      <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
        <div title={t.configPage.modelPathTip}>
          <label className="block text-xs font-medium mb-1 text-gray-500">{`${t.configPage.modelPath} (-m)`}</label>
          <div className="flex gap-1">
            <input type="text" value={local.model_path} onChange={e => set('model_path', e.target.value)} className="flex-1 px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
            <button onClick={onShowPicker} className="px-2 py-1.5 bg-blue-600 hover:bg-blue-700 text-white rounded-lg text-xs" title={t.configPage.modelPathBtn}>{'\uD83D\uDCC2'}</button>
          </div>
        </div>
        <Input label={`${t.configPage.alias} (-a)`} value={local.alias} onChange={v => set('alias', v)} title={t.configPage.aliasTip}  active={a('alias')} />
        <Select label={`${t.configPage.chatTemplate} (--chat-template)`} value={local.chat_template} onChange={v => set('chat_template', v)} options={chatTemplates} title={t.configPage.chatTemplateTip} defaultLabel={t.common.default}  active={a('chat_template')} />
        <Input label={`${t.configPage.host} (--host)`} value={local.host} onChange={v => set('host', v)} title={t.configPage.hostTip}  active={a('host')} />
        <Num label={`${t.configPage.portLabel} (--port)`} value={local.port} onChange={v => set('port', v)} min={1} max={65535} title={t.configPage.portLabelTip}  active={a('port')} />
        <Num label={`${t.configPage.gpuLayers} (-ngl)`} value={local.gpu_layers} onChange={v => set('gpu_layers', v)} min={0} max={99} title={t.configPage.gpuLayersTip} disabled={local.gpu_layers_auto}  active={a('gpu_layers')} />
        <Switch label={`${t.configPage.gpuLayersAuto}`} value={local.gpu_layers_auto} onChange={v => set('gpu_layers_auto', v)} title={t.configPage.gpuLayersAutoTip}  active={a('gpu_layers_auto')} />
        <Num label={`${t.configPage.ctxSize} (-c)`} value={local.ctx_size} onChange={v => set('ctx_size', v)} min={0} step={1024} title={t.configPage.ctxSizeTip} disabled={local.ctx_size_auto}  active={a('ctx_size')} />
        <Switch label={`${t.configPage.ctxAuto}`} value={local.ctx_size_auto} onChange={v => set('ctx_size_auto', v)} title={t.configPage.ctxAutoTip}  active={a('ctx_size_auto')} />
        <Switch label={`${t.configPage.embedding} (--embedding)`} value={local.embedding} onChange={v => set('embedding', v)} title={t.configPage.embeddingTip}  active={a('embedding')} />
        <Select label={`${t.configPage.pooling} (--pooling)`} value={local.pooling} onChange={v => set('pooling', v)} options={['', 'none', 'mean', 'cls', 'last', 'rank']} title={t.configPage.poolingTip} defaultLabel={t.common.default}  active={a('pooling')} />
      </div>
    </Section>
  )
}

export function ReasoningSection({ local, set, t, isEmbedding, activeParams }: Props) {
  const a = (k: keyof InstanceConfig) => activeParams.has(k)
  return (
    <Section title={t.configPage.reasoning} defaultOpen={true}>
      {(() => { const specActive = local.spec_type && local.spec_type !== 'none' && !isEmbedding; return (<>
      <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
        <Select label={`${t.configPage.reasoningSwitch} (--reasoning)`} value={local.reasoning} onChange={v => set('reasoning', v)} options={['', 'on', 'off', 'auto']} title={t.configPage.reasoningTip} disabled={isEmbedding} defaultLabel={t.common.default}  active={a('reasoning')} />
        <Select label={`${t.configPage.specType} (--spec-type)`} value={local.spec_type} onChange={v => set('spec_type', v)} options={specTypes} title={t.configPage.specTypeTip} disabled={isEmbedding} defaultLabel={t.common.default}  active={a('spec_type')} />
        <Num label={`${t.configPage.draftTokens} (--spec-draft-n-max)`} value={local.draft_tokens} onChange={v => set('draft_tokens', v)} min={0} title={t.configPage.draftTokensTip} disabled={!specActive}  active={a('draft_tokens')} />
        <Num label={`${t.configPage.specDraftNMin} (--spec-draft-n-min)`} value={local.spec_draft_n_min} onChange={v => set('spec_draft_n_min', v)} min={0} title={t.configPage.specDraftNMinTip} disabled={!specActive}  active={a('spec_draft_n_min')} />
        <Num label={`${t.configPage.temp} (--temp)`} value={local.temp} onChange={v => set('temp', v)} min={0} max={2} step={0.1} title={t.configPage.tempTip} disabled={isEmbedding}  active={a('temp')} />
        <Num label={`${t.configPage.topK} (--top-k)`} value={local.top_k} onChange={v => set('top_k', v)} min={0} title={t.configPage.topKTip} disabled={isEmbedding}  active={a('top_k')} />
        <Num label={`${t.configPage.topP} (--top-p)`} value={local.top_p} onChange={v => set('top_p', v)} min={0} max={1} step={0.1} title={t.configPage.topPTip} disabled={isEmbedding}  active={a('top_p')} />
        <Num label={`${t.configPage.repeatPenalty} (--repeat-penalty)`} value={local.repeat_penalty} onChange={v => set('repeat_penalty', v)} min={0} max={2} step={0.1} title={t.configPage.repeatPenaltyTip} disabled={isEmbedding}  active={a('repeat_penalty')} />
        <Num label={`${t.configPage.nPredict} (-n)`} value={local.n_predict} onChange={v => set('n_predict', v)} min={-1} title={t.configPage.nPredictTip} disabled={isEmbedding}  active={a('n_predict')} />
        <Switch label={`${t.configPage.ignoreEos} (--ignore-eos)`} value={local.ignore_eos} onChange={v => set('ignore_eos', v)} title={t.configPage.ignoreEosTip} disabled={isEmbedding}  active={a('ignore_eos')} />
        <Input label={`${t.configPage.reversePrompt} (-r)`} value={local.reverse_prompt} onChange={v => set('reverse_prompt', v)} title={t.configPage.reversePromptTip} disabled={isEmbedding}  active={a('reverse_prompt')} />
      </div>
      </>); })()}
    </Section>
  )
}

export function PerformanceSection({ local, set, t, activeParams }: Props) {
  const a = (k: keyof InstanceConfig) => activeParams.has(k)
  return (
    <Section title={t.configPage.performance} defaultOpen={true}>
      <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
        <Num label={`${t.configPage.threads} (-t)`} value={local.threads} onChange={v => set('threads', v)} min={0} title={t.configPage.threadsTip}  active={a('threads')} />
        <Num label={`${t.configPage.threadsBatch} (--threads-batch)`} value={local.threads_batch} onChange={v => set('threads_batch', v)} min={0} title={t.configPage.threadsBatchTip}  active={a('threads_batch')} />
        <Num label={`${t.configPage.batchSize} (-b)`} value={local.batch_size} onChange={v => set('batch_size', v)} min={1} title={t.configPage.batchSizeTip}  active={a('batch_size')} />
        <Num label={`${t.configPage.ubatchSize} (-ub)`} value={local.ubatch_size} onChange={v => set('ubatch_size', v)} min={1} title={t.configPage.ubatchSizeTip}  active={a('ubatch_size')} />
        <Num label={`${t.configPage.parallel} (-np)`} value={local.parallel} onChange={v => set('parallel', v)} min={-1} title={t.configPage.parallelTip}  active={a('parallel')} />
        <Switch label={`${t.configPage.contBatching} (-cb)`} value={local.cont_batching} onChange={v => set('cont_batching', v)} title={t.configPage.contBatchingTip}  active={a('cont_batching')} />
        <Select label={`${t.configPage.flashAttn} (-fa)`} value={local.flash_attn} onChange={v => set('flash_attn', v)} options={['auto', 'on', 'off']} title={t.configPage.flashAttnTip} defaultLabel={t.common.default}  active={a('flash_attn')} />
        <Switch label={`${t.configPage.mlock} (--mlock)`} value={local.mlock} onChange={v => set('mlock', v)} title={t.configPage.mlockTip}  active={a('mlock')} />
        <Switch label={`${t.configPage.noMmap} (--no-mmap)`} value={local.no_mmap} onChange={v => set('no_mmap', v)} title={t.configPage.noMmapTip}  active={a('no_mmap')} />
      <Switch label={`${t.configPage.noRepack} (--no-repack)`} value={local.no_repack} onChange={v => set('no_repack', v)} title={t.configPage.noRepackTip}  active={a('no_repack')} />
        <Switch label={`${t.configPage.numa} (--numa)`} value={local.numa} onChange={v => set('numa', v)} title={t.configPage.numaTip}  active={a('numa')} />
      </div>
    </Section>
  )
}

// ━━━━━━━━━━━━━━━━━━━━━━ ADVANCED CONTAINER ━━━━━━━━━━━━━━━━━━━━━━

export function AdvancedSection({ local, set, t, isEmbedding, onShowDraftPicker, activeParams }: Props) {
  const a = (k: keyof InstanceConfig) => activeParams.has(k)
  const resetAll = () => {
    for (const defaults of Object.values(RESET_MAP)) {
      for (const [k, v] of Object.entries(defaults)) {
        set(k as keyof InstanceConfig, v)
      }
    }
  }

  const resetGroup = (id: string) => {
    const defaults = (RESET_MAP as any)[id]
    if (defaults) {
      for (const [k, v] of Object.entries(defaults)) {
        set(k as keyof InstanceConfig, v)
      }
    }
  }
  const specActive = local.spec_type && local.spec_type !== 'none' && !isEmbedding

  return (
    <Section title={t.configPage.advSectionTitle} defaultOpen={false}>
      <div className="flex items-center justify-end mb-2">
        <span className="text-xs text-gray-400 mr-2">{t.configPage.advSectionReset}</span>
        <ResetButton onClick={resetAll} title={t.configPage.advSectionReset} />
      </div>
      <div className="space-y-2">
        {/* 推理配置 (6) */}
        <CollapsibleGroup title={t.configPage.subAdvReasoning} onReset={() => resetGroup('advancedReasoningConfig')} disabled={isEmbedding}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Select label={`${t.configPage.reasoningFormat} (--reasoning-format)`} value={local.reasoning_format} onChange={v => set('reasoning_format', v)} options={['', 'none', 'deepseek', 'deepseek-legacy']} title={t.configPage.reasoningFormatTip} defaultLabel={t.common.default}  active={a('reasoning_format')} />
            <Select label={`${t.configPage.reasoningEffort} (--chat-template-kwargs)`} value={local.reasoning_effort} onChange={v => set('reasoning_effort', v)} options={['', 'low', 'medium', 'high']} title={t.configPage.reasoningEffortTip} defaultLabel={t.common.default}  active={a('reasoning_effort')} />
            <Num label={`${t.configPage.reasoningBudget} (--reasoning-budget)`} value={local.reasoning_budget ? parseInt(local.reasoning_budget) : 0} onChange={v => set('reasoning_budget', v.toString())} min={0} max={65536} step={256} title={t.configPage.reasoningBudgetTip} />
            <Input label={`${t.configPage.reasoningBudgetMsg} (--reasoning-budget-message)`} value={local.reasoning_budget_message} onChange={v => set('reasoning_budget_message', v)} title={t.configPage.reasoningBudgetMsgTip}  active={a('reasoning_budget_message')} />
            <Switch label={`${t.configPage.jinja} (--jinja)`} value={local.jinja} onChange={v => set('jinja', v)} title={t.configPage.jinjaTip}  active={a('jinja')} />
            <Switch label={`${t.configPage.skipChatParsing} (--skip-chat-parsing)`} value={local.skip_chat_parsing} onChange={v => set('skip_chat_parsing', v)} title={t.configPage.skipChatParsingTip}  active={a('skip_chat_parsing')} />
          </div>
        </CollapsibleGroup>

        {/* 模型适配 (8) */}
        <CollapsibleGroup title={t.configPage.subAdvModelAdapt} onReset={() => resetGroup('advancedModelAdapt')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Input label={`${t.configPage.chatTemplateFile} (--chat-template-file)`} value={local.chat_template_file} onChange={v => set('chat_template_file', v)} title={t.configPage.chatTemplateFileTip}  active={a('chat_template_file')} />
            <Input label={`${t.configPage.lora} (--lora)`} value={local.lora_path} onChange={v => set('lora_path', v)} title={t.configPage.loraTip}  active={a('lora_path')} />
            <Switch label={`${t.configPage.loraInitNoApply} (--lora-init-without-apply)`} value={local.lora_init_without_apply} onChange={v => set('lora_init_without_apply', v)} title={t.configPage.loraInitNoApplyTip}  active={a('lora_init_without_apply')} />
      <Input label={`${t.configPage.loraScaled} (--lora-scaled)`} value={local.lora_scaled || ''} onChange={v => set('lora_scaled', v)} title={t.configPage.loraScaledTip} disabled={isEmbedding}  active={a('lora_scaled')} />
            <Input label={`${t.configPage.mmproj} (--mmproj)`} value={local.mmproj_path} onChange={v => set('mmproj_path', v)} title={t.configPage.mmprojTip} disabled={isEmbedding}  active={a('mmproj_path')} />
            <Input label={`${t.configPage.grammarFile} (--grammar-file)`} value={local.grammar_file} onChange={v => set('grammar_file', v)} title={t.configPage.grammarFileTip}  active={a('grammar_file')} />
            <Input label={`${t.configPage.grammar} (--grammar)`} value={local.grammar} onChange={v => set('grammar', v)} title={t.configPage.grammarTip}  active={a('grammar')} />
            <Num label={`${t.configPage.embdNormalize} (--embd-normalize)`} value={local.embd_normalize} onChange={v => set('embd_normalize', v)} min={0} max={2} title={t.configPage.embdNormalizeTip}  active={a('embd_normalize')} />
            <Switch label={`${t.configPage.reranking} (--reranking)`} value={local.reranking} onChange={v => set('reranking', v)} title={t.configPage.rerankingTip}  active={a('reranking')} />
          </div>
        </CollapsibleGroup>

        {/* 高级采样 (19) */}
        <CollapsibleGroup title={t.configPage.subAdvSampling} onReset={() => resetGroup('advancedSampling')} disabled={isEmbedding}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Select label={`${t.configPage.mirostat} (--mirostat)`} value={local.mirostat.toString()} onChange={v => set('mirostat', parseInt(v))} options={['0', '1', '2']} title={t.configPage.mirostatTip} defaultLabel={t.common.default} />
            <Num label={`${t.configPage.mirostatLr} (--mirostat-lr)`} value={local.mirostat_lr} onChange={v => set('mirostat_lr', v)} min={0} max={1} step={0.001} title={t.configPage.mirostatLrTip}  active={a('mirostat_lr')} />
            <Num label={`${t.configPage.mirostatEnt} (--mirostat-ent)`} value={local.mirostat_ent} onChange={v => set('mirostat_ent', v)} min={0} max={10} step={0.1} title={t.configPage.mirostatEntTip}  active={a('mirostat_ent')} />
            <Num label={`${t.configPage.xtcProbability} (--xtc-probability)`} value={local.xtc_probability} onChange={v => set('xtc_probability', v)} min={0} max={1} step={0.05} title={t.configPage.xtcProbabilityTip}  active={a('xtc_probability')} />
            <Num label={`${t.configPage.xtcThreshold} (--xtc-threshold)`} value={local.xtc_threshold} onChange={v => set('xtc_threshold', v)} min={0} max={1} step={0.05} title={t.configPage.xtcThresholdTip}  active={a('xtc_threshold')} />
            <Num label={`${t.configPage.dynatempRange} (--dynatemp-range)`} value={local.dynatemp_range} onChange={v => set('dynatemp_range', v)} min={0} max={10} step={0.1} title={t.configPage.dynatempRangeTip}  active={a('dynatemp_range')} />
            <Num label={`${t.configPage.dynatempExp} (--dynatemp-exp)`} value={local.dynatemp_exp} onChange={v => set('dynatemp_exp', v)} min={0} max={10} step={0.1} title={t.configPage.dynatempExpTip}  active={a('dynatemp_exp')} />
            <Num label={`${t.configPage.typicalP} (--typical-p)`} value={local.typical_p} onChange={v => set('typical_p', v)} min={0} max={1} step={0.05} title={t.configPage.typicalPTip}  active={a('typical_p')} />
            <Num label={`${t.configPage.dryMultiplier} (--dry-multiplier)`} value={local.dry_multiplier} onChange={v => set('dry_multiplier', v)} min={0} max={10} step={0.1} title={t.configPage.dryMultiplierTip}  active={a('dry_multiplier')} />
            <Num label={`${t.configPage.dryBase} (--dry-base)`} value={local.dry_base} onChange={v => set('dry_base', v)} min={0} max={10} step={0.1} title={t.configPage.dryBaseTip}  active={a('dry_base')} />
            <Num label={`${t.configPage.dryAllowedLength} (--dry-allowed-length)`} value={local.dry_allowed_length} onChange={v => set('dry_allowed_length', v)} min={0} step={1} title={t.configPage.dryAllowedLengthTip}  active={a('dry_allowed_length')} />
            <Num label={`${t.configPage.dryPenaltyLastN} (--dry-penalty-last-n)`} value={local.dry_penalty_last_n} onChange={v => set('dry_penalty_last_n', v)} min={-1} title={t.configPage.dryPenaltyLastNTip}  active={a('dry_penalty_last_n')} />
            <Input label={`${t.configPage.drySeqBreaker} (--dry-sequence-breaker)`} value={local.dry_sequence_breaker} onChange={v => set('dry_sequence_breaker', v)} title={t.configPage.drySeqBreakerTip}  active={a('dry_sequence_breaker')} />
            <Num label={`${t.configPage.adaptiveTarget} (--adaptive-target)`} value={local.adaptive_target} onChange={v => set('adaptive_target', v)} min={0} max={10} step={0.1} title={t.configPage.adaptiveTargetTip}  active={a('adaptive_target')} />
            <Num label={`${t.configPage.adaptiveDecay} (--adaptive-decay)`} value={local.adaptive_decay} onChange={v => set('adaptive_decay', v)} min={0} max={1} step={0.01} title={t.configPage.adaptiveDecayTip}  active={a('adaptive_decay')} />
            <Num label={`${t.configPage.topNSigma} (--top-n-sigma)`} value={local.top_n_sigma} onChange={v => set('top_n_sigma', v)} min={-1} max={10} step={0.1} title={t.configPage.topNSigmaTip}  active={a('top_n_sigma')} />
            <Input label={`${t.configPage.logitBias} (-l)`} value={local.logit_bias} onChange={v => set('logit_bias', v)} title={t.configPage.logitBiasTip}  active={a('logit_bias')} />
            <Input label={`${t.configPage.samplers} (--samplers)`} value={local.samplers} onChange={v => set('samplers', v)} title={t.configPage.samplersTip}  active={a('samplers')} />
            <Input label={`${t.configPage.samplerSeq} (--sampler-seq)`} value={local.sampler_seq} onChange={v => set('sampler_seq', v)} title={t.configPage.samplerSeqTip}  active={a('sampler_seq')} />
          </div>
        </CollapsibleGroup>

        {/* 采样参数扩展 (9) */}
        <CollapsibleGroup title={t.configPage.subAdvSamplingExt} onReset={() => resetGroup('advancedSamplingExt')} disabled={isEmbedding}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Num label={`${t.configPage.seed} (--seed)`} value={local.seed} onChange={v => set('seed', v)} min={-1} title={t.configPage.seedTip}  active={a('seed')} />
            <Num label={`${t.configPage.minP} (--min-p)`} value={local.min_p} onChange={v => set('min_p', v)} min={0} max={1} step={0.05} title={t.configPage.minPTip}  active={a('min_p')} />
            <Num label={`${t.configPage.presencePenalty} (--presence-penalty)`} value={local.presence_penalty} onChange={v => set('presence_penalty', v)} min={0} max={2} step={0.1} title={t.configPage.presencePenaltyTip}  active={a('presence_penalty')} />
            <Num label={`${t.configPage.frequencyPenalty} (--frequency-penalty)`} value={local.frequency_penalty} onChange={v => set('frequency_penalty', v)} min={0} max={2} step={0.1} title={t.configPage.frequencyPenaltyTip}  active={a('frequency_penalty')} />
            <Num label={`${t.configPage.repeatLastN} (--repeat-last-n)`} value={local.repeat_last_n} onChange={v => set('repeat_last_n', v)} min={-1} title={t.configPage.repeatLastNTip}  active={a('repeat_last_n')} />
            <Switch label={`${t.configPage.special} (-sp)`} value={local.special} onChange={v => set('special', v)} title={t.configPage.specialTip}  active={a('special')} />
            <Switch label={`${t.configPage.spmInfill} (--spm-infill)`} value={local.spm_infill} onChange={v => set('spm_infill', v)} title={t.configPage.spmInfillTip}  active={a('spm_infill')} />
            <Switch label={`${t.configPage.backendSampling} (-bs)`} value={local.backend_sampling} onChange={v => set('backend_sampling', v)} title={t.configPage.backendSamplingTip}  active={a('backend_sampling')} />
            <Input label={`${t.configPage.jsonSchema} (--json-schema)`} value={local.json_schema} onChange={v => set('json_schema', v)} title={t.configPage.jsonSchemaTip}  active={a('json_schema')} />
            <Input label={`${t.configPage.jsonSchemaFile} (-jf)`} value={local.json_schema_file} onChange={v => set('json_schema_file', v)} title={t.configPage.jsonSchemaFileTip}  active={a('json_schema_file')} />
          </div>
        </CollapsibleGroup>

        {/* 推测解码 (9) */}
        <CollapsibleGroup title={t.configPage.subAdvSpec} onReset={() => resetGroup('advancedSpec')} disabled={!specActive}>
          {!specActive && <div className="bg-gray-50 dark:bg-gray-800/50 rounded px-3 py-2 text-xs text-gray-500 mb-2">{t.configPage.specDisabled}</div>}
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <div title={t.configPage.draftModelTip}>
              <label className={`block text-xs font-medium mb-1 ${a('draft_model_path') ? 'text-green-600 dark:text-green-400' : 'text-gray-500'}`}>{`${t.configPage.draftModel} (-md)`}</label>
              <div className="flex gap-1">
                <input type="text" value={local.draft_model_path} onChange={e => set('draft_model_path', e.target.value)} disabled={isEmbedding} className={`flex-1 px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 ${isEmbedding ? 'opacity-50 cursor-not-allowed' : ''}`} />
                <button onClick={onShowDraftPicker} disabled={isEmbedding} className="px-2 py-1.5 bg-blue-600 hover:bg-blue-700 text-white rounded-lg text-xs disabled:opacity-50 disabled:cursor-not-allowed" title={t.configPage.draftModelTip}>{'\uD83D\uDCC2'}</button>
              </div>
            </div>
            <Num label={`${t.configPage.draftGpu} (-ngld)`} value={local.draft_gpu_layers} onChange={v => set('draft_gpu_layers', v)} min={0} max={99} title={t.configPage.draftGpuTip} disabled={isEmbedding}  active={a('draft_gpu_layers')} />
            <Num label={`${t.configPage.specDraftPMin} (--spec-draft-p-min)`} value={local.spec_draft_p_min} onChange={v => set('spec_draft_p_min', v)} min={0} max={1} step={0.05} title={t.configPage.specDraftPMinTip} disabled={isEmbedding}  active={a('spec_draft_p_min')} />
            <Num label={`${t.configPage.specDraftPSplit} (--spec-draft-p-split)`} value={local.spec_draft_p_split} onChange={v => set('spec_draft_p_split', v)} min={0} max={1} step={0.05} title={t.configPage.specDraftPSplitTip} disabled={isEmbedding}  active={a('spec_draft_p_split')} />
            <Input label={`${t.configPage.specDraftDevice} (--spec-draft-device)`} value={local.spec_draft_device} onChange={v => set('spec_draft_device', v)} title={t.configPage.specDraftDeviceTip} disabled={isEmbedding}  active={a('spec_draft_device')} />
            <Input label={`${t.configPage.lookupCacheStatic} (-lcs)`} value={local.lookup_cache_static} onChange={v => set('lookup_cache_static', v)} title={t.configPage.lookupCacheStaticTip} disabled={isEmbedding}  active={a('lookup_cache_static')} />
            <Input label={`${t.configPage.lookupCacheDynamic} (-lcd)`} value={local.lookup_cache_dynamic} onChange={v => set('lookup_cache_dynamic', v)} title={t.configPage.lookupCacheDynamicTip} disabled={isEmbedding}  active={a('lookup_cache_dynamic')} />
            <Select label={`${t.configPage.cacheTypeDraftK} (-ctdk)`} value={local.cache_type_draft_k} onChange={v => set('cache_type_draft_k', v)} options={cacheTypes} title={t.configPage.cacheTypeDraftKTip} defaultLabel={t.common.default}  active={a('cache_type_draft_k')} />
            <Select label={`${t.configPage.cacheTypeDraftV} (-ctdv)`} value={local.cache_type_draft_v} onChange={v => set('cache_type_draft_v', v)} options={cacheTypes} title={t.configPage.cacheTypeDraftVTip} defaultLabel={t.common.default}  active={a('cache_type_draft_v')} />
            <Switch label={`${t.configPage.specDefault} (--spec-default)`} value={local.spec_default} onChange={v => set('spec_default', v)} title={t.configPage.specDefaultTip} disabled={isEmbedding}  active={a('spec_default')} />
            <Switch label={`${t.configPage.specDraftBackendSampling} (--no-spec-draft-backend-sampling)`} value={!local.spec_draft_backend_sampling} onChange={v => set('spec_draft_backend_sampling', !v)} title={t.configPage.specDraftBackendSamplingTip} disabled={isEmbedding} />
            <Num label={`${t.configPage.specDraftThreads} (-td)`} value={local.spec_draft_threads} onChange={v => set('spec_draft_threads', v)} min={0} title={t.configPage.specDraftThreadsTip} disabled={isEmbedding}  active={a('spec_draft_threads')} />
            <Num label={`${t.configPage.specDraftThreadsBatch} (-tbd)`} value={local.spec_draft_threads_batch} onChange={v => set('spec_draft_threads_batch', v)} min={0} title={t.configPage.specDraftThreadsBatchTip} disabled={isEmbedding}  active={a('spec_draft_threads_batch')} />
          </div>
        </CollapsibleGroup>

        {/* 上下文缩放 / RoPE · YaRN (8) */}
        <CollapsibleGroup title={t.configPage.subAdvRope} onReset={() => resetGroup('advancedRope')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Select label={`${t.configPage.ropeScaling} (--rope-scaling)`} value={local.rope_scaling} onChange={v => set('rope_scaling', v)} options={['', 'none', 'linear', 'yarn']} title={t.configPage.ropeScalingTip} defaultLabel={t.common.default}  active={a('rope_scaling')} />
            <Num label={`${t.configPage.ropeScale} (--rope-scale)`} value={local.rope_scale} onChange={v => set('rope_scale', v)} min={0} max={10} step={0.1} title={t.configPage.ropeScaleTip}  active={a('rope_scale')} />
            <Num label={`${t.configPage.ropeFreqBase} (--rope-freq-base)`} value={local.rope_freq_base} onChange={v => set('rope_freq_base', v)} min={0} title={t.configPage.ropeFreqBaseTip}  active={a('rope_freq_base')} />
            <Num label={`${t.configPage.ropeFreqScale} (--rope-freq-scale)`} value={local.rope_freq_scale} onChange={v => set('rope_freq_scale', v)} min={0} max={10} step={0.1} title={t.configPage.ropeFreqScaleTip}  active={a('rope_freq_scale')} />
            <Num label={`${t.configPage.yarnExtFactor} (--yarn-ext-factor)`} value={local.yarn_ext_factor} onChange={v => set('yarn_ext_factor', v)} min={-1} max={10} step={0.1} title={t.configPage.yarnExtFactorTip}  active={a('yarn_ext_factor')} />
            <Num label={`${t.configPage.yarnAttnFactor} (--yarn-attn-factor)`} value={local.yarn_attn_factor} onChange={v => set('yarn_attn_factor', v)} min={-1} max={10} step={0.1} title={t.configPage.yarnAttnFactorTip}  active={a('yarn_attn_factor')} />
            <Num label={`${t.configPage.yarnBetaSlow} (--yarn-beta-slow)`} value={local.yarn_beta_slow} onChange={v => set('yarn_beta_slow', v)} min={0} max={10} step={0.1} title={t.configPage.yarnBetaSlowTip}  active={a('yarn_beta_slow')} />
            <Num label={`${t.configPage.yarnBetaFast} (--yarn-beta-fast)`} value={local.yarn_beta_fast} onChange={v => set('yarn_beta_fast', v)} min={-1} max={128} title={t.configPage.yarnBetaFastTip}  active={a('yarn_beta_fast')} />
      <Num label={`${t.configPage.yarnOrigCtx} (--yarn-orig-ctx)`} value={local.yarn_orig_ctx || 0} onChange={v => set('yarn_orig_ctx', v)} min={0} max={1048576} title={t.configPage.yarnOrigCtxTip}  active={a('yarn_orig_ctx')} />
          </div>
        </CollapsibleGroup>

        {/* KV 缓存 (8) */}
        <CollapsibleGroup title={t.configPage.subAdvKvCache} onReset={() => resetGroup('advancedKvCache')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Select label={`${t.configPage.cacheTypeK} (-ctk)`} value={local.cache_type_k} onChange={v => set('cache_type_k', v)} options={cacheTypes} title={t.configPage.cacheTypeKTip} defaultLabel={t.common.default}  active={a('cache_type_k')} />
            <Select label={`${t.configPage.cacheTypeV} (-ctv)`} value={local.cache_type_v} onChange={v => set('cache_type_v', v)} options={cacheTypes} title={t.configPage.cacheTypeVTip} defaultLabel={t.common.default}  active={a('cache_type_v')} />
            <Switch label={`${t.configPage.cachePrompt} (--no-cache-prompt)`} value={!local.cache_prompt} onChange={v => set('cache_prompt', !v)} title={t.configPage.cachePromptTip} />
            <Num label={`${t.configPage.cacheReuse} (--cache-reuse)`} value={local.cache_reuse} onChange={v => set('cache_reuse', v)} min={0} title={t.configPage.cacheReuseTip}  active={a('cache_reuse')} />
            <Num label={`${t.configPage.cacheRam} (-cram)`} value={local.cache_ram} onChange={v => set('cache_ram', v)} min={-1} step={256} title={t.configPage.cacheRamTip}  active={a('cache_ram')} />
            <Switch label={`${t.configPage.warmup} (--warmup)`} value={local.warmup} onChange={v => set('warmup', v)} title={t.configPage.warmupTip}  active={a('warmup')} />
            <Switch label={`${t.configPage.cacheIdleSlots} (--no-cache-idle-slots)`} value={!local.cache_idle_slots} onChange={v => set('cache_idle_slots', !v)} title={t.configPage.cacheIdleSlotsTip} />
            <Switch label={`${t.configPage.kvUnified} (--kv-unified)`} value={local.kv_unified} onChange={v => set('kv_unified', v)} title={t.configPage.kvUnifiedTip}  active={a('kv_unified')} />
      <Switch label={`${t.configPage.noKvOffload} (--no-kv-offload)`} value={local.no_kv_offload} onChange={v => set('no_kv_offload', v)} title={t.configPage.noKvOffloadTip}  active={a('no_kv_offload')} />
          </div>
        </CollapsibleGroup>

        {/* 上下文管理 (6) */}
        <CollapsibleGroup title={t.configPage.subAdvContextMgmt} onReset={() => resetGroup('advancedContextMgmt')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Num label={`${t.configPage.ctxCheckpoints} (-ctxcp)`} value={local.ctx_checkpoints} onChange={v => set('ctx_checkpoints', v)} min={0} title={t.configPage.ctxCheckpointsTip}  active={a('ctx_checkpoints')} />
            <Num label={`${t.configPage.checkpointMinStep} (-cms)`} value={local.checkpoint_min_step} onChange={v => set('checkpoint_min_step', v)} min={0} title={t.configPage.checkpointMinStepTip}  active={a('checkpoint_min_step')} />
            <Switch label={`${t.configPage.contextShift} (--context-shift)`} value={local.context_shift} onChange={v => set('context_shift', v)} title={t.configPage.contextShiftTip} disabled={isEmbedding}  active={a('context_shift')} />
            <Switch label={`${t.configPage.swaFull} (--swa-full)`} value={local.swa_full} onChange={v => set('swa_full', v)} title={t.configPage.swaFullTip}  active={a('swa_full')} />
            <Num label={`${t.configPage.keep} (--keep)`} value={local.keep} onChange={v => set('keep', v)} min={0} title={t.configPage.keepTip}  active={a('keep')} />
            <Input label={`${t.configPage.overrideKv} (--override-kv)`} value={local.override_kv} onChange={v => set('override_kv', v)} title={t.configPage.overrideKvTip}  active={a('override_kv')} />
          </div>
        </CollapsibleGroup>

        {/* 硬件配置 (9) */}
        <CollapsibleGroup title={t.configPage.subAdvHardware} onReset={() => resetGroup('advancedHardware')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Num label={`${t.configPage.moeCpu} (--n-cpu-moe)`} value={local.moe_cpu_layers} onChange={v => set('moe_cpu_layers', v)} min={0} max={99} title={t.configPage.moeCpuTip} disabled={isEmbedding}  active={a('moe_cpu_layers')} />
            <Input label={`${t.configPage.device} (-dev)`} value={local.device} onChange={v => set('device', v)} title={t.configPage.deviceTip}  active={a('device')} />
            <Select label={`${t.configPage.splitMode} (-sm)`} value={local.split_mode} onChange={v => set('split_mode', v)} options={['', 'none', 'layer', 'row']} title={t.configPage.splitModeTip} defaultLabel={t.common.default}  active={a('split_mode')} />
            <Input label={`${t.configPage.tensorSplit} (-ts)`} value={local.tensor_split} onChange={v => set('tensor_split', v)} title={t.configPage.tensorSplitTip}  active={a('tensor_split')} />
            <Num label={`${t.configPage.mainGpu} (-mg)`} value={local.main_gpu} onChange={v => set('main_gpu', v)} min={0} max={9} title={t.configPage.mainGpuTip}  active={a('main_gpu')} />
            <Switch label={`${t.configPage.perf} (--perf)`} value={local.perf} onChange={v => set('perf', v)} title={t.configPage.perfTip}  active={a('perf')} />
            <Switch label={`${t.configPage.checkTensors} (--check-tensors)`} value={local.check_tensors} onChange={v => set('check_tensors', v)} title={t.configPage.checkTensorsTip}  active={a('check_tensors')} />
            <Switch label={`${t.configPage.fit} (--fit)`} value={local.fit} onChange={v => set('fit', v)} title={t.configPage.fitTip}  active={a('fit')} />
            <Input label={`${t.configPage.fitTarget} (-fitt)`} value={local.fit_target} onChange={v => set('fit_target', v)} title={t.configPage.fitTargetTip} disabled={!local.fit}  active={a('fit_target')} />
            <Num label={`${t.configPage.fitCtx} (-fitc)`} value={local.fit_ctx} onChange={v => set('fit_ctx', v)} min={0} title={t.configPage.fitCtxTip} disabled={!local.fit}  active={a('fit_ctx')} />
            <Num label={`${t.configPage.threadsHttp} (--threads-http)`} value={local.threads_http} onChange={v => set('threads_http', v)} min={-1} title={t.configPage.threadsHttpTip}  active={a('threads_http')} />
          </div>
        </CollapsibleGroup>

        {/* 服务基础 (10) */}
        <CollapsibleGroup title={t.configPage.subAdvServer} onReset={() => resetGroup('advancedServerBasic')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Input label={`${t.configPage.apiKey} (--api-key)`} value={local.api_key} onChange={v => set('api_key', v)} type="password" title={t.configPage.apiKeyTip}  active={a('api_key')} />
            <Input label={`${t.configPage.apiKeyFile} (--api-key-file)`} value={local.api_key_file} onChange={v => set('api_key_file', v)} title={t.configPage.apiKeyFileTip}  active={a('api_key_file')} />
            <Switch label={`${t.configPage.noUi} (--no-ui)`} value={local.no_ui} onChange={v => set('no_ui', v)} title={t.configPage.noUiTip}  active={a('no_ui')} />
      <Switch label={`${t.configPage.offline} (--offline)`} value={local.offline} onChange={v => set('offline', v)} title={t.configPage.offlineTip}  active={a('offline')} />
            <Input label={`${t.configPage.pathPrefix} (--path)`} value={local.path_prefix} onChange={v => set('path_prefix', v)} title={t.configPage.pathPrefixTip}  active={a('path_prefix')} />
            <Input label={`${t.configPage.apiPrefix} (--api-prefix)`} value={local.api_prefix} onChange={v => set('api_prefix', v)} title={t.configPage.apiPrefixTip}  active={a('api_prefix')} />
            <Num label={`${t.configPage.timeout} (-to)`} value={local.timeout} onChange={v => set('timeout', v)} min={1} title={t.configPage.timeoutTip}  active={a('timeout')} />
            <Num label={`${t.configPage.sleepIdle} (--sleep-idle-seconds)`} value={local.sleep_idle} onChange={v => set('sleep_idle', v)} min={-1} title={t.configPage.sleepIdleTip}  active={a('sleep_idle')} />
            <Switch label={`${t.configPage.verbose} (-v)`} value={local.verbose} onChange={v => set('verbose', v)} title={t.configPage.verboseTip}  active={a('verbose')} />
          </div>
          <div className="grid grid-cols-2 gap-3 mt-3">
            <Input label={`${t.configPage.sslKey} (--ssl-key-file)`} value={local.ssl_key_file} onChange={v => set('ssl_key_file', v)} title={t.configPage.sslKeyTip}  active={a('ssl_key_file')} />
            <Input label={`${t.configPage.sslCert} (--ssl-cert-file)`} value={local.ssl_cert_file} onChange={v => set('ssl_cert_file', v)} title={t.configPage.sslCertTip}  active={a('ssl_cert_file')} />
          </div>
        </CollapsibleGroup>

        {/* 服务扩展 (7) */}
        <CollapsibleGroup title={t.configPage.subAdvServerExt} onReset={() => resetGroup('advancedServerExt')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Switch label={`${t.configPage.slotsEnabled} (--no-slots)`} value={!local.slots_enabled} onChange={v => set('slots_enabled', !v)} title={t.configPage.slotsEnabledTip} />
            <Switch label={`${t.configPage.metrics} (--metrics)`} value={local.metrics} onChange={v => set('metrics', v)} title={t.configPage.metricsTip}  active={a('metrics')} />
            <Switch label={`${t.configPage.props} (--props)`} value={local.props} onChange={v => set('props', v)} title={t.configPage.propsTip}  active={a('props')} />
            <Input label={`${t.configPage.slotSavePath} (--slot-save-path)`} value={local.slot_save_path} onChange={v => set('slot_save_path', v)} title={t.configPage.slotSavePathTip}  active={a('slot_save_path')} />
            <Num label={`${t.configPage.slotPromptSimilarity} (-sps)`} value={local.slot_prompt_similarity} onChange={v => set('slot_prompt_similarity', v)} min={0} max={1} step={0.05} title={t.configPage.slotPromptSimilarityTip}  active={a('slot_prompt_similarity')} />
            <Switch label={`${t.configPage.prefillAssistant} (--prefill-assistant)`} value={local.prefill_assistant} onChange={v => set('prefill_assistant', v)} title={t.configPage.prefillAssistantTip}  active={a('prefill_assistant')} />
            <Input label={`${t.configPage.uiConfigFile} (--ui-config-file)`} value={local.ui_config_file} onChange={v => set('ui_config_file', v)} title={t.configPage.uiConfigFileTip}  active={a('ui_config_file')} />
            <Input label={`${t.configPage.uiConfig} (--ui-config)`} value={local.ui_config} onChange={v => set('ui_config', v)} title={t.configPage.uiConfigTip}  active={a('ui_config')} />
            <Switch label={`${t.configPage.uiMcpProxy} (--ui-mcp-proxy)`} value={local.ui_mcp_proxy} onChange={v => set('ui_mcp_proxy', v)} title={t.configPage.uiMcpProxyTip}  active={a('ui_mcp_proxy')} />
          </div>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3 mt-3">
            <div className="col-span-2 md:col-span-3">
              <WorkerSelector value={local.rpc_servers} onChange={v => set('rpc_servers', v)} t={t} />
            </div>
            <Num label={`${t.configPage.ssePingInterval} (--sse-ping-interval)`} value={local.sse_ping_interval} onChange={v => set('sse_ping_interval', v)} min={0} title={t.configPage.ssePingIntervalTip}  active={a('sse_ping_interval')} />
            <Switch label={`${t.configPage.reusePort} (--reuse-port)`} value={local.reuse_port} onChange={v => set('reuse_port', v)} title={t.configPage.reusePortTip}  active={a('reuse_port')} />
          </div>
        </CollapsibleGroup>

        {/* 多模型/专家 (11) */}
        <CollapsibleGroup title={t.configPage.subAdvMulti} onReset={() => resetGroup('advancedMulti')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Input label={`${t.configPage.modelsDir} (--models-dir)`} value={local.models_dir} onChange={v => set('models_dir', v)} title={t.configPage.modelsDirTip}  active={a('models_dir')} />
            <Input label={`${t.configPage.modelsPreset} (--models-preset)`} value={local.models_preset} onChange={v => set('models_preset', v)} title={t.configPage.modelsPresetTip}  active={a('models_preset')} />
            <Num label={`${t.configPage.modelsMax} (--models-max)`} value={local.models_max} onChange={v => set('models_max', v)} min={1} max={99} title={t.configPage.modelsMaxTip}  active={a('models_max')} />
            <Switch label={`${t.configPage.modelsAutoload} (--models-autoload)`} value={local.models_autoload} onChange={v => set('models_autoload', v)} title={t.configPage.modelsAutoloadTip}  active={a('models_autoload')} />
            <Input label={`${t.configPage.mmprojUrl} (--mmproj-url)`} value={local.mmproj_url} onChange={v => set('mmproj_url', v)} title={t.configPage.mmprojUrlTip} disabled={isEmbedding}  active={a('mmproj_url')} />
            <Switch label={`${t.configPage.mmprojAuto} (--mmproj-auto)`} value={local.mmproj_auto} onChange={v => set('mmproj_auto', v)} title={t.configPage.mmprojAutoTip} disabled={isEmbedding}  active={a('mmproj_auto')} />
      <Switch label={`${t.configPage.noMmproj} (--no-mmproj)`} value={local.no_mmproj} onChange={v => set('no_mmproj', v)} title={t.configPage.noMmprojTip} disabled={isEmbedding}  active={a('no_mmproj')} />
      <Switch label={`${t.configPage.noMmprojOffload} (--no-mmproj-offload)`} value={local.no_mmproj_offload} onChange={v => set('no_mmproj_offload', v)} title={t.configPage.noMmprojOffloadTip} disabled={isEmbedding || local.no_mmproj}  active={a('no_mmproj_offload')} />
            <Num label={`${t.configPage.imageMinTokens} (--image-min-tokens)`} value={local.image_min_tokens} onChange={v => set('image_min_tokens', v)} min={0} title={t.configPage.imageMinTokensTip} disabled={isEmbedding}  active={a('image_min_tokens')} />
            <Num label={`${t.configPage.imageMaxTokens} (--image-max-tokens)`} value={local.image_max_tokens} onChange={v => set('image_max_tokens', v)} min={0} title={t.configPage.imageMaxTokensTip} disabled={isEmbedding}  active={a('image_max_tokens')} />
            <Input label={`${t.configPage.mediaPath} (--media-path)`} value={local.media_path} onChange={v => set('media_path', v)} title={t.configPage.mediaPathTip}  active={a('media_path')} />
            <Input label={`${t.configPage.tags} (--tags)`} value={local.tags} onChange={v => set('tags', v)} title={t.configPage.tagsTip}  active={a('tags')} />
            <Input label={`${t.configPage.tools} (--tools)`} value={local.tools} onChange={v => set('tools', v)} title={t.configPage.toolsTip}  active={a('tools')} />
          </div>
        </CollapsibleGroup>
      </div>
    </Section>
  )
}
