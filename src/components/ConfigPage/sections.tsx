import type { InstanceConfig } from '../../store'
import { Section, Input, Num, Toggle, Select, CollapsibleGroup, ResetButton, RESET_MAP, chatTemplates, specTypes, cacheTypes } from './shared'

interface Props {
  local: InstanceConfig
  set: (k: keyof InstanceConfig, v: any) => void
  t: any
  isEmbedding: boolean
  onShowPicker: () => void
}

// ━━━━━━━━━━━━━━━━━━━━━━ COMMON SECTIONS ━━━━━━━━━━━━━━━━━━━━━━

export function BasicSection({ local, set, t, onShowPicker }: Props) {
  return (
    <Section title={t.configPage.basic} defaultOpen={true}>
      <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
        <div title={t.configPage.modelPathTip}>
          <label className="block text-xs font-medium mb-1 text-gray-500">{t.configPage.modelPath}</label>
          <div className="flex gap-1">
            <input type="text" value={local.model_path} onChange={e => set('model_path', e.target.value)} className="flex-1 px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
            <button onClick={onShowPicker} className="px-2 py-1.5 bg-blue-600 hover:bg-blue-700 text-white rounded-lg text-xs" title={t.configPage.modelPathBtn}>{'\uD83D\uDCC2'}</button>
          </div>
        </div>
        <Input label={t.configPage.alias} value={local.alias} onChange={v => set('alias', v)} title={t.configPage.aliasTip} />
        <Select label={t.configPage.chatTemplate} value={local.chat_template} onChange={v => set('chat_template', v)} options={chatTemplates} title={t.configPage.chatTemplateTip} defaultLabel={t.common.default} />
        <Input label={t.configPage.host} value={local.host} onChange={v => set('host', v)} title={t.configPage.hostTip} />
        <Num label={t.configPage.portLabel} value={local.port} onChange={v => set('port', v)} min={1} max={65535} title={t.configPage.portLabelTip} />
        <Num label={t.configPage.gpuLayers} value={local.gpu_layers} onChange={v => set('gpu_layers', v)} min={0} max={99} title={t.configPage.gpuLayersTip} />
        <Num label={t.configPage.ctxSize} value={local.ctx_size} onChange={v => set('ctx_size', v)} min={0} step={1024} title={t.configPage.ctxSizeTip} disabled={local.ctx_size_auto} />
        <Toggle label={t.configPage.embedding} value={local.embedding} onChange={v => set('embedding', v)} title={t.configPage.embeddingTip} />
        <Select label={t.configPage.pooling} value={local.pooling} onChange={v => set('pooling', v)} options={['', 'none', 'mean', 'cls', 'last', 'rank']} title={t.configPage.poolingTip} defaultLabel={t.common.default} />
      </div>
    </Section>
  )
}

export function ReasoningSection({ local, set, t, isEmbedding }: Props) {
  return (
    <Section title={t.configPage.reasoning} defaultOpen={true}>
      <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
        <Select label={t.configPage.reasoningSwitch} value={local.reasoning} onChange={v => set('reasoning', v)} options={['', 'on', 'off', 'auto']} title={t.configPage.reasoningTip} disabled={isEmbedding} defaultLabel={t.common.default} />
        <Select label={t.configPage.specType} value={local.spec_type} onChange={v => set('spec_type', v)} options={specTypes} title={t.configPage.specTypeTip} disabled={isEmbedding} defaultLabel={t.common.default} />
        <Num label={t.configPage.draftTokens} value={local.draft_tokens} onChange={v => set('draft_tokens', v)} min={0} title={t.configPage.draftTokensTip} disabled={isEmbedding} />
        <Num label={t.configPage.specDraftNMin} value={local.spec_draft_n_min} onChange={v => set('spec_draft_n_min', v)} min={0} title={t.configPage.specDraftNMinTip} disabled={isEmbedding} />
        <Num label={t.configPage.temp} value={local.temp} onChange={v => set('temp', v)} min={0} max={2} step={0.1} title={t.configPage.tempTip} disabled={isEmbedding} />
        <Num label={t.configPage.topK} value={local.top_k} onChange={v => set('top_k', v)} min={0} title={t.configPage.topKTip} disabled={isEmbedding} />
        <Num label={t.configPage.topP} value={local.top_p} onChange={v => set('top_p', v)} min={0} max={1} step={0.1} title={t.configPage.topPTip} disabled={isEmbedding} />
        <Num label={t.configPage.repeatPenalty} value={local.repeat_penalty} onChange={v => set('repeat_penalty', v)} min={0} max={2} step={0.1} title={t.configPage.repeatPenaltyTip} disabled={isEmbedding} />
        <Num label={t.configPage.nPredict} value={local.n_predict} onChange={v => set('n_predict', v)} min={-1} title={t.configPage.nPredictTip} disabled={isEmbedding} />
        <Toggle label={t.configPage.ignoreEos} value={local.ignore_eos} onChange={v => set('ignore_eos', v)} title={t.configPage.ignoreEosTip} disabled={isEmbedding} />
        <Input label={t.configPage.reversePrompt} value={local.reverse_prompt} onChange={v => set('reverse_prompt', v)} title={t.configPage.reversePromptTip} disabled={isEmbedding} />
      </div>
    </Section>
  )
}

export function PerformanceSection({ local, set, t }: Props) {
  return (
    <Section title={t.configPage.performance} defaultOpen={true}>
      <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
        <Num label={t.configPage.threads} value={local.threads} onChange={v => set('threads', v)} min={0} title={t.configPage.threadsTip} />
        <Num label={t.configPage.batchSize} value={local.batch_size} onChange={v => set('batch_size', v)} min={1} title={t.configPage.batchSizeTip} />
        <Num label={t.configPage.ubatchSize} value={local.ubatch_size} onChange={v => set('ubatch_size', v)} min={1} title={t.configPage.ubatchSizeTip} />
        <Num label={t.configPage.parallel} value={local.parallel} onChange={v => set('parallel', v)} min={-1} title={t.configPage.parallelTip} />
        <Toggle label={t.configPage.contBatching} value={local.cont_batching} onChange={v => set('cont_batching', v)} title={t.configPage.contBatchingTip} />
        <Select label={t.configPage.flashAttn} value={local.flash_attn} onChange={v => set('flash_attn', v)} options={['auto', 'on', 'off']} title={t.configPage.flashAttnTip} defaultLabel={t.common.default} />
        <Toggle label={t.configPage.mlock} value={local.mlock} onChange={v => set('mlock', v)} title={t.configPage.mlockTip} />
        <Toggle label={t.configPage.noMmap} value={local.no_mmap} onChange={v => set('no_mmap', v)} title={t.configPage.noMmapTip} />
        <Toggle label={t.configPage.numa} value={local.numa} onChange={v => set('numa', v)} title={t.configPage.numaTip} />
      </div>
    </Section>
  )
}

// ━━━━━━━━━━━━━━━━━━━━━━ ADVANCED CONTAINER ━━━━━━━━━━━━━━━━━━━━━━

export function AdvancedSection({ local, set, t, isEmbedding }: Props) {
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
            <Select label={t.configPage.reasoningFormat} value={local.reasoning_format} onChange={v => set('reasoning_format', v)} options={['', 'none', 'deepseek', 'deepseek-legacy']} title={t.configPage.reasoningFormatTip} defaultLabel={t.common.default} />
            <Select label={t.configPage.reasoningEffort} value={local.reasoning_effort} onChange={v => set('reasoning_effort', v)} options={['', 'low', 'medium', 'high']} title={t.configPage.reasoningEffortTip} defaultLabel={t.common.default} />
            <Num label={t.configPage.reasoningBudget} value={local.reasoning_budget ? parseInt(local.reasoning_budget) : 0} onChange={v => set('reasoning_budget', v.toString())} min={0} max={65536} step={256} title={t.configPage.reasoningBudgetTip} />
            <Input label={t.configPage.reasoningBudgetMsg} value={local.reasoning_budget_message} onChange={v => set('reasoning_budget_message', v)} title={t.configPage.reasoningBudgetMsgTip} />
            <Toggle label={t.configPage.jinja} value={local.jinja} onChange={v => set('jinja', v)} title={t.configPage.jinjaTip} />
            <Toggle label={t.configPage.skipChatParsing} value={local.skip_chat_parsing} onChange={v => set('skip_chat_parsing', v)} title={t.configPage.skipChatParsingTip} />
          </div>
        </CollapsibleGroup>

        {/* 模型适配 (8) */}
        <CollapsibleGroup title={t.configPage.subAdvModelAdapt} onReset={() => resetGroup('advancedModelAdapt')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Input label={t.configPage.chatTemplateFile} value={local.chat_template_file} onChange={v => set('chat_template_file', v)} title={t.configPage.chatTemplateFileTip} />
            <Input label={t.configPage.lora} value={local.lora_path} onChange={v => set('lora_path', v)} title={t.configPage.loraTip} />
            <Toggle label={t.configPage.loraInitNoApply} value={local.lora_init_without_apply} onChange={v => set('lora_init_without_apply', v)} title={t.configPage.loraInitNoApplyTip} />
            <Input label={t.configPage.mmproj} value={local.mmproj_path} onChange={v => set('mmproj_path', v)} title={t.configPage.mmprojTip} disabled={isEmbedding} />
            <Input label={t.configPage.grammarFile} value={local.grammar_file} onChange={v => set('grammar_file', v)} title={t.configPage.grammarFileTip} />
            <Input label={t.configPage.grammar} value={local.grammar} onChange={v => set('grammar', v)} title={t.configPage.grammarTip} />
            <Num label={t.configPage.embdNormalize} value={local.embd_normalize} onChange={v => set('embd_normalize', v)} min={0} max={2} title={t.configPage.embdNormalizeTip} />
            <Toggle label={t.configPage.reranking} value={local.reranking} onChange={v => set('reranking', v)} title={t.configPage.rerankingTip} />
          </div>
        </CollapsibleGroup>

        {/* 高级采样 (19) */}
        <CollapsibleGroup title={t.configPage.subAdvSampling} onReset={() => resetGroup('advancedSampling')} disabled={isEmbedding}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Select label={t.configPage.mirostat} value={local.mirostat.toString()} onChange={v => set('mirostat', parseInt(v))} options={['0', '1', '2']} title={t.configPage.mirostatTip} defaultLabel={t.common.default} />
            <Num label={t.configPage.mirostatLr} value={local.mirostat_lr} onChange={v => set('mirostat_lr', v)} min={0.001} max={1} step={0.001} title={t.configPage.mirostatLrTip} />
            <Num label={t.configPage.mirostatEnt} value={local.mirostat_ent} onChange={v => set('mirostat_ent', v)} min={0} max={10} step={0.1} title={t.configPage.mirostatEntTip} />
            <Num label={t.configPage.xtcProbability} value={local.xtc_probability} onChange={v => set('xtc_probability', v)} min={0} max={1} step={0.05} title={t.configPage.xtcProbabilityTip} />
            <Num label={t.configPage.xtcThreshold} value={local.xtc_threshold} onChange={v => set('xtc_threshold', v)} min={0} max={1} step={0.05} title={t.configPage.xtcThresholdTip} />
            <Num label={t.configPage.dynatempRange} value={local.dynatemp_range} onChange={v => set('dynatemp_range', v)} min={0} max={10} step={0.1} title={t.configPage.dynatempRangeTip} />
            <Num label={t.configPage.dynatempExp} value={local.dynatemp_exp} onChange={v => set('dynatemp_exp', v)} min={0} max={10} step={0.1} title={t.configPage.dynatempExpTip} />
            <Num label={t.configPage.typicalP} value={local.typical_p} onChange={v => set('typical_p', v)} min={0} max={1} step={0.05} title={t.configPage.typicalPTip} />
            <Num label={t.configPage.dryMultiplier} value={local.dry_multiplier} onChange={v => set('dry_multiplier', v)} min={0} max={10} step={0.1} title={t.configPage.dryMultiplierTip} />
            <Num label={t.configPage.dryBase} value={local.dry_base} onChange={v => set('dry_base', v)} min={0} max={10} step={0.1} title={t.configPage.dryBaseTip} />
            <Num label={t.configPage.dryAllowedLength} value={local.dry_allowed_length} onChange={v => set('dry_allowed_length', v)} min={0} step={1} title={t.configPage.dryAllowedLengthTip} />
            <Num label={t.configPage.dryPenaltyLastN} value={local.dry_penalty_last_n} onChange={v => set('dry_penalty_last_n', v)} min={-1} title={t.configPage.dryPenaltyLastNTip} />
            <Input label={t.configPage.drySeqBreaker} value={local.dry_sequence_breaker} onChange={v => set('dry_sequence_breaker', v)} title={t.configPage.drySeqBreakerTip} />
            <Num label={t.configPage.adaptiveTarget} value={local.adaptive_target} onChange={v => set('adaptive_target', v)} min={0} max={10} step={0.1} title={t.configPage.adaptiveTargetTip} />
            <Num label={t.configPage.adaptiveDecay} value={local.adaptive_decay} onChange={v => set('adaptive_decay', v)} min={0} max={1} step={0.01} title={t.configPage.adaptiveDecayTip} />
            <Num label={t.configPage.topNSigma} value={local.top_n_sigma} onChange={v => set('top_n_sigma', v)} min={-1} max={10} step={0.1} title={t.configPage.topNSigmaTip} />
            <Input label={t.configPage.logitBias} value={local.logit_bias} onChange={v => set('logit_bias', v)} title={t.configPage.logitBiasTip} />
            <Input label={t.configPage.samplers} value={local.samplers} onChange={v => set('samplers', v)} title={t.configPage.samplersTip} />
            <Input label={t.configPage.samplerSeq} value={local.sampler_seq} onChange={v => set('sampler_seq', v)} title={t.configPage.samplerSeqTip} />
          </div>
        </CollapsibleGroup>

        {/* 采样参数扩展 (9) */}
        <CollapsibleGroup title={t.configPage.subAdvSamplingExt} onReset={() => resetGroup('advancedSamplingExt')} disabled={isEmbedding}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Num label={t.configPage.seed} value={local.seed} onChange={v => set('seed', v)} min={-1} title={t.configPage.seedTip} />
            <Num label={t.configPage.minP} value={local.min_p} onChange={v => set('min_p', v)} min={0} max={1} step={0.05} title={t.configPage.minPTip} />
            <Num label={t.configPage.presencePenalty} value={local.presence_penalty} onChange={v => set('presence_penalty', v)} min={0} max={2} step={0.1} title={t.configPage.presencePenaltyTip} />
            <Num label={t.configPage.frequencyPenalty} value={local.frequency_penalty} onChange={v => set('frequency_penalty', v)} min={0} max={2} step={0.1} title={t.configPage.frequencyPenaltyTip} />
            <Num label={t.configPage.repeatLastN} value={local.repeat_last_n} onChange={v => set('repeat_last_n', v)} min={-1} title={t.configPage.repeatLastNTip} />
            <Toggle label={t.configPage.special} value={local.special} onChange={v => set('special', v)} title={t.configPage.specialTip} />
            <Toggle label={t.configPage.spmInfill} value={local.spm_infill} onChange={v => set('spm_infill', v)} title={t.configPage.spmInfillTip} />
            <Toggle label={t.configPage.backendSampling} value={local.backend_sampling} onChange={v => set('backend_sampling', v)} title={t.configPage.backendSamplingTip} />
            <Input label={t.configPage.jsonSchema} value={local.json_schema} onChange={v => set('json_schema', v)} title={t.configPage.jsonSchemaTip} />
          </div>
        </CollapsibleGroup>

        {/* 推测解码 (9) */}
        <CollapsibleGroup title={t.configPage.subAdvSpec} onReset={() => resetGroup('advancedSpec')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Input label={t.configPage.draftModel} value={local.draft_model_path} onChange={v => set('draft_model_path', v)} title={t.configPage.draftModelTip} disabled={isEmbedding} />
            <Num label={t.configPage.draftGpu} value={local.draft_gpu_layers} onChange={v => set('draft_gpu_layers', v)} min={0} max={99} title={t.configPage.draftGpuTip} disabled={isEmbedding} />
            <Num label={t.configPage.specDraftPMin} value={local.spec_draft_p_min} onChange={v => set('spec_draft_p_min', v)} min={0} max={1} step={0.05} title={t.configPage.specDraftPMinTip} disabled={isEmbedding} />
            <Num label={t.configPage.specDraftPSplit} value={local.spec_draft_p_split} onChange={v => set('spec_draft_p_split', v)} min={0} max={1} step={0.05} title={t.configPage.specDraftPSplitTip} disabled={isEmbedding} />
            <Input label={t.configPage.specDraftDevice} value={local.spec_draft_device} onChange={v => set('spec_draft_device', v)} title={t.configPage.specDraftDeviceTip} disabled={isEmbedding} />
            <Input label={t.configPage.lookupCacheStatic} value={local.lookup_cache_static} onChange={v => set('lookup_cache_static', v)} title={t.configPage.lookupCacheStaticTip} disabled={isEmbedding} />
            <Input label={t.configPage.lookupCacheDynamic} value={local.lookup_cache_dynamic} onChange={v => set('lookup_cache_dynamic', v)} title={t.configPage.lookupCacheDynamicTip} disabled={isEmbedding} />
            <Select label={t.configPage.cacheTypeDraftK} value={local.cache_type_draft_k} onChange={v => set('cache_type_draft_k', v)} options={cacheTypes} title={t.configPage.cacheTypeDraftKTip} defaultLabel={t.common.default} />
            <Select label={t.configPage.cacheTypeDraftV} value={local.cache_type_draft_v} onChange={v => set('cache_type_draft_v', v)} options={cacheTypes} title={t.configPage.cacheTypeDraftVTip} defaultLabel={t.common.default} />
          </div>
        </CollapsibleGroup>

        {/* 上下文缩放 / RoPE · YaRN (8) */}
        <CollapsibleGroup title={t.configPage.subAdvRope} onReset={() => resetGroup('advancedRope')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Select label={t.configPage.ropeScaling} value={local.rope_scaling} onChange={v => set('rope_scaling', v)} options={['', 'none', 'linear', 'yarn']} title={t.configPage.ropeScalingTip} defaultLabel={t.common.default} />
            <Num label={t.configPage.ropeScale} value={local.rope_scale} onChange={v => set('rope_scale', v)} min={0} max={10} step={0.1} title={t.configPage.ropeScaleTip} />
            <Num label={t.configPage.ropeFreqBase} value={local.rope_freq_base} onChange={v => set('rope_freq_base', v)} min={0} title={t.configPage.ropeFreqBaseTip} />
            <Num label={t.configPage.ropeFreqScale} value={local.rope_freq_scale} onChange={v => set('rope_freq_scale', v)} min={0} max={10} step={0.1} title={t.configPage.ropeFreqScaleTip} />
            <Num label={t.configPage.yarnExtFactor} value={local.yarn_ext_factor} onChange={v => set('yarn_ext_factor', v)} min={-1} max={10} step={0.1} title={t.configPage.yarnExtFactorTip} />
            <Num label={t.configPage.yarnAttnFactor} value={local.yarn_attn_factor} onChange={v => set('yarn_attn_factor', v)} min={0} max={10} step={0.1} title={t.configPage.yarnAttnFactorTip} />
            <Num label={t.configPage.yarnBetaSlow} value={local.yarn_beta_slow} onChange={v => set('yarn_beta_slow', v)} min={0} max={10} step={0.1} title={t.configPage.yarnBetaSlowTip} />
            <Num label={t.configPage.yarnBetaFast} value={local.yarn_beta_fast} onChange={v => set('yarn_beta_fast', v)} min={0} max={128} title={t.configPage.yarnBetaFastTip} />
          </div>
        </CollapsibleGroup>

        {/* KV 缓存 (8) */}
        <CollapsibleGroup title={t.configPage.subAdvKvCache} onReset={() => resetGroup('advancedKvCache')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Select label={t.configPage.cacheTypeK} value={local.cache_type_k} onChange={v => set('cache_type_k', v)} options={cacheTypes} title={t.configPage.cacheTypeKTip} defaultLabel={t.common.default} />
            <Select label={t.configPage.cacheTypeV} value={local.cache_type_v} onChange={v => set('cache_type_v', v)} options={cacheTypes} title={t.configPage.cacheTypeVTip} defaultLabel={t.common.default} />
            <Toggle label={t.configPage.cachePrompt} value={local.cache_prompt} onChange={v => set('cache_prompt', v)} title={t.configPage.cachePromptTip} />
            <Num label={t.configPage.cacheReuse} value={local.cache_reuse} onChange={v => set('cache_reuse', v)} min={0} title={t.configPage.cacheReuseTip} />
            <Num label={t.configPage.cacheRam} value={local.cache_ram} onChange={v => set('cache_ram', v)} min={0} step={256} title={t.configPage.cacheRamTip} />
            <Toggle label={t.configPage.warmup} value={local.warmup} onChange={v => set('warmup', v)} title={t.configPage.warmupTip} />
            <Toggle label={t.configPage.cacheIdleSlots} value={local.cache_idle_slots} onChange={v => set('cache_idle_slots', v)} title={t.configPage.cacheIdleSlotsTip} />
            <Toggle label={t.configPage.kvUnified} value={local.kv_unified} onChange={v => set('kv_unified', v)} title={t.configPage.kvUnifiedTip} />
          </div>
        </CollapsibleGroup>

        {/* 上下文管理 (7) */}
        <CollapsibleGroup title={t.configPage.subAdvContextMgmt} onReset={() => resetGroup('advancedContextMgmt')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Num label={t.configPage.ctxCheckpoints} value={local.ctx_checkpoints} onChange={v => set('ctx_checkpoints', v)} min={0} title={t.configPage.ctxCheckpointsTip} />
            <Num label={t.configPage.checkpointMinStep} value={local.checkpoint_min_step} onChange={v => set('checkpoint_min_step', v)} min={0} title={t.configPage.checkpointMinStepTip} />
            <Toggle label={t.configPage.contextShift} value={local.context_shift} onChange={v => set('context_shift', v)} title={t.configPage.contextShiftTip} disabled={isEmbedding} />
            <Toggle label={t.configPage.swaFull} value={local.swa_full} onChange={v => set('swa_full', v)} title={t.configPage.swaFullTip} />
            <Toggle label={t.configPage.ctxAuto} value={local.ctx_size_auto} onChange={v => set('ctx_size_auto', v)} title={t.configPage.ctxAutoTip} />
            <Num label={t.configPage.keep} value={local.keep} onChange={v => set('keep', v)} min={0} title={t.configPage.keepTip} />
            <Input label={t.configPage.overrideKv} value={local.override_kv} onChange={v => set('override_kv', v)} title={t.configPage.overrideKvTip} />
          </div>
        </CollapsibleGroup>

        {/* 硬件配置 (9) */}
        <CollapsibleGroup title={t.configPage.subAdvHardware} onReset={() => resetGroup('advancedHardware')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Num label={t.configPage.moeCpu} value={local.moe_cpu_layers} onChange={v => set('moe_cpu_layers', v)} min={0} max={99} title={t.configPage.moeCpuTip} disabled={isEmbedding} />
            <Input label={t.configPage.device} value={local.device} onChange={v => set('device', v)} title={t.configPage.deviceTip} />
            <Select label={t.configPage.splitMode} value={local.split_mode} onChange={v => set('split_mode', v)} options={['', 'none', 'layer', 'row']} title={t.configPage.splitModeTip} defaultLabel={t.common.default} />
            <Input label={t.configPage.tensorSplit} value={local.tensor_split} onChange={v => set('tensor_split', v)} title={t.configPage.tensorSplitTip} />
            <Num label={t.configPage.mainGpu} value={local.main_gpu} onChange={v => set('main_gpu', v)} min={0} max={9} title={t.configPage.mainGpuTip} />
            <Toggle label={t.configPage.checkTensors} value={local.check_tensors} onChange={v => set('check_tensors', v)} title={t.configPage.checkTensorsTip} />
            <Toggle label={t.configPage.fit} value={local.fit} onChange={v => set('fit', v)} title={t.configPage.fitTip} />
            <Num label={t.configPage.threadsBatch} value={local.threads_batch} onChange={v => set('threads_batch', v)} min={0} title={t.configPage.threadsBatchTip} />
            <Num label={t.configPage.threadsHttp} value={local.threads_http} onChange={v => set('threads_http', v)} min={-1} title={t.configPage.threadsHttpTip} />
          </div>
        </CollapsibleGroup>

        {/* 服务基础 (10) */}
        <CollapsibleGroup title={t.configPage.subAdvServer} onReset={() => resetGroup('advancedServerBasic')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Input label={t.configPage.apiKey} value={local.api_key} onChange={v => set('api_key', v)} type="password" title={t.configPage.apiKeyTip} />
            <Input label={t.configPage.apiKeyFile} value={local.api_key_file} onChange={v => set('api_key_file', v)} title={t.configPage.apiKeyFileTip} />
            <Toggle label={t.configPage.noUi} value={local.no_ui} onChange={v => set('no_ui', v)} title={t.configPage.noUiTip} />
            <Input label={t.configPage.pathPrefix} value={local.path_prefix} onChange={v => set('path_prefix', v)} title={t.configPage.pathPrefixTip} />
            <Input label={t.configPage.apiPrefix} value={local.api_prefix} onChange={v => set('api_prefix', v)} title={t.configPage.apiPrefixTip} />
            <Num label={t.configPage.timeout} value={local.timeout} onChange={v => set('timeout', v)} min={1} title={t.configPage.timeoutTip} />
            <Num label={t.configPage.sleepIdle} value={local.sleep_idle} onChange={v => set('sleep_idle', v)} min={-1} title={t.configPage.sleepIdleTip} />
            <Toggle label={t.configPage.verbose} value={local.verbose} onChange={v => set('verbose', v)} title={t.configPage.verboseTip} />
          </div>
          <div className="grid grid-cols-2 gap-3 mt-3">
            <Input label={t.configPage.sslKey} value={local.ssl_key_file} onChange={v => set('ssl_key_file', v)} title={t.configPage.sslKeyTip} />
            <Input label={t.configPage.sslCert} value={local.ssl_cert_file} onChange={v => set('ssl_cert_file', v)} title={t.configPage.sslCertTip} />
          </div>
        </CollapsibleGroup>

        {/* 服务扩展 (7) */}
        <CollapsibleGroup title={t.configPage.subAdvServerExt} onReset={() => resetGroup('advancedServerExt')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Toggle label={t.configPage.slotsEnabled} value={local.slots_enabled} onChange={v => set('slots_enabled', v)} title={t.configPage.slotsEnabledTip} />
            <Toggle label={t.configPage.metrics} value={local.metrics} onChange={v => set('metrics', v)} title={t.configPage.metricsTip} />
            <Toggle label={t.configPage.props} value={local.props} onChange={v => set('props', v)} title={t.configPage.propsTip} />
            <Input label={t.configPage.slotSavePath} value={local.slot_save_path} onChange={v => set('slot_save_path', v)} title={t.configPage.slotSavePathTip} />
            <Num label={t.configPage.slotPromptSimilarity} value={local.slot_prompt_similarity} onChange={v => set('slot_prompt_similarity', v)} min={0} max={1} step={0.05} title={t.configPage.slotPromptSimilarityTip} />
            <Input label={t.configPage.prefillAssistant} value={local.prefill_assistant} onChange={v => set('prefill_assistant', v)} title={t.configPage.prefillAssistantTip} />
            <Input label={t.configPage.uiConfigFile} value={local.ui_config_file} onChange={v => set('ui_config_file', v)} title={t.configPage.uiConfigFileTip} />
          </div>
        </CollapsibleGroup>

        {/* 多模型/专家 (11) */}
        <CollapsibleGroup title={t.configPage.subAdvMulti} onReset={() => resetGroup('advancedMulti')}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Input label={t.configPage.modelsDir} value={local.models_dir} onChange={v => set('models_dir', v)} title={t.configPage.modelsDirTip} />
            <Input label={t.configPage.modelsPreset} value={local.models_preset} onChange={v => set('models_preset', v)} title={t.configPage.modelsPresetTip} />
            <Num label={t.configPage.modelsMax} value={local.models_max} onChange={v => set('models_max', v)} min={1} max={99} title={t.configPage.modelsMaxTip} />
            <Toggle label={t.configPage.modelsAutoload} value={local.models_autoload} onChange={v => set('models_autoload', v)} title={t.configPage.modelsAutoloadTip} />
            <Input label={t.configPage.mmprojUrl} value={local.mmproj_url} onChange={v => set('mmproj_url', v)} title={t.configPage.mmprojUrlTip} disabled={isEmbedding} />
            <Toggle label={t.configPage.mmprojAuto} value={local.mmproj_auto} onChange={v => set('mmproj_auto', v)} title={t.configPage.mmprojAutoTip} disabled={isEmbedding} />
            <Num label={t.configPage.imageMinTokens} value={local.image_min_tokens} onChange={v => set('image_min_tokens', v)} min={0} title={t.configPage.imageMinTokensTip} disabled={isEmbedding} />
            <Num label={t.configPage.imageMaxTokens} value={local.image_max_tokens} onChange={v => set('image_max_tokens', v)} min={0} title={t.configPage.imageMaxTokensTip} disabled={isEmbedding} />
            <Input label={t.configPage.mediaPath} value={local.media_path} onChange={v => set('media_path', v)} title={t.configPage.mediaPathTip} />
            <Input label={t.configPage.tags} value={local.tags} onChange={v => set('tags', v)} title={t.configPage.tagsTip} />
            <Input label={t.configPage.tools} value={local.tools} onChange={v => set('tools', v)} title={t.configPage.toolsTip} />
          </div>
        </CollapsibleGroup>
      </div>
    </Section>
  )
}
