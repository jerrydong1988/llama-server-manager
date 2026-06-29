import { useState, useRef, useEffect, createContext, useContext } from 'react'
import { ChevronDown, ChevronRight, RotateCcw } from 'lucide-react'
import type { InstanceConfig } from '../../store'

export const cacheTypes = ['', 'f32', 'f16', 'bf16', 'q8_0', 'q4_0', 'q4_1', 'iq4_nl', 'q5_0', 'q5_1']
export const specTypes = ['', 'none', 'draft-mtp', 'draft-simple', 'draft-eagle3', 'draft-dflash', 'ngram-cache', 'ngram-simple', 'ngram-map-k', 'ngram-map-k4v', 'ngram-mod']
export const chatTemplates = ['', 'bailing', 'bailing-think', 'bailing2', 'chatglm3', 'chatglm4', 'chatml', 'command-r', 'deepseek', 'deepseek-ocr', 'deepseek2', 'deepseek3', 'exaone-moe', 'exaone3', 'exaone4', 'falcon3', 'gemma', 'gigachat', 'glmedge', 'gpt-oss', 'granite', 'granite-4.0', 'granite-4.1', 'grok-2', 'hunyuan-dense', 'hunyuan-moe', 'hunyuan-vl', 'kimi-k2', 'llama2', 'llama2-sys', 'llama2-sys-bos', 'llama2-sys-strip', 'llama3', 'llama4', 'megrez', 'minicpm', 'mistral', 'mistral-v1', 'mistral-v3', 'mistral-v3-tekken', 'mistral-v7', 'mistral-v7-tekken', 'monarch', 'openchat', 'orion', 'pangu-embedded', 'phi3', 'phi4', 'rwkv-world', 'seed_oss', 'smolvlm', 'solar-open', 'vicuna', 'vicuna-orca', 'yandex', 'zephyr']

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
    if (match && q && ref.current) {
      _matchedElements.add(ref.current)
    }
    return () => {
      if (ref.current) _matchedElements.delete(ref.current)
    }
  }, [match, q])
  return ref
}

export const Section = ({ title, children, disabled, onToggle, toggled, defaultOpen, searchQuery }: { title: string; children: React.ReactNode; disabled?: boolean; onToggle?: (v: boolean) => void; toggled?: boolean; defaultOpen?: boolean; searchQuery?: string }) => {
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
    <div className={`border dark:border-gray-700 rounded-lg overflow-hidden`}>
      <button onClick={handleToggle} className="w-full flex items-center gap-2 px-4 py-2.5 bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors text-left dark:text-gray-200">
        {isOpen ? <ChevronDown className="w-4 h-4 text-gray-400 shrink-0" /> : <ChevronRight className="w-4 h-4 text-gray-400 shrink-0" />}
        <span className="text-sm font-medium">{title}</span>
        {disabled && <span className="text-xs text-gray-400 ml-1">{'\uD83D\uDED1'}</span>}
        {onToggle !== undefined && (
          <label className="ml-auto flex items-center gap-1 cursor-pointer shrink-0" onClick={e => e.stopPropagation()}>
            <span className="text-xs text-gray-400">{toggled ? 'On' : 'Off'}</span>
            <input type="checkbox" checked={toggled} onChange={e => { e.stopPropagation(); onToggle(e.target.checked) }} className="w-3.5 h-3.5 rounded" />
          </label>
        )}
      </button>
      {isOpen && (
        <SearchCtx.Provider value={searchQuery || ''}>
          <div className={`px-4 py-3 space-y-3 ${disabled ? 'opacity-50 pointer-events-none' : ''}`}>{children}</div>
        </SearchCtx.Provider>
      )}
    </div>
  )
}

export const Input = ({ label, value, onChange, placeholder, type, title, disabled, active }: { label: string; value: string; onChange: (v: string) => void; placeholder?: string; type?: string; title?: string; disabled?: boolean; active?: boolean }) => {
  const match = useLabelMatch(label)
  const ref = useSearchScroll(match)
  return (
  <div title={title} ref={ref}>
    <label className={`block text-xs font-medium mb-1 ${match ? 'text-amber-600 dark:text-amber-400' : active ? 'text-green-600 dark:text-green-400' : 'text-gray-500'}`}>{label}</label>
    <input type={type || 'text'} value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder} disabled={disabled} className={`w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 ${match ? 'ring-2 ring-amber-400 border-amber-400 flash-match' : ''} ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`} />
  </div>
)}

export const Num = ({ label, value, onChange, min, max, step, title, disabled, active }: { label: string; value: number; onChange: (v: number) => void; min?: number; max?: number; step?: number; title?: string; disabled?: boolean; active?: boolean }) => {
  const match = useLabelMatch(label)
  const ref = useSearchScroll(match)
  return (
  <div title={title} ref={ref}>
    <label className={`block text-xs font-medium mb-1 ${match ? 'text-amber-600 dark:text-amber-400' : active ? 'text-green-600 dark:text-green-400' : 'text-gray-500'}`}>{label}</label>
    <input type="number" value={value} min={min} max={max} step={step || 1} onChange={e => onChange(parseFloat(e.target.value) || 0)} disabled={disabled} className={`w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 ${match ? 'ring-2 ring-amber-400 border-amber-400 flash-match' : ''} ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`} />
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
      className={`relative inline-flex h-5 w-9 shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400 ${value ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'}`}
    >
      <span className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200 ease-in-out ${value ? 'translate-x-4' : 'translate-x-0'}`} />
    </button>
    <span className={`text-sm ${match ? 'text-amber-600 dark:text-amber-400' : active ? 'text-green-600 dark:text-green-400' : ''}`}>{label}</span>
  </label>
)}

export const Select = ({ label, value, onChange, options, title, disabled, defaultLabel, active }: { label: string; value: string; onChange: (v: string) => void; options: string[]; title?: string; disabled?: boolean; defaultLabel?: string; active?: boolean }) => {
  const match = useLabelMatch(label)
  const ref = useSearchScroll(match)
  return (
  <div title={title} ref={ref}>
    <label className={`block text-xs font-medium mb-1 ${match ? 'text-amber-600 dark:text-amber-400' : active ? 'text-green-600 dark:text-green-400' : 'text-gray-500'}`}>{label}</label>
    <select value={value} onChange={e => onChange(e.target.value)} disabled={disabled} className={`select-custom w-full pl-3 pr-8 py-1.5 text-sm text-gray-900 dark:text-gray-100 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 ${match ? 'ring-2 ring-amber-400 border-amber-400 flash-match' : ''} ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}>
      {options.map(o => <option key={o} value={o}>{o || defaultLabel || '\u9ED8\u8BA4'}</option>)}
    </select>
  </div>
)}

// ━━━━━━━━━━━━━━━━━━━━━━ NEW COMPONENTS ━━━━━━━━━━━━━━━━━━━━━━

// Small reset button (↻ icon), used on sub-groups and container
export const ResetButton = ({ onClick, title }: { onClick: () => void; title?: string }) => (
  <button
    onClick={e => { e.stopPropagation(); onClick() }}
    title={title || 'Reset to defaults'}
    className="ml-auto p-1 rounded hover:bg-red-100 dark:hover:bg-red-900/30 text-gray-400 hover:text-red-500 transition-colors shrink-0"
  >
    <RotateCcw className="w-3.5 h-3.5" />
  </button>
)

// Collapsible sub-group card (chevron toggle + optional reset button)
export const CollapsibleGroup = ({ title, defaultOpen, onReset, children, disabled }: { title: string; defaultOpen?: boolean; onReset?: () => void; children: React.ReactNode; disabled?: boolean }) => {
  const q = useSearchQuery()
  const [open, setOpen] = useState(defaultOpen || false)
  const hasSearch = !!q
  return (
    <div className="border dark:border-gray-600 rounded-lg overflow-hidden">
      <button onClick={() => setOpen(!open)} className="w-full flex items-center gap-2 px-3 py-2 bg-gray-50 dark:bg-gray-800/50 hover:bg-gray-100 dark:hover:bg-gray-700/50 transition-colors text-left dark:text-gray-200">
        {(hasSearch || open) ? <ChevronDown className="w-3.5 h-3.5 text-gray-400 shrink-0" /> : <ChevronRight className="w-3.5 h-3.5 text-gray-400 shrink-0" />}
        <span className="text-xs font-medium">{title}</span>
        {onReset && <ResetButton onClick={onReset} />}
      </button>
      {(hasSearch || open) && <div className={`px-3 py-3 ${disabled ? 'opacity-50 pointer-events-none' : ''}`}>{children}</div>}
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
    jinja: false, skip_chat_parsing: false,
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
    cache_idle_slots: true, kv_unified: false,
  },
  advancedContextMgmt: {
    ctx_checkpoints: 32, checkpoint_min_step: 0,
    context_shift: false, swa_full: false,
    keep: 0, override_kv: '',
  },
  advancedHardware: {
    moe_cpu_layers: 0, device: '', split_mode: '', tensor_split: '', main_gpu: 0,
    perf: false, check_tensors: false, fit: false, fit_target: '', fit_ctx: 4096,
    threads_http: -1,
  },
  advancedServerBasic: {
    api_key: '', api_key_file: '', no_ui: false,
    path_prefix: '', api_prefix: '', timeout: 600, sleep_idle: -1, verbose: false,
    ssl_key_file: '', ssl_cert_file: '',
  },
  advancedServerExt: {
    slots_enabled: true, metrics: false, props: false,
    slot_save_path: '', slot_prompt_similarity: 0.1, prefill_assistant: false,
    ui_config_file: '', ui_config: '', ui_mcp_proxy: false,
    rpc_servers: '', sse_ping_interval: 30, reuse_port: false,
  },
  advancedMulti: {
    models_dir: '', models_preset: '', models_max: 4, models_autoload: false,
    mmproj_url: '', mmproj_auto: false, image_min_tokens: 0, image_max_tokens: 0,
    media_path: '', tags: '', tools: '',
  },
}
