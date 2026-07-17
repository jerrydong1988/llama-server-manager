import { useState, useRef, useEffect, createContext, useContext } from 'react'
import { ChevronDown, ChevronRight, RotateCcw } from 'lucide-react'
import type { InstanceConfig } from '../../store'
import { SelectInput, TextInput, surfaceClassName } from '../ui'

export const cacheTypes = ['', 'f32', 'f16', 'bf16', 'q8_0', 'q4_0', 'q4_1', 'iq4_nl', 'q5_0', 'q5_1']
export const specTypes = ['', 'none', 'draft-mtp', 'draft-simple', 'draft-eagle3', 'draft-dflash', 'ngram-cache', 'ngram-simple', 'ngram-map-k', 'ngram-map-k4v', 'ngram-mod']
export const chatTemplates = ['', 'bailing', 'bailing-think', 'bailing2', 'chatglm3', 'chatglm4', 'chatml', 'command-r', 'deepseek', 'deepseek-ocr', 'deepseek2', 'deepseek3', 'exaone-moe', 'exaone3', 'exaone4', 'falcon3', 'gemma', 'gigachat', 'glmedge', 'gpt-oss', 'granite', 'granite-4.0', 'granite-4.1', 'grok-2', 'hunyuan-dense', 'hunyuan-moe', 'hunyuan-vl', 'kimi-k2', 'llama2', 'llama2-sys', 'llama2-sys-bos', 'llama2-sys-strip', 'llama3', 'llama4', 'megrez', 'minicpm', 'mistral-v1', 'mistral-v3', 'mistral-v3-tekken', 'mistral-v7', 'mistral-v7-tekken', 'monarch', 'openchat', 'orion', 'pangu-embedded', 'phi3', 'phi4', 'rwkv-world', 'seed_oss', 'smolvlm', 'solar-open', 'vicuna', 'vicuna-orca', 'yandex', 'zephyr']

// ── Search Context: injects searchQuery to all nested form fields without prop drilling ──
const SearchCtx = createContext<string>('')
const useSearchQuery = () => useContext(SearchCtx)
const useLabelMatch = (label: string) => {
  const q = useSearchQuery()
  return !!(q && label.toLowerCase().includes(q.toLowerCase()))
}

// Module-level Set of matched DOM elements — ConfigPage triggers scrollIntoView on first match
export const _matchedElements = new Set<HTMLElement>()

function useSearchScroll(match: boolean) {
  const ref = useRef<HTMLDivElement>(null)
  const q = useSearchQuery()
  useEffect(() => {
    const element = ref.current
    if (match && q && element) {
      _matchedElements.add(element)
    }
    return () => {
      if (element) _matchedElements.delete(element)
    }
  }, [match, q])
  return ref
}

export const Section = ({
  title,
  children,
  disabled,
  onToggle,
  toggled,
  defaultOpen,
  searchQuery,
  id,
  summary,
}: {
  title: string
  children: React.ReactNode
  disabled?: boolean
  onToggle?: (v: boolean) => void
  toggled?: boolean
  defaultOpen?: boolean
  searchQuery?: string
  id?: string
  summary?: React.ReactNode
}) => {
  const [open, setOpen] = useState(defaultOpen || false)
  const userToggled = useRef(false)
  const shouldOpen = !!(searchQuery && title.toLowerCase().includes(searchQuery.toLowerCase()))
  const hasSearch = !!searchQuery

  useEffect(() => { userToggled.current = false }, [searchQuery])

  const handleToggle = () => {
    userToggled.current = true
    setOpen(!open)
  }

  // Search active: always expand to reveal matching fields
  const isOpen = hasSearch || (!userToggled.current && shouldOpen) || open
  return (
    <section id={id} className={`${surfaceClassName} scroll-mt-6 overflow-hidden p-0`}>
      <button onClick={handleToggle} className="flex w-full items-center gap-2 border-b border-slate-800 bg-slate-950/80 px-4 py-3 text-left text-slate-100 transition hover:bg-slate-900">
        {isOpen ? <ChevronDown className="h-4 w-4 shrink-0 text-slate-500" /> : <ChevronRight className="h-4 w-4 shrink-0 text-slate-500" />}
        <span className="text-sm font-medium">{title}</span>
        {disabled && <span className="ml-1 text-xs text-slate-500">{'\uD83D\uDED1'}</span>}
        {summary && <span className="ml-auto text-xs text-slate-500">{summary}</span>}
        {onToggle !== undefined && (
          <label className={`${summary ? 'ml-2' : 'ml-auto'} flex items-center gap-1 cursor-pointer shrink-0`} onClick={e => e.stopPropagation()}>
            <span className="text-xs text-slate-500">{toggled ? 'On' : 'Off'}</span>
            <input type="checkbox" checked={toggled} onChange={e => { e.stopPropagation(); onToggle(e.target.checked) }} className="w-3.5 h-3.5 rounded" />
          </label>
        )}
      </button>
      {isOpen && (
        <SearchCtx.Provider value={searchQuery || ''}>
          <div className={`space-y-4 px-4 py-4 ${disabled ? 'pointer-events-none opacity-50' : ''}`}>{children}</div>
        </SearchCtx.Provider>
      )}
    </section>
  )
}

export const Input = ({ label, value, onChange, placeholder, type, title, disabled, active }: { label: string; value: string; onChange: (v: string) => void; placeholder?: string; type?: string; title?: string; disabled?: boolean; active?: boolean }) => {
  const match = useLabelMatch(label)
  const ref = useSearchScroll(match)
  return (
  <div title={title} ref={ref}>
    <label className={`mb-1 block text-xs font-medium ${match ? 'text-amber-300' : active ? 'text-emerald-300' : 'text-slate-400'}`}>{label}</label>
    <TextInput type={type || 'text'} value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder} disabled={disabled} className={`h-10 ${match ? 'flash-match border-amber-400 ring-2 ring-amber-400/70' : ''}`} />
  </div>
)}

export const Num = ({ label, value, onChange, min, max, step, title, disabled, active }: { label: string; value: number; onChange: (v: number) => void; min?: number; max?: number; step?: number; title?: string; disabled?: boolean; active?: boolean }) => {
  const match = useLabelMatch(label)
  const ref = useSearchScroll(match)
  return (
  <div title={title} ref={ref}>
    <label className={`mb-1 block text-xs font-medium ${match ? 'text-amber-300' : active ? 'text-emerald-300' : 'text-slate-400'}`}>{label}</label>
    <TextInput type="number" value={value} min={min} max={max} step={step || 1} onChange={e => onChange(parseFloat(e.target.value) || 0)} disabled={disabled} className={`h-10 ${match ? 'flash-match border-amber-400 ring-2 ring-amber-400/70' : ''}`} />
  </div>
)}

export const Toggle = ({ label, value, onChange, title, disabled }: { label: string; value: boolean; onChange: (v: boolean) => void; title?: string; disabled?: boolean }) => (
  <label className={`flex items-center gap-2 ${disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}`} title={title}>
    <input type="checkbox" checked={value} onChange={e => onChange(e.target.checked)} disabled={disabled} className="w-4 h-4 rounded" />
    <span className="text-sm">{label}</span>
  </label>
)

export const Switch = ({ label, value, onChange, title, disabled, active }: { label: string; value: boolean; onChange: (v: boolean) => void; title?: string; disabled?: boolean; active?: boolean }) => {
  const match = useLabelMatch(label)
  const ref = useSearchScroll(match)
  return (
  <label className={`flex items-center gap-2.5 ${disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'} ${match ? 'ring-2 ring-amber-400 rounded-lg p-1 -m-1 flash-match' : ''}`} title={title} ref={ref as any}>
    <button
      type="button"
      role="switch"
      aria-checked={value}
      onClick={() => onChange(!value)}
      disabled={disabled}
      className={`relative inline-flex h-5 w-9 shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400 ${value ? 'bg-blue-600' : 'bg-slate-700'}`}
    >
      <span className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200 ease-in-out ${value ? 'translate-x-4' : 'translate-x-0'}`} />
    </button>
    <span className={`text-sm ${match ? 'text-amber-300' : active ? 'text-emerald-300' : 'text-slate-200'}`}>{label}</span>
  </label>
)}

export const Select = ({ label, value, onChange, options, title, disabled, defaultLabel, active }: { label: string; value: string; onChange: (v: string) => void; options: string[]; title?: string; disabled?: boolean; defaultLabel?: string; active?: boolean }) => {
  const match = useLabelMatch(label)
  const ref = useSearchScroll(match)
  return (
  <div title={title} ref={ref}>
    <label className={`mb-1 block text-xs font-medium ${match ? 'text-amber-300' : active ? 'text-emerald-300' : 'text-slate-400'}`}>{label}</label>
    <SelectInput value={value} onChange={e => onChange(e.target.value)} disabled={disabled} className={`h-10 w-full ${match ? 'flash-match border-amber-400 ring-2 ring-amber-400/70' : ''}`}>
      {options.map(o => <option key={o} value={o}>{o || defaultLabel || '\u9ED8\u8BA4'}</option>)}
    </SelectInput>
  </div>
)}

// ━━━━━━━━━━━━━━━━━━━━━━ NEW COMPONENTS ━━━━━━━━━━━━━━━━━━━━━━

// Small reset button (↻ icon), used on sub-groups and container
export const ResetButton = ({ onClick, title }: { onClick: () => void; title?: string }) => (
  <button
    onClick={e => { e.stopPropagation(); onClick() }}
    title={title || 'Reset to defaults'}
    className="ml-auto shrink-0 rounded-lg p-1 text-slate-500 transition hover:bg-red-500/10 hover:text-red-300"
  >
    <RotateCcw className="w-3.5 h-3.5" />
  </button>
)

// Collapsible sub-group card (chevron toggle + optional reset button)
export const CollapsibleGroup = ({
  title,
  defaultOpen,
  onReset,
  children,
  disabled,
  id,
  summary,
}: {
  title: string
  defaultOpen?: boolean
  onReset?: () => void
  children: React.ReactNode
  disabled?: boolean
  id?: string
  summary?: React.ReactNode
}) => {
  const q = useSearchQuery()
  const [open, setOpen] = useState(defaultOpen || false)
  const hasSearch = !!q
  return (
    <div id={id} className="scroll-mt-6 overflow-hidden rounded-lg border border-slate-800 bg-slate-950/50">
      <button onClick={() => setOpen(!open)} className="flex w-full items-center gap-2 border-b border-slate-800 bg-slate-950/80 px-3 py-2.5 text-left text-slate-200 transition hover:bg-slate-900">
        {(hasSearch || open) ? <ChevronDown className="h-3.5 w-3.5 shrink-0 text-slate-500" /> : <ChevronRight className="h-3.5 w-3.5 shrink-0 text-slate-500" />}
        <span className="text-xs font-medium">{title}</span>
        {summary && <span className="ml-auto text-xs text-slate-500">{summary}</span>}
        {onReset && <ResetButton onClick={onReset} />}
      </button>
      {(hasSearch || open) && <div className={`px-3 py-3 ${disabled ? 'pointer-events-none opacity-50' : ''}`}>{children}</div>}
    </div>
  )
}

// ━━━━━━━━━━━━━━━━━━━━━━ RESET MAP ━━━━━━━━━━━━━━━━━━━━━━

// Field registry for advanced sub-groups. Values are IGNORED — actual defaults
// come from store/defaults.ts via defaultInstanceConfig(). Only field names matter.
// Add new fields here to make them resettable; values auto-sync with defaults.
export const RESET_MAP: Record<string, Partial<InstanceConfig>> = {
  advancedReasoningConfig: {
    reasoning_format: '', reasoning_effort: '', reasoning_budget: '', reasoning_budget_message: '',
    reasoning_preserve: '', jinja: false, skip_chat_parsing: false,
  },
  advancedModelAdapt: {
    chat_template_file: '', lora_path: '', lora_init_without_apply: false, mmproj_path: '',
    grammar: '', grammar_file: '', embd_normalize: 2, reranking: false,
  },
  advancedSampling: {
    mirostat: 0, mirostat_lr: 0, mirostat_ent: 0,
    xtc_probability: 0, xtc_threshold: 0,
    dynatemp_range: 0, dynatemp_exp: 0, typical_p: 1.0,
    dry_multiplier: 0, dry_base: 0, dry_allowed_length: 0, dry_penalty_last_n: 0,
    dry_sequence_breaker: '',
    adaptive_target: 0, adaptive_decay: 0, top_n_sigma: -1,
    logit_bias: '', samplers: '', sampler_seq: '',
  },
  advancedSamplingExt: {
    seed: -1, min_p: 0.05, presence_penalty: 0, frequency_penalty: 0,
    repeat_last_n: 64, special: false, spm_infill: false,
    backend_sampling: false, json_schema: '', json_schema_file: '',
  },
  advancedSpec: {
    draft_model_path: '', draft_gpu_layers: 99,
    spec_draft_p_min: 0, spec_draft_p_split: 0.1, spec_draft_device: '',
    lookup_cache_static: '', lookup_cache_dynamic: '',
    cache_type_draft_k: '', cache_type_draft_v: '',
    spec_default: false, spec_draft_backend_sampling: true,
    spec_draft_threads: 0, spec_draft_threads_batch: 0,
  },
  advancedRope: {
    rope_scaling: '', rope_scale: 0, rope_freq_base: 0, rope_freq_scale: 0,
    yarn_ext_factor: -1, yarn_attn_factor: -1, yarn_beta_slow: 0, yarn_beta_fast: -1,
  },
  advancedKvCache: {
    cache_type_k: '', cache_type_v: '',
    cache_prompt: true, cache_reuse: 0, cache_ram: 0, warmup: false,
    cache_idle_slots: true, kv_unified: false, kv_unified_mode: '',
  },
  advancedContextMgmt: {
    ctx_checkpoints: 32, checkpoint_min_step: 0,
    context_shift: false, swa_full: false,
    keep: 0, override_kv: '',
  },
  advancedHardware: {
    moe_cpu_layers: 0, cpu_moe: false, device: '', split_mode: '', tensor_split: '', main_gpu: 0,
    perf: false, check_tensors: false, fit: false, fit_mode: '', fit_target: '', fit_ctx: 4096,
    numa_mode: '',
    direct_io: false,
    threads_http: -1,
  },
  advancedServerBasic: {
    api_key: '', api_key_file: '', no_ui: false,
    path_prefix: '', api_prefix: '', timeout: 600, sleep_idle: -1, verbose: false,
    cors_origins: '', cors_methods: '', cors_headers: '', cors_credentials: '',
    ssl_key_file: '', ssl_cert_file: '',
  },
  advancedServerExt: {
    slots_enabled: true, metrics: false, props: false,
    slot_save_path: '', log_prompts_dir: '', slot_prompt_similarity: 0.1, prefill_assistant: false,
    ui_config_file: '', ui_config: '', ui_mcp_proxy: false, agent: false,
    rpc_servers: '', sse_ping_interval: 30, reuse_port: false,
  },
  advancedMulti: {
    models_dir: '', models_preset: '', models_max: 4, models_autoload: false,
    mmproj_url: '', mmproj_auto: false, mmproj_mode: '', image_min_tokens: 0, image_max_tokens: 0,
    media_path: '', tags: '', tools: '',
  },
}
