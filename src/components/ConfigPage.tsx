import { useState, useEffect } from 'react'
import { ChevronDown, ChevronRight, Settings, File, Image, X, FolderOpen } from 'lucide-react'
import { useAppStore, type InstanceConfig } from '../store'
import { useI18n } from '../i18n'
import { validateConfig, type Warning } from '../validators'

const cacheTypes = ['', 'f32', 'f16', 'bf16', 'q8_0', 'q4_0', 'q4_1', 'iq4_nl', 'q5_0', 'q5_1']
const specTypes = ['', 'none', 'mtp', 'draft-mtp', 'draft-simple', 'draft-eagle3', 'ngram-cache', 'ngram-simple', 'ngram-map-k', 'ngram-map-k4v', 'ngram-mod']
const chatTemplates = ['', 'bailing', 'chatglm3', 'chatglm4', 'chatml', 'command-r', 'deepseek', 'deepseek2', 'deepseek3', 'exaone3', 'gemma', 'gpt-oss', 'kimi-k2', 'llama2', 'llama3', 'llama4', 'mistral', 'openchat', 'phi3', 'phi4', 'vicuna', 'zephyr']

const Section = ({ title, children, disabled, onToggle, toggled }: { title: string; children: React.ReactNode; disabled?: boolean; onToggle?: (v: boolean) => void; toggled?: boolean }) => {
  const [open, setOpen] = useState(false)
  return (
    <div className="border dark:border-gray-700 rounded-lg overflow-hidden">
      <button onClick={() => setOpen(!open)} className="w-full flex items-center gap-2 px-4 py-2.5 bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors text-left dark:text-gray-200">
        {open ? <ChevronDown className="w-4 h-4 text-gray-400 shrink-0" /> : <ChevronRight className="w-4 h-4 text-gray-400 shrink-0" />}
        <span className="text-sm font-medium">{title}</span>
        {disabled && <span className="text-xs text-gray-400 ml-1">🛑</span>}
        {onToggle !== undefined && (
          <label className="ml-auto flex items-center gap-1 cursor-pointer shrink-0" onClick={e => e.stopPropagation()}>
            <span className="text-xs text-gray-400">{toggled ? 'On' : 'Off'}</span>
            <input type="checkbox" checked={toggled} onChange={e => { e.stopPropagation(); onToggle(e.target.checked) }} className="w-3.5 h-3.5 rounded" />
          </label>
        )}
      </button>
      {open && <div className={`px-4 py-3 space-y-3 ${disabled ? 'opacity-50 pointer-events-none' : ''}`}>{children}</div>}
    </div>
  )
}

const Input = ({ label, value, onChange, placeholder, type, title, disabled }: { label: string; value: string; onChange: (v: string) => void; placeholder?: string; type?: string; title?: string; disabled?: boolean }) => (
  <div title={title}>
    <label className="block text-xs font-medium mb-1 text-gray-500">{label}</label>
    <input type={type || 'text'} value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder} disabled={disabled} className={`w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`} />
  </div>
)

const Num = ({ label, value, onChange, min, max, step, title, disabled }: { label: string; value: number; onChange: (v: number) => void; min?: number; max?: number; step?: number; title?: string; disabled?: boolean }) => (
  <div title={title}>
    <label className="block text-xs font-medium mb-1 text-gray-500">{label}</label>
    <input type="number" value={value} min={min} max={max} step={step || 1} onChange={e => onChange(parseFloat(e.target.value) || 0)} disabled={disabled} className={`w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`} />
  </div>
)

const Toggle = ({ label, value, onChange, title, disabled }: { label: string; value: boolean; onChange: (v: boolean) => void; title?: string; disabled?: boolean }) => (
  <label className={`flex items-center gap-2 ${disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}`} title={title}>
    <input type="checkbox" checked={value} onChange={e => onChange(e.target.checked)} disabled={disabled} className="w-4 h-4 rounded" />
    <span className="text-sm">{label}</span>
  </label>
)

const Select = ({ label, value, onChange, options, title, disabled, defaultLabel }: { label: string; value: string; onChange: (v: string) => void; options: string[]; title?: string; disabled?: boolean; defaultLabel?: string }) => (
  <div title={title}>
    <label className="block text-xs font-medium mb-1 text-gray-500">{label}</label>
    <select value={value} onChange={e => onChange(e.target.value)} disabled={disabled} className={`w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}>
      {options.map(o => <option key={o} value={o}>{o || defaultLabel || '\u9ED8\u8BA4'}</option>)}
    </select>
  </div>
)

const SubGroup = ({ label, toggled, onToggle, children }: { label: string; toggled: boolean; onToggle: (v: boolean) => void; children: React.ReactNode }) => (
  <div>
    <label className="flex items-center gap-2 cursor-pointer mb-2">
      <input type="checkbox" checked={toggled} onChange={e => onToggle(e.target.checked)} className="w-3.5 h-3.5 rounded" />
      <span className="text-xs font-medium text-gray-500">{label}</span>
    </label>
    {toggled && children}
  </div>
)

const ConfigPage = () => {
  const { instances, activeConfigInstanceId, updateInstance, saveConfig, models, modelDirs, engines, defaultEngineId } = useAppStore()
  const { t } = useI18n()
  const inst = instances.find(i => i.id === activeConfigInstanceId)
  const [local, setLocal] = useState<InstanceConfig | null>(null)
  const [saved, setSaved] = useState(false)
  const [showPicker, setShowPicker] = useState(false)
  const [pickerCollapsed, setPickerCollapsed] = useState<Set<string>>(new Set())
  const [useAdvSampling, setUseAdvSampling] = useState(false)
  const [useSpecDecoding, setUseSpecDecoding] = useState(false)
  const [useExpertSettings, setUseExpertSettings] = useState(false)
  const [saveWarnings, setSaveWarnings] = useState<Warning[]>([])

  useEffect(() => { if (inst) setLocal({ ...inst.config }); else setLocal(null) }, [activeConfigInstanceId, instances])

  // Sync toggle states with actual config values on load
  useEffect(() => {
    if (!local) return
    setUseAdvSampling(
      local.mirostat !== 0 || local.mirostat_lr !== 0 || local.mirostat_ent !== 0 ||
      local.xtc_probability !== 0 || local.xtc_threshold !== 0 ||
      local.dynatemp_range !== 0 || local.dynatemp_exp !== 0 ||
      local.typical_p !== 1.0 ||
      local.dry_multiplier !== 0 || local.dry_base !== 0 || local.dry_allowed_length !== 0 ||
      local.dry_penalty_last_n !== 0 || local.dry_sequence_breaker !== '' ||
      local.adaptive_target !== 0 || local.adaptive_decay !== 0 ||
      local.top_n_sigma !== -1 ||
      local.logit_bias !== '' || local.samplers !== '' || local.sampler_seq !== ''
    )
    setUseSpecDecoding(
      local.draft_model_path !== '' || local.spec_type !== '' ||
      local.spec_draft_p_min !== 0 || local.spec_draft_p_split !== 0.1 ||
      local.spec_draft_device !== '' ||
      local.lookup_cache_static !== '' || local.lookup_cache_dynamic !== ''
    )
    setUseExpertSettings(
      local.device !== '' || local.split_mode !== '' || local.tensor_split !== '' ||
      local.main_gpu !== 0 || local.override_kv !== '' ||
      local.models_dir !== '' || local.models_preset !== '' || local.models_max !== 4 ||
      local.models_autoload !== false ||
      local.mmproj_url !== '' || local.mmproj_auto !== false ||
      local.image_min_tokens !== 0 || local.image_max_tokens !== 0 ||
      local.media_path !== '' || local.tags !== '' || local.tools !== ''
    )
  }, [local])

  const EMBED_ARCHS = ['bge', 'gte', 'e5', 'text-embedding', 'sentence-bert', 'sentence-t5', 'instructor', 'bert', 'nomic', 'jina']
  const isEmbedding = (() => {
    if (!local?.model_path) return false
    const fname = local.model_path.replace(/\\/g, '/').split('/').pop() || ''
    if (fname.toLowerCase().includes('embed')) return true
    const model = models.find(m => m.path === local.model_path)
    if (model?.architecture && EMBED_ARCHS.some(a => model.architecture!.toLowerCase().includes(a))) return true
    return false
  })()

  useEffect(() => { if (isEmbedding && local) { if (!local.embedding) set('embedding', true); if (!local.pooling) set('pooling', 'mean') } }, [isEmbedding, local?.model_path])

  if (!local) return (
    <div className="bg-white dark:bg-gray-800 rounded-lg p-12 text-center border dark:border-gray-700">
      <Settings className="w-12 h-12 mx-auto mb-3 text-gray-400" />
      <p className="text-gray-500">{t.configPage.noInstance}</p>
    </div>
  )

  const set = (k: keyof InstanceConfig, v: any) => setLocal(l => l ? { ...l, [k]: v } : l)
  const pickModel = (modelPath: string) => {
    set('model_path', modelPath)
    const dir = modelPath.replace(/[/\\][^/\\]*$/, '')
    const mmproj = models.find(m => { const mDir = m.path.replace(/[/\\][^/\\]*$/, ''); return m.file_type === 'mmproj' && mDir === dir })
    if (mmproj) set('mmproj_path', mmproj.path); else set('mmproj_path', '')
    setShowPicker(false)
  }
  const save = () => {
    if (!local || !inst) return
    const model = models.find(m => m.path === local.model_path)
    const engine = engines.find(e => e.id === (local.engine_id || defaultEngineId || '')) || engines[0]
    const warnings = validateConfig(local, model, engine)
    updateInstance(inst.id, { config: local })
    saveConfig()
    setSaved(true)
    setSaveWarnings(warnings)
    setTimeout(() => { setSaved(false); setSaveWarnings([]) }, 6000)
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Settings className="w-5 h-5 text-blue-500" />
          <span className="text-lg font-semibold">{t.configPage.title}</span>
          <span className="text-sm text-gray-500">{'\u2014'} {inst?.name}</span>
        </div>
        <button onClick={save} className="px-6 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors">{saved ? t.configPage.saved : t.configPage.save}</button>
      </div>
      {saved && <div className="bg-green-50 dark:bg-green-900/20 rounded-lg p-3 text-sm text-green-600 dark:text-green-400">{t.configPage.savedMsg}{'\u300C'}{inst?.name}{'\u300D\u3002'}{t.configPage.savedHint}</div>}
      {isEmbedding && <div className="bg-blue-50 dark:bg-blue-900/20 rounded-lg p-3 text-sm text-blue-600 dark:text-blue-400">{t.configPage.embeddingBanner}</div>}
      {saveWarnings.length > 0 && (
        <div className="space-y-1.5">
          {saveWarnings.filter(w => w.severity === 'high').map((w, i) => (
            <div key={`h-${i}`} className="bg-red-50 dark:bg-red-900/20 rounded-lg px-3 py-2 text-sm text-red-600 dark:text-red-400">
              {'\u26A0'} {(t.configPage as any)[w.key] || w.key}
            </div>
          ))}
          {saveWarnings.filter(w => w.severity === 'medium').map((w, i) => (
            <div key={`m-${i}`} className="bg-yellow-50 dark:bg-yellow-900/20 rounded-lg px-3 py-2 text-sm text-yellow-700 dark:text-yellow-400">
              {(t.configPage as any)[w.key] || w.key}
            </div>
          ))}
          {saveWarnings.filter(w => w.severity === 'low').map((w, i) => (
            <div key={`l-${i}`} className="bg-sky-50 dark:bg-sky-900/20 rounded-lg px-3 py-2 text-sm text-sky-600 dark:text-sky-400">
              {(t.configPage as any)[w.key] || w.key}
            </div>
          ))}
        </div>
      )}

      <div className="space-y-3">
        {/* ════════════════════════ Section 1: Basic ════════════════════════ */}
        <Section title={t.configPage.basic}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <div title={t.configPage.modelPathTip}>
              <label className="block text-xs font-medium mb-1 text-gray-500">{t.configPage.modelPath}</label>
              <div className="flex gap-1">
                <input type="text" value={local.model_path} onChange={e => set('model_path', e.target.value)} className="flex-1 px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
                <button onClick={() => setShowPicker(true)} className="px-2 py-1.5 bg-blue-600 hover:bg-blue-700 text-white rounded-lg text-xs" title={t.configPage.modelPathBtn}>{'\uD83D\uDCC2'}</button>
              </div>
            </div>
            <Input label={t.configPage.alias} value={local.alias} onChange={v => set('alias', v)} title={t.configPage.aliasTip} />
            <Input label={t.configPage.lora} value={local.lora_path} onChange={v => set('lora_path', v)} title={t.configPage.loraTip} disabled={isEmbedding} />
            <Input label={t.configPage.mmproj} value={local.mmproj_path} onChange={v => set('mmproj_path', v)} title={t.configPage.mmprojTip} disabled={isEmbedding} />
            <Input label={t.configPage.grammarFile} value={local.grammar_file} onChange={v => set('grammar_file', v)} title={t.configPage.grammarFileTip} disabled={isEmbedding} />
            <Input label={t.configPage.grammar} value={local.grammar} onChange={v => set('grammar', v)} title={t.configPage.grammarTip} disabled={isEmbedding} />
            <Select label={t.configPage.chatTemplate} value={local.chat_template} onChange={v => set('chat_template', v)} options={chatTemplates} title={t.configPage.chatTemplateTip} disabled={isEmbedding} defaultLabel={t.common.default} />
            <Input label={t.configPage.chatTemplateFile} value={local.chat_template_file} onChange={v => set('chat_template_file', v)} title={t.configPage.chatTemplateFileTip} disabled={isEmbedding} />
            <Select label={t.configPage.reasoningFormat} value={local.reasoning_format} onChange={v => set('reasoning_format', v)} options={['', 'none', 'deepseek', 'deepseek-legacy']} title={t.configPage.reasoningFormatTip} disabled={isEmbedding} defaultLabel={t.common.default} />
            <Select label={t.configPage.reasoning} value={local.reasoning} onChange={v => set('reasoning', v)} options={['', 'on', 'off', 'auto']} title={t.configPage.reasoningTip} disabled={isEmbedding} defaultLabel={t.common.default} />
            <Toggle label={t.configPage.jinja} value={local.jinja} onChange={v => set('jinja', v)} title={t.configPage.jinjaTip} disabled={isEmbedding} />
            <Toggle label={t.configPage.skipChatParsing} value={local.skip_chat_parsing} onChange={v => set('skip_chat_parsing', v)} title={t.configPage.skipChatParsingTip} disabled={isEmbedding} />
            <Select label={t.configPage.reasoningEffort} value={local.reasoning_effort} onChange={v => set('reasoning_effort', v)} options={['', 'low', 'medium', 'high']} title={t.configPage.reasoningEffortTip} disabled={isEmbedding} defaultLabel={t.common.default} />
            <Num label={t.configPage.reasoningBudget} value={local.reasoning_budget ? parseInt(local.reasoning_budget) : 0} onChange={v => set('reasoning_budget', v.toString())} min={0} max={65536} step={256} title={t.configPage.reasoningBudgetTip} disabled={isEmbedding} />
            <Input label={t.configPage.reasoningBudgetMsg} value={local.reasoning_budget_message} onChange={v => set('reasoning_budget_message', v)} title={t.configPage.reasoningBudgetMsgTip} disabled={isEmbedding} />
          </div>
        </Section>

        {/* ════════════════════════ Section 2: Generation ════════════════════════ */}
        <Section title={t.configPage.generation} disabled={isEmbedding}>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Num label={t.configPage.nPredict} value={local.n_predict} onChange={v => set('n_predict', v)} min={-1} title={t.configPage.nPredictTip} />
            <Toggle label={t.configPage.ignoreEos} value={local.ignore_eos} onChange={v => set('ignore_eos', v)} title={t.configPage.ignoreEosTip} />
            <Num label={t.configPage.seed} value={local.seed} onChange={v => set('seed', v)} min={-1} title={t.configPage.seedTip} />
            <Input label={t.configPage.jsonSchema} value={local.json_schema} onChange={v => set('json_schema', v)} title={t.configPage.jsonSchemaTip} />
            <Num label={t.configPage.temp} value={local.temp} onChange={v => set('temp', v)} min={0} max={2} step={0.1} title={t.configPage.tempTip} />
            <Num label={t.configPage.topK} value={local.top_k} onChange={v => set('top_k', v)} min={0} title={t.configPage.topKTip} />
            <Num label={t.configPage.topP} value={local.top_p} onChange={v => set('top_p', v)} min={0} max={1} step={0.1} title={t.configPage.topPTip} />
            <Num label={t.configPage.minP} value={local.min_p} onChange={v => set('min_p', v)} min={0} max={1} step={0.05} title={t.configPage.minPTip} />
            <Num label={t.configPage.repeatPenalty} value={local.repeat_penalty} onChange={v => set('repeat_penalty', v)} min={0} max={2} step={0.1} title={t.configPage.repeatPenaltyTip} />
            <Num label={t.configPage.repeatLastN} value={local.repeat_last_n} onChange={v => set('repeat_last_n', v)} min={-1} title={t.configPage.repeatLastNTip} />
            <Num label={t.configPage.presencePenalty} value={local.presence_penalty} onChange={v => set('presence_penalty', v)} min={0} max={2} step={0.1} title={t.configPage.presencePenaltyTip} />
            <Num label={t.configPage.frequencyPenalty} value={local.frequency_penalty} onChange={v => set('frequency_penalty', v)} min={0} max={2} step={0.1} title={t.configPage.frequencyPenaltyTip} />
            <Input label={t.configPage.reversePrompt} value={local.reverse_prompt} onChange={v => set('reverse_prompt', v)} title={t.configPage.reversePromptTip} />
            <Toggle label={t.configPage.special} value={local.special} onChange={v => set('special', v)} title={t.configPage.specialTip} />
            <Toggle label={t.configPage.spmInfill} value={local.spm_infill} onChange={v => set('spm_infill', v)} title={t.configPage.spmInfillTip} />
            <Toggle label={t.configPage.backendSampling} value={local.backend_sampling} onChange={v => set('backend_sampling', v)} title={t.configPage.backendSamplingTip} />
          </div>
        </Section>

        {/* ════════════════════════ Section 3: Advanced Sampling ════════════════════════ */}
        <Section title={t.configPage.advancedSampling} disabled={isEmbedding || !useAdvSampling} onToggle={v => { setUseAdvSampling(v); if (!v) { set('mirostat', 0); set('mirostat_lr', 0); set('mirostat_ent', 0); set('xtc_probability', 0); set('xtc_threshold', 0); set('dynatemp_range', 0); set('dynatemp_exp', 0); set('typical_p', 1.0); set('dry_multiplier', 0); set('dry_base', 0); set('dry_allowed_length', 0); set('dry_penalty_last_n', 0); set('dry_sequence_breaker', ''); set('adaptive_target', 0); set('adaptive_decay', 0); set('top_n_sigma', -1); set('logit_bias', ''); set('samplers', ''); set('sampler_seq', '') } }} toggled={useAdvSampling}>
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
        </Section>

        {/* ════════════════════════ Section 4: Performance & Context ════════════════════════ */}
        <Section title={t.configPage.performance}>
          {/* Compute */}
          <div className="text-xs font-medium text-gray-400 mb-2">{t.configPage.subCompute}</div>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Num label={t.configPage.threads} value={local.threads} onChange={v => set('threads', v)} min={0} title={t.configPage.threadsTip} />
            <Num label={t.configPage.threadsBatch} value={local.threads_batch} onChange={v => set('threads_batch', v)} min={0} title={t.configPage.threadsBatchTip} />
            <Num label={t.configPage.threadsHttp} value={local.threads_http} onChange={v => set('threads_http', v)} min={-1} title={t.configPage.threadsHttpTip} />
            <Num label={t.configPage.gpuLayers} value={local.gpu_layers} onChange={v => set('gpu_layers', v)} min={0} max={99} title={t.configPage.gpuLayersTip} />
            <Num label={t.configPage.batchSize} value={local.batch_size} onChange={v => set('batch_size', v)} min={1} title={t.configPage.batchSizeTip} />
            <Num label={t.configPage.ubatchSize} value={local.ubatch_size} onChange={v => set('ubatch_size', v)} min={1} title={t.configPage.ubatchSizeTip} />
            <Num label={t.configPage.parallel} value={local.parallel} onChange={v => set('parallel', v)} min={-1} title={t.configPage.parallelTip} />
            <Toggle label={t.configPage.contBatching} value={local.cont_batching} onChange={v => set('cont_batching', v)} title={t.configPage.contBatchingTip} />
          </div>
          {/* Cache */}
          <div className="text-xs font-medium text-gray-400 mt-3 mb-2">{t.configPage.subCache}</div>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Toggle label={t.configPage.cachePrompt} value={local.cache_prompt} onChange={v => set('cache_prompt', v)} title={t.configPage.cachePromptTip} />
            <Num label={t.configPage.cacheReuse} value={local.cache_reuse} onChange={v => set('cache_reuse', v)} min={0} title={t.configPage.cacheReuseTip} />
            <Num label={t.configPage.cacheRam} value={local.cache_ram} onChange={v => set('cache_ram', v)} min={0} step={256} title={t.configPage.cacheRamTip} />
            <Toggle label={t.configPage.warmup} value={local.warmup} onChange={v => set('warmup', v)} title={t.configPage.warmupTip} />
          </div>
          {/* Context Window */}
          <div className="text-xs font-medium text-gray-400 mt-3 mb-2">{t.configPage.subContext}</div>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Toggle label={t.configPage.ctxAuto} value={local.ctx_size_auto} onChange={v => set('ctx_size_auto', v)} title={t.configPage.ctxAutoTip} />
            {!local.ctx_size_auto && <Num label={t.configPage.ctxSize} value={local.ctx_size} onChange={v => set('ctx_size', v)} min={0} step={1024} title={t.configPage.ctxSizeTip} />}
            <Num label={t.configPage.keep} value={local.keep} onChange={v => set('keep', v)} min={0} title={t.configPage.keepTip} />
            <Toggle label={t.configPage.swaFull} value={local.swa_full} onChange={v => set('swa_full', v)} title={t.configPage.swaFullTip} />
            <Toggle label={t.configPage.contextShift} value={local.context_shift} onChange={v => set('context_shift', v)} title={t.configPage.contextShiftTip} disabled={isEmbedding} />
            <Num label={t.configPage.ctxCheckpoints} value={local.ctx_checkpoints} onChange={v => set('ctx_checkpoints', v)} min={0} title={t.configPage.ctxCheckpointsTip} />
            <Num label={t.configPage.checkpointMinStep} value={local.checkpoint_min_step} onChange={v => set('checkpoint_min_step', v)} min={0} title={t.configPage.checkpointMinStepTip} />
          </div>
          {/* RoPE */}
          <div className="text-xs font-medium text-gray-400 mt-3 mb-2">{t.configPage.subRope}</div>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Select label={t.configPage.ropeScaling} value={local.rope_scaling} onChange={v => set('rope_scaling', v)} options={['', 'none', 'linear', 'yarn']} title={t.configPage.ropeScalingTip} defaultLabel={t.common.default} />
            <Num label={t.configPage.ropeScale} value={local.rope_scale} onChange={v => set('rope_scale', v)} min={0} max={10} step={0.1} title={t.configPage.ropeScaleTip} />
            <Num label={t.configPage.ropeFreqBase} value={local.rope_freq_base} onChange={v => set('rope_freq_base', v)} min={0} title={t.configPage.ropeFreqBaseTip} />
            <Num label={t.configPage.ropeFreqScale} value={local.rope_freq_scale} onChange={v => set('rope_freq_scale', v)} min={0} max={10} step={0.1} title={t.configPage.ropeFreqScaleTip} />
          </div>
          {/* YaRN */}
          <div className="text-xs font-medium text-gray-400 mt-3 mb-2">{t.configPage.subYarn}</div>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Num label={t.configPage.yarnExtFactor} value={local.yarn_ext_factor} onChange={v => set('yarn_ext_factor', v)} min={-1} max={10} step={0.1} title={t.configPage.yarnExtFactorTip} />
            <Num label={t.configPage.yarnAttnFactor} value={local.yarn_attn_factor} onChange={v => set('yarn_attn_factor', v)} min={0} max={10} step={0.1} title={t.configPage.yarnAttnFactorTip} />
            <Num label={t.configPage.yarnBetaSlow} value={local.yarn_beta_slow} onChange={v => set('yarn_beta_slow', v)} min={0} max={10} step={0.1} title={t.configPage.yarnBetaSlowTip} />
            <Num label={t.configPage.yarnBetaFast} value={local.yarn_beta_fast} onChange={v => set('yarn_beta_fast', v)} min={0} max={128} title={t.configPage.yarnBetaFastTip} />
          </div>
        </Section>

        {/* ════════════════════════ Section 5: Server & Network ════════════════════════ */}
        <Section title={t.configPage.network}>
          {/* Network */}
          <div className="text-xs font-medium text-gray-400 mb-2">{t.configPage.subNetwork}</div>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Input label={t.configPage.host} value={local.host} onChange={v => set('host', v)} title={t.configPage.hostTip} />
            <Num label={t.configPage.portLabel} value={local.port} onChange={v => set('port', v)} min={1} max={65535} title={t.configPage.portLabelTip} />
            <Input label={t.configPage.apiKey} value={local.api_key} onChange={v => set('api_key', v)} type="password" title={t.configPage.apiKeyTip} />
            <Input label={t.configPage.apiKeyFile} value={local.api_key_file} onChange={v => set('api_key_file', v)} title={t.configPage.apiKeyFileTip} />
            <Toggle label={t.configPage.noUi} value={local.no_ui} onChange={v => set('no_ui', v)} title={t.configPage.noUiTip} />
            <Input label={t.configPage.pathPrefix} value={local.path_prefix} onChange={v => set('path_prefix', v)} title={t.configPage.pathPrefixTip} />
            <Input label={t.configPage.apiPrefix} value={local.api_prefix} onChange={v => set('api_prefix', v)} title={t.configPage.apiPrefixTip} />
            <Input label={t.configPage.uiConfigFile} value={local.ui_config_file} onChange={v => set('ui_config_file', v)} title={t.configPage.uiConfigFileTip} />
          </div>
          {/* SSL */}
          <div className="grid grid-cols-2 gap-3 mt-3">
            <Input label={t.configPage.sslKey} value={local.ssl_key_file} onChange={v => set('ssl_key_file', v)} title={t.configPage.sslKeyTip} />
            <Input label={t.configPage.sslCert} value={local.ssl_cert_file} onChange={v => set('ssl_cert_file', v)} title={t.configPage.sslCertTip} />
          </div>
          {/* Server Features */}
          <div className="text-xs font-medium text-gray-400 mt-3 mb-2">{t.configPage.subServer}</div>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Num label={t.configPage.timeout} value={local.timeout} onChange={v => set('timeout', v)} min={1} title={t.configPage.timeoutTip} />
            <Num label={t.configPage.sleepIdle} value={local.sleep_idle} onChange={v => set('sleep_idle', v)} min={-1} title={t.configPage.sleepIdleTip} />
            <Toggle label={t.configPage.verbose} value={local.verbose} onChange={v => set('verbose', v)} title={t.configPage.verboseTip} />
            <Toggle label={t.configPage.metrics} value={local.metrics} onChange={v => set('metrics', v)} title={t.configPage.metricsTip} />
            <Toggle label={t.configPage.props} value={local.props} onChange={v => set('props', v)} title={t.configPage.propsTip} />
            <Toggle label={t.configPage.slotsEnabled} value={local.slots_enabled} onChange={v => set('slots_enabled', v)} title={t.configPage.slotsEnabledTip} />
            <Input label={t.configPage.slotSavePath} value={local.slot_save_path} onChange={v => set('slot_save_path', v)} title={t.configPage.slotSavePathTip} />
            <Num label={t.configPage.slotPromptSimilarity} value={local.slot_prompt_similarity} onChange={v => set('slot_prompt_similarity', v)} min={0} max={1} step={0.05} title={t.configPage.slotPromptSimilarityTip} />
            <Input label={t.configPage.prefillAssistant} value={local.prefill_assistant} onChange={v => set('prefill_assistant', v)} title={t.configPage.prefillAssistantTip} />
          </div>
          {/* Embedding */}
          <div className="text-xs font-medium text-gray-400 mt-3 mb-2">{t.configPage.subEmbedding}</div>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Toggle label={t.configPage.embedding} value={local.embedding} onChange={v => set('embedding', v)} title={t.configPage.embeddingTip} />
            <Select label={t.configPage.pooling} value={local.pooling} onChange={v => set('pooling', v)} options={['', 'none', 'mean', 'cls', 'last', 'rank']} title={t.configPage.poolingTip} defaultLabel={t.common.default} />
            <Num label={t.configPage.embdNormalize} value={local.embd_normalize} onChange={v => set('embd_normalize', v)} min={0} max={2} title={t.configPage.embdNormalizeTip} />
            <Toggle label={t.configPage.reranking} value={local.reranking} onChange={v => set('reranking', v)} title={t.configPage.rerankingTip} />
          </div>
        </Section>

        {/* ════════════════════════ Section 6: Advanced ════════════════════════ */}
        <Section title={t.configPage.advanced}>
          {/* Memory & Loading */}
          <div className="text-xs font-medium text-gray-400 mb-2">{t.configPage.subMemLoad}</div>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Select label={t.configPage.flashAttn} value={local.flash_attn} onChange={v => set('flash_attn', v)} options={['auto', 'on', 'off']} title={t.configPage.flashAttnTip} defaultLabel={t.common.default} />
            <Num label={t.configPage.moeCpu} value={local.moe_cpu_layers} onChange={v => set('moe_cpu_layers', v)} min={0} max={99} title={t.configPage.moeCpuTip} disabled={isEmbedding} />
            <Toggle label={t.configPage.mlock} value={local.mlock} onChange={v => set('mlock', v)} title={t.configPage.mlockTip} />
            <Toggle label={t.configPage.noMmap} value={local.no_mmap} onChange={v => set('no_mmap', v)} title={t.configPage.noMmapTip} />
            <Toggle label={t.configPage.numa} value={local.numa} onChange={v => set('numa', v)} title={t.configPage.numaTip} />
            <Toggle label={t.configPage.checkTensors} value={local.check_tensors} onChange={v => set('check_tensors', v)} title={t.configPage.checkTensorsTip} />
            <Toggle label={t.configPage.fit} value={local.fit} onChange={v => set('fit', v)} title={t.configPage.fitTip} />
            <Toggle label={t.configPage.loraInitNoApply} value={local.lora_init_without_apply} onChange={v => set('lora_init_without_apply', v)} title={t.configPage.loraInitNoApplyTip} disabled={isEmbedding} />
          </div>
          {/* KV Cache */}
          <div className="text-xs font-medium text-gray-400 mt-3 mb-2">{t.configPage.subKvCache}</div>
          <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
            <Select label={t.configPage.cacheTypeK} value={local.cache_type_k} onChange={v => set('cache_type_k', v)} options={cacheTypes} title={t.configPage.cacheTypeKTip} defaultLabel={t.common.default} />
            <Select label={t.configPage.cacheTypeV} value={local.cache_type_v} onChange={v => set('cache_type_v', v)} options={cacheTypes} title={t.configPage.cacheTypeVTip} defaultLabel={t.common.default} />
            <Select label={t.configPage.cacheTypeDraftK} value={local.cache_type_draft_k} onChange={v => set('cache_type_draft_k', v)} options={cacheTypes} title={t.configPage.cacheTypeDraftKTip} defaultLabel={t.common.default} />
            <Select label={t.configPage.cacheTypeDraftV} value={local.cache_type_draft_v} onChange={v => set('cache_type_draft_v', v)} options={cacheTypes} title={t.configPage.cacheTypeDraftVTip} defaultLabel={t.common.default} />
            <Toggle label={t.configPage.kvUnified} value={local.kv_unified} onChange={v => set('kv_unified', v)} title={t.configPage.kvUnifiedTip} />
            <Toggle label={t.configPage.cacheIdleSlots} value={local.cache_idle_slots} onChange={v => set('cache_idle_slots', v)} title={t.configPage.cacheIdleSlotsTip} />
          </div>
          {/* Speculative Decoding (toggled) */}
          <div className="mt-3">
            <SubGroup label={t.configPage.subSpecDecoding} toggled={useSpecDecoding} onToggle={v => { setUseSpecDecoding(v); if (!v) { set('draft_model_path', ''); set('draft_gpu_layers', 99); set('draft_tokens', 16); set('spec_draft_n_min', 0); set('spec_type', ''); set('spec_draft_p_min', 0); set('spec_draft_p_split', 0.1); set('spec_draft_device', ''); set('lookup_cache_static', ''); set('lookup_cache_dynamic', '') } }}>
              <div className="grid grid-cols-2 md:grid-cols-3 gap-3" style={isEmbedding ? { opacity: 0.5, pointerEvents: 'none' } : {}}>
                <Input label={t.configPage.draftModel} value={local.draft_model_path} onChange={v => set('draft_model_path', v)} title={t.configPage.draftModelTip} disabled={isEmbedding} />
                <Num label={t.configPage.draftGpu} value={local.draft_gpu_layers} onChange={v => set('draft_gpu_layers', v)} min={0} max={99} title={t.configPage.draftGpuTip} disabled={isEmbedding} />
                <Num label={t.configPage.draftTokens} value={local.draft_tokens} onChange={v => set('draft_tokens', v)} min={0} title={t.configPage.draftTokensTip} disabled={isEmbedding} />
                <Num label={t.configPage.specDraftNMin} value={local.spec_draft_n_min} onChange={v => set('spec_draft_n_min', v)} min={0} title={t.configPage.specDraftNMinTip} disabled={isEmbedding} />
                <Select label={t.configPage.specType} value={local.spec_type} onChange={v => set('spec_type', v)} options={specTypes} title={t.configPage.specTypeTip} disabled={isEmbedding} defaultLabel={t.common.default} />
                <Num label={t.configPage.specDraftPMin} value={local.spec_draft_p_min} onChange={v => set('spec_draft_p_min', v)} min={0} max={1} step={0.05} title={t.configPage.specDraftPMinTip} disabled={isEmbedding} />
                <Num label={t.configPage.specDraftPSplit} value={local.spec_draft_p_split} onChange={v => set('spec_draft_p_split', v)} min={0} max={1} step={0.05} title={t.configPage.specDraftPSplitTip} disabled={isEmbedding} />
                <Input label={t.configPage.specDraftDevice} value={local.spec_draft_device} onChange={v => set('spec_draft_device', v)} title={t.configPage.specDraftDeviceTip} disabled={isEmbedding} />
                <Input label={t.configPage.lookupCacheStatic} value={local.lookup_cache_static} onChange={v => set('lookup_cache_static', v)} title={t.configPage.lookupCacheStaticTip} disabled={isEmbedding} />
                <Input label={t.configPage.lookupCacheDynamic} value={local.lookup_cache_dynamic} onChange={v => set('lookup_cache_dynamic', v)} title={t.configPage.lookupCacheDynamicTip} disabled={isEmbedding} />
              </div>
            </SubGroup>
          </div>
          {/* Expert Settings (toggled) */}
          <div className="mt-3">
            <SubGroup label={t.configPage.subExpert} toggled={useExpertSettings} onToggle={v => { setUseExpertSettings(v); if (!v) { set('device', ''); set('split_mode', ''); set('tensor_split', ''); set('main_gpu', 0); set('override_kv', ''); set('models_dir', ''); set('models_preset', ''); set('models_max', 4); set('models_autoload', false); set('mmproj_url', ''); set('mmproj_auto', false); set('image_min_tokens', 0); set('image_max_tokens', 0); set('media_path', ''); set('tags', ''); set('tools', '') } }}>
              {/* GPU / Device */}
              <div className="text-xs font-medium text-gray-400 mb-2">{t.configPage.subGpuDevice}</div>
              <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
                <Input label={t.configPage.device} value={local.device} onChange={v => set('device', v)} title={t.configPage.deviceTip} />
                <Select label={t.configPage.splitMode} value={local.split_mode} onChange={v => set('split_mode', v)} options={['', 'none', 'layer', 'row']} title={t.configPage.splitModeTip} defaultLabel={t.common.default} />
                <Input label={t.configPage.tensorSplit} value={local.tensor_split} onChange={v => set('tensor_split', v)} title={t.configPage.tensorSplitTip} />
                <Num label={t.configPage.mainGpu} value={local.main_gpu} onChange={v => set('main_gpu', v)} min={0} max={9} title={t.configPage.mainGpuTip} />
                <Input label={t.configPage.overrideKv} value={local.override_kv} onChange={v => set('override_kv', v)} title={t.configPage.overrideKvTip} />
              </div>
              {/* Multi-Model */}
              <div className="text-xs font-medium text-gray-400 mt-3 mb-2">{t.configPage.subMultiModel}</div>
              <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
                <Input label={t.configPage.modelsDir} value={local.models_dir} onChange={v => set('models_dir', v)} title={t.configPage.modelsDirTip} />
                <Input label={t.configPage.modelsPreset} value={local.models_preset} onChange={v => set('models_preset', v)} title={t.configPage.modelsPresetTip} />
                <Num label={t.configPage.modelsMax} value={local.models_max} onChange={v => set('models_max', v)} min={1} max={99} title={t.configPage.modelsMaxTip} />
                <Toggle label={t.configPage.modelsAutoload} value={local.models_autoload} onChange={v => set('models_autoload', v)} title={t.configPage.modelsAutoloadTip} />
              </div>
              {/* Multimodal / Media */}
              <div className="text-xs font-medium text-gray-400 mt-3 mb-2">{t.configPage.subMedia}</div>
              <div className="grid grid-cols-2 md:grid-cols-3 gap-3">
                <Input label={t.configPage.mmprojUrl} value={local.mmproj_url} onChange={v => set('mmproj_url', v)} title={t.configPage.mmprojUrlTip} disabled={isEmbedding} />
                <Toggle label={t.configPage.mmprojAuto} value={local.mmproj_auto} onChange={v => set('mmproj_auto', v)} title={t.configPage.mmprojAutoTip} disabled={isEmbedding} />
                <Num label={t.configPage.imageMinTokens} value={local.image_min_tokens} onChange={v => set('image_min_tokens', v)} min={0} title={t.configPage.imageMinTokensTip} disabled={isEmbedding} />
                <Num label={t.configPage.imageMaxTokens} value={local.image_max_tokens} onChange={v => set('image_max_tokens', v)} min={0} title={t.configPage.imageMaxTokensTip} disabled={isEmbedding} />
                <Input label={t.configPage.mediaPath} value={local.media_path} onChange={v => set('media_path', v)} title={t.configPage.mediaPathTip} />
                <Input label={t.configPage.tags} value={local.tags} onChange={v => set('tags', v)} title={t.configPage.tagsTip} />
                <Input label={t.configPage.tools} value={local.tools} onChange={v => set('tools', v)} title={t.configPage.toolsTip} />
              </div>
            </SubGroup>
          </div>
        </Section>
      </div>

      {showPicker && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-lg w-full max-w-2xl max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between px-6 py-4 border-b dark:border-gray-700">
              <h3 className="text-lg font-semibold">{t.modelRepo.selectFromRepo}</h3>
              <button onClick={() => setShowPicker(false)} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded"><X className="w-5 h-5" /></button>
            </div>
            <div className="flex-1 overflow-y-auto p-4">
              {(function TreeRenderer() {
                interface TNode { name: string; path: string; isDir: boolean; children?: Map<string, TNode>; model?: typeof models[0] }
                function buildTree(rootDir: string): TNode {
                  const root: TNode = { name: rootDir, path: rootDir, isDir: true, children: new Map() }
                  const normRoot = rootDir.replace(/\\/g, '\\').toLowerCase()
                  for (const m of models) {
                    const normPath = m.path.replace(/\\/g, '\\').toLowerCase()
                    if (!normPath.startsWith(normRoot)) continue
                    const rel = m.path.substring(rootDir.length).replace(/^[\\/]+/, '')
                    if (!rel) continue
                    const parts = rel.split(/[\\/]/)
                    let cur = root
                    for (let i = 0; i < parts.length; i++) {
                      if (i === parts.length - 1) { cur.children!.set(parts[i], { name: parts[i], path: m.path, isDir: false, model: m }) }
                      else {
                        if (!cur.children!.has(parts[i])) { cur.children!.set(parts[i], { name: parts[i], path: cur.path + (cur.path.endsWith('\\') ? '' : '\\') + parts[i], isDir: true, children: new Map() }) }
                        cur = cur.children!.get(parts[i])!
                      }
                    }
                  }
                  return root
                }
                const toggleP = (k: string) => { const n = new Set(pickerCollapsed); if (n.has(k)) n.delete(k); else n.add(k); setPickerCollapsed(n) }
                function renderNode(node: TNode, depth: number): any {
                  if (node.isDir) {
                    const c = pickerCollapsed.has(node.path)
                    return (<div key={node.path}>
                      <button onClick={() => toggleP(node.path)} style={{ paddingLeft: `${depth * 12 + 4}px` }} className="w-full flex items-center gap-1.5 py-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded text-left text-xs">
                        {c ? <ChevronRight className="w-3 h-3 text-gray-400 shrink-0" /> : <ChevronDown className="w-3 h-3 text-gray-400 shrink-0" />}
                        {depth === 0 ? <FolderOpen className="w-3 h-3 text-yellow-500 shrink-0" /> : <span className="text-xs shrink-0">{'\uD83D\uDCC1'}</span>}
                        <span className="truncate font-medium text-xs">{node.name}</span>
                      </button>
                      {!c && node.children && [...node.children.values()].sort((a, b) => { if (a.isDir !== b.isDir) return a.isDir ? -1 : 1; return a.name.localeCompare(b.name) }).map(ch => renderNode(ch, depth + 1))}
                    </div>)
                  }
                  const m = node.model!
                  if (m.file_type === 'mmproj') return (<div key={node.path} style={{ paddingLeft: `${depth * 12 + 20}px` }} className="flex items-center gap-2 py-1 pr-2 text-xs text-gray-500"><Image className="w-3 h-3 text-purple-500 shrink-0" /><span className="truncate flex-1">{m.name}</span><span className="text-purple-400 shrink-0 text-xs">{t.modelRepo.typeMmprojShort}</span></div>)
                  return (<button key={node.path} onClick={() => pickModel(m.path)} style={{ paddingLeft: `${depth * 12 + 20}px` }} className="w-full flex items-center gap-2 py-1 pr-2 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded text-left text-xs">
                    <File className="w-3 h-3 text-blue-500 shrink-0" /><span className="truncate flex-1">{m.name}</span><span className="text-gray-400 shrink-0">{m.quant_type || ''}</span>
                    <span className="text-gray-400 shrink-0">{m.size > 1024 * 1024 * 1024 ? (m.size / 1024 / 1024 / 1024).toFixed(1) + ' GB' : m.size > 1024 * 1024 ? (m.size / 1024 / 1024).toFixed(1) + ' MB' : m.size > 1024 ? (m.size / 1024).toFixed(1) + ' KB' : m.size + ' B'}</span>
                  </button>)
                }
                return modelDirs.map(d => buildTree(d)).map(t => renderNode(t, 0))
              })()}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

export default ConfigPage
