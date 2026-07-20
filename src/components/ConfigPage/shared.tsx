import { useState, useRef, useEffect, createContext, useContext, useId } from 'react'
import { createPortal } from 'react-dom'
import { ChevronDown, ChevronRight, CircleHelp, RotateCcw } from 'lucide-react'
import type { EngineCapabilities, InstanceConfig } from '../../store'
import {
  SYSTEM_MANAGED_PARAMETER_KEYS,
  parameterDependencyActive,
  parameterEngineDefault,
  parameterFlags,
  type ParameterSource,
} from '../../parameterCatalog'
import { reviewFieldKeys } from './configWorkspace'
import { SelectInput, TextInput, surfaceClassName } from '../ui'

export const cacheTypes = ['', 'f32', 'f16', 'bf16', 'q8_0', 'q4_0', 'q4_1', 'iq4_nl', 'q5_0', 'q5_1']
export const specTypes = ['', 'none', 'draft-mtp', 'draft-simple', 'draft-eagle3', 'draft-dflash', 'ngram-cache', 'ngram-simple', 'ngram-map-k', 'ngram-map-k4v', 'ngram-mod']
export const chatTemplates = ['', 'bailing', 'bailing-think', 'bailing2', 'chatglm3', 'chatglm4', 'chatml', 'command-r', 'deepseek', 'deepseek-ocr', 'deepseek2', 'deepseek3', 'exaone-moe', 'exaone3', 'exaone4', 'falcon3', 'gemma', 'gigachat', 'glmedge', 'gpt-oss', 'granite', 'granite-4.0', 'granite-4.1', 'grok-2', 'hunyuan-dense', 'hunyuan-moe', 'hunyuan-vl', 'kimi-k2', 'llama2', 'llama2-sys', 'llama2-sys-bos', 'llama2-sys-strip', 'llama3', 'llama4', 'megrez', 'minicpm', 'mistral-v1', 'mistral-v3', 'mistral-v3-tekken', 'mistral-v7', 'mistral-v7-tekken', 'monarch', 'openchat', 'orion', 'pangu-embedded', 'phi3', 'phi4', 'rwkv-world', 'seed_oss', 'smolvlm', 'solar-open', 'vicuna', 'vicuna-orca', 'yandex', 'zephyr']

// Search and draft-change metadata are shared by every field without prop drilling.
type FieldKey = keyof InstanceConfig
export type FieldRuntimeLabels = {
  inherited: string
  explicit: string
  managed: string
  inactive: string
  unsupported: string
  unsaved: string
  currentState: string
  engineDefault: string
  commandFlags: string
  restoreInheritance: string
  help: string
}

type FieldRuntimeContextValue = {
  config?: InstanceConfig
  isEmbedding: boolean
  explicitKeys: ReadonlySet<FieldKey>
  emittedKeys: ReadonlySet<FieldKey>
  unsupportedFlags: ReadonlySet<string>
  capabilities?: EngineCapabilities
  engineVersion?: string
  lang: string
  labels?: FieldRuntimeLabels
  onInherit?: (keys: FieldKey[]) => void
}

const FieldRuntimeCtx = createContext<FieldRuntimeContextValue>({
  isEmbedding: false,
  explicitKeys: new Set(),
  emittedKeys: new Set(),
  unsupportedFlags: new Set(),
  lang: 'en-US',
})

export const FieldRuntimeProvider = ({
  config,
  isEmbedding,
  explicitKeys,
  emittedKeys,
  unsupportedFlags,
  capabilities,
  engineVersion,
  lang,
  labels,
  onInherit,
  children,
}: FieldRuntimeContextValue & { children: React.ReactNode }) => (
  <FieldRuntimeCtx.Provider value={{
    config,
    isEmbedding,
    explicitKeys,
    emittedKeys,
    unsupportedFlags,
    capabilities,
    engineVersion,
    lang,
    labels,
    onInherit,
  }}>
    {children}
  </FieldRuntimeCtx.Provider>
)

type SearchContextValue = {
  query: string
  changedKeys: ReadonlySet<FieldKey>
  emittedKeys: ReadonlySet<FieldKey>
  changedLabel: string
  emittedLabel: string
}
const EMPTY_FIELD_KEYS = new Set<FieldKey>()
const SearchCtx = createContext<SearchContextValue>({
  query: '',
  changedKeys: EMPTY_FIELD_KEYS,
  emittedKeys: EMPTY_FIELD_KEYS,
  changedLabel: '',
  emittedLabel: '',
})
const useSearchQuery = () => useContext(SearchCtx).query
const useFieldState = (label: string, fieldKey?: FieldKey) => {
  const context = useContext(SearchCtx)
  return {
    match: !!(context.query && label.toLowerCase().includes(context.query.toLowerCase())),
    changed: !!(fieldKey && context.changedKeys.has(fieldKey)),
    emitted: !!(fieldKey && context.emittedKeys.has(fieldKey)),
    changedLabel: context.changedLabel,
    emittedLabel: context.emittedLabel,
  }
}

const sourceTone: Record<ParameterSource, string> = {
  inherited: 'border-slate-300 bg-slate-100 text-slate-600 dark:border-slate-700 dark:bg-slate-800 dark:text-slate-300',
  explicit: 'border-blue-300/60 bg-blue-50 text-blue-700 dark:border-blue-500/30 dark:bg-blue-500/10 dark:text-blue-300',
  managed: 'border-violet-300/60 bg-violet-50 text-violet-700 dark:border-violet-500/30 dark:bg-violet-500/10 dark:text-violet-300',
  inactive: 'border-amber-300/60 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300',
  unsupported: 'border-red-300/60 bg-red-50 text-red-700 dark:border-red-500/30 dark:bg-red-500/10 dark:text-red-300',
}

const sourceText = (source: ParameterSource, labels?: FieldRuntimeLabels) => labels?.[source] ?? source

const FieldStatusMarker = ({ source, changed, labels }: { source: ParameterSource; changed: boolean; labels?: FieldRuntimeLabels }) => (
  <span className="inline-flex shrink-0 items-center gap-1.5">
    {changed && (
      <span
        data-config-status="changed"
        className="h-1.5 w-1.5 rounded-full bg-slate-400"
        title={labels?.unsaved}
        aria-label={labels?.unsaved}
      />
    )}
    <span data-config-status="source" data-config-source={source} className={`rounded-md border px-1.5 py-0.5 text-[10px] font-medium ${sourceTone[source]}`}>
      {sourceText(source, labels)}
    </span>
  </span>
)

const ParameterHelp = ({
  label,
  title,
  fieldKey,
  source,
  engineDefault,
  flags,
  labels,
  onInherit,
  canRestore,
}: {
  label: string
  title: string
  fieldKey?: FieldKey
  source: ParameterSource
  engineDefault?: string
  flags: string[]
  labels?: FieldRuntimeLabels
  onInherit?: (keys: FieldKey[]) => void
  canRestore: boolean
}) => {
  const id = useId()
  const buttonRef = useRef<HTMLButtonElement>(null)
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [open, setOpen] = useState(false)
  const [position, setPosition] = useState({ top: 0, left: 0 })

  const cancelClose = () => {
    if (closeTimer.current !== null) clearTimeout(closeTimer.current)
    closeTimer.current = null
  }
  const show = () => {
    cancelClose()
    const rect = buttonRef.current?.getBoundingClientRect()
    if (rect) {
      const width = Math.min(360, Math.max(280, window.innerWidth - 24))
      setPosition({
        top: Math.max(12, Math.min(rect.bottom + 8, window.innerHeight - 240)),
        left: Math.max(12, Math.min(rect.left, window.innerWidth - width - 12)),
      })
    }
    setOpen(true)
  }
  const scheduleClose = () => {
    cancelClose()
    closeTimer.current = setTimeout(() => setOpen(false), 140)
  }

  useEffect(() => () => {
    if (closeTimer.current !== null) clearTimeout(closeTimer.current)
  }, [])

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        aria-label={`${labels?.help ?? 'Help'}: ${label}`}
        aria-describedby={open ? id : undefined}
        onMouseEnter={show}
        onMouseLeave={scheduleClose}
        onFocus={show}
        onBlur={scheduleClose}
        onClick={show}
        onKeyDown={event => {
          if (event.key === 'Escape') {
            cancelClose()
            setOpen(false)
            buttonRef.current?.focus()
          }
        }}
        className="mt-0.5 shrink-0 rounded text-slate-500 transition hover:text-blue-400 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400"
      >
        <CircleHelp className="h-3.5 w-3.5" />
      </button>
      {open && createPortal(
        <div
          id={id}
          role="tooltip"
          onMouseEnter={cancelClose}
          onMouseLeave={scheduleClose}
          className="fixed z-[100] max-h-[calc(100vh-24px)] w-[min(360px,calc(100vw-24px))] overflow-y-auto rounded-xl border border-slate-200 bg-white p-3 text-left shadow-2xl dark:border-slate-700 dark:bg-slate-900"
          style={position}
        >
          <p className="text-xs font-semibold text-slate-900 dark:text-slate-100">{label}</p>
          <p className="mt-2 whitespace-pre-line text-xs leading-5 text-slate-600 dark:text-slate-300">{title}</p>
          <dl className="mt-3 space-y-1.5 border-t border-slate-200 pt-2 text-[11px] dark:border-slate-700">
            <div className="flex gap-2">
              <dt className="shrink-0 text-slate-500">{labels?.currentState}</dt>
              <dd className="text-slate-800 dark:text-slate-200">{sourceText(source, labels)}</dd>
            </div>
            {engineDefault && (
              <div className="flex gap-2">
                <dt className="shrink-0 text-slate-500">{labels?.engineDefault}</dt>
                <dd className="text-slate-800 dark:text-slate-200">{engineDefault}</dd>
              </div>
            )}
            {flags.length > 0 && (
              <div className="flex gap-2">
                <dt className="shrink-0 text-slate-500">{labels?.commandFlags}</dt>
                <dd className="break-all font-mono text-blue-700 dark:text-blue-300">{flags.join(' / ')}</dd>
              </div>
            )}
          </dl>
          {fieldKey && canRestore && onInherit && (
            <button
              type="button"
              onClick={() => { onInherit(reviewFieldKeys(fieldKey)); setOpen(false) }}
              className="mt-3 inline-flex items-center gap-1 rounded-md border border-slate-300 px-2 py-1 text-[11px] text-slate-600 transition hover:border-blue-400 hover:text-blue-600 dark:border-slate-700 dark:text-slate-300"
            >
              <RotateCcw className="h-3 w-3" />
              {labels?.restoreInheritance}
            </button>
          )}
        </div>,
        document.body,
      )}
    </>
  )
}

const FieldFrame = ({
  label,
  fieldKey,
  title,
  disabled,
  className = '',
  showLabel = true,
  children,
}: {
  label: string
  fieldKey?: FieldKey
  title?: string
  disabled?: boolean
  className?: string
  showLabel?: boolean
  children: React.ReactNode
}) => {
  const { match, changed, emitted } = useFieldState(label, fieldKey)
  const runtime = useContext(FieldRuntimeCtx)
  const explicit = !!(fieldKey && runtime.explicitKeys.has(fieldKey))
  const managed = !!(fieldKey && SYSTEM_MANAGED_PARAMETER_KEYS.has(fieldKey))
  const dependencyActive = !!(!fieldKey || !runtime.config || parameterDependencyActive(fieldKey, runtime.config, runtime.isEmbedding))
  const catalogFlags = fieldKey ? parameterFlags(fieldKey) : []
  const flags = catalogFlags.length > 0
    ? catalogFlags
    : Array.from(new Set(label.match(/-{1,2}[a-z][a-z0-9-]*/gi) ?? []))
  const unsupported = flags.some(flag => runtime.unsupportedFlags.has(flag))
  const source: ParameterSource = unsupported
    ? 'unsupported'
    : managed
      ? 'managed'
      : explicit && (!dependencyActive || !emitted)
        ? 'inactive'
        : explicit
          ? 'explicit'
          : 'inherited'
  const engineDefault = fieldKey
    ? parameterEngineDefault(fieldKey, runtime.capabilities, runtime.engineVersion, runtime.lang, flags)
    : undefined
  return (
    <div
      className={`flex h-full min-w-0 flex-col justify-between ${disabled ? 'opacity-50' : ''} ${match ? 'flash-match rounded-lg ring-2 ring-amber-400/70' : ''} ${className}`}
      data-config-field={fieldKey}
      data-config-search-match={match ? 'true' : undefined}
      data-config-emitted={emitted ? 'true' : undefined}
      data-config-source={source}
    >
      {showLabel && (
        <div className="mb-1 flex min-h-8 min-w-0 items-start justify-between gap-2">
          <span className="flex min-w-0 items-start gap-1">
            <label className={`min-w-0 break-words text-xs font-medium leading-4 ${match ? 'text-amber-300' : 'text-slate-500 dark:text-slate-400'}`}>
              {label}
            </label>
            {title && <ParameterHelp label={label} title={title} fieldKey={fieldKey} source={source} engineDefault={engineDefault} flags={flags} labels={runtime.labels} onInherit={runtime.onInherit} canRestore={explicit} />}
          </span>
          <FieldStatusMarker source={source} changed={changed} labels={runtime.labels} />
        </div>
      )}
      <div className="min-w-0">{children}</div>
    </div>
  )
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
  changedParams,
  emittedParams,
  changedLabel,
  emittedLabel,
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
  changedParams?: ReadonlySet<FieldKey>
  emittedParams?: ReadonlySet<FieldKey>
  changedLabel?: string
  emittedLabel?: string
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
        <SearchCtx.Provider value={{
          query: searchQuery || '',
          changedKeys: changedParams || EMPTY_FIELD_KEYS,
          emittedKeys: emittedParams || EMPTY_FIELD_KEYS,
          changedLabel: changedLabel || '',
          emittedLabel: emittedLabel || '',
        }}>
          <div className={`config-layout-container space-y-4 px-4 py-4 ${disabled ? 'pointer-events-none opacity-50' : ''}`}>{children}</div>
        </SearchCtx.Provider>
      )}
    </section>
  )
}

export const Input = ({ label, value, onChange, placeholder, type, title, disabled, fieldKey }: { label: string; value: string; onChange: (v: string) => void; placeholder?: string; type?: string; title?: string; disabled?: boolean; fieldKey?: FieldKey }) => {
  return (
    <FieldFrame label={label} fieldKey={fieldKey} title={title} disabled={disabled}>
      <TextInput type={type || 'text'} value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder} disabled={disabled} className="h-10 w-full" />
    </FieldFrame>
)}

export const Num = ({ label, value, onChange, min, max, step, title, disabled, fieldKey }: { label: string; value: number; onChange: (v: number) => void; min?: number; max?: number; step?: number; title?: string; disabled?: boolean; fieldKey?: FieldKey }) => {
  return (
    <FieldFrame label={label} fieldKey={fieldKey} title={title} disabled={disabled}>
      <TextInput type="number" value={value} min={min} max={max} step={step || 1} onChange={e => onChange(parseFloat(e.target.value) || 0)} disabled={disabled} className="h-10 w-full" />
    </FieldFrame>
)}

export const IntentNum = ({
  label,
  value,
  onChange,
  inherited,
  onInherit,
  onManual,
  inheritedLabel,
  manualLabel,
  min,
  max,
  step,
  title,
  disabled,
  fieldKey,
}: {
  label: string
  value: number
  onChange: (v: number) => void
  inherited: boolean
  onInherit: () => void
  onManual: () => void
  inheritedLabel: string
  manualLabel: string
  min?: number
  max?: number
  step?: number
  title?: string
  disabled?: boolean
  fieldKey: FieldKey
}) => {
  const runtime = useContext(FieldRuntimeCtx)
  const catalogFlags = parameterFlags(fieldKey)
  const fallbackFlags = label.match(/-{1,2}[a-z][a-z0-9-]*/gi) ?? []
  const engineDefault = parameterEngineDefault(
    fieldKey,
    runtime.capabilities,
    runtime.engineVersion,
    runtime.lang,
    catalogFlags.length > 0 ? catalogFlags : fallbackFlags,
  )
  return (
    <FieldFrame label={label} fieldKey={fieldKey} title={title} disabled={disabled}>
      <div className="grid grid-cols-[minmax(88px,0.9fr)_minmax(0,1.25fr)] gap-2">
        <SelectInput
          aria-label={`${label}: ${inherited ? inheritedLabel : manualLabel}`}
          value={inherited ? 'inherit' : 'manual'}
          onChange={event => event.target.value === 'inherit' ? onInherit() : onManual()}
          disabled={disabled}
          className="h-10 min-w-0 px-2 text-xs"
        >
          <option value="inherit">{inheritedLabel}</option>
          <option value="manual">{manualLabel}</option>
        </SelectInput>
        {inherited ? (
          <div
            data-config-inherited-value="true"
            className="flex h-10 min-w-0 items-center truncate rounded-lg border border-dashed border-slate-300 bg-slate-50 px-3 text-xs text-slate-500 dark:border-slate-700 dark:bg-slate-950/40 dark:text-slate-400"
            title={engineDefault ?? inheritedLabel}
          >
            {engineDefault ?? inheritedLabel}
          </div>
        ) : (
          <TextInput
            type="number"
            value={value}
            min={min}
            max={max}
            step={step || 1}
            onChange={event => onChange(parseFloat(event.target.value) || 0)}
            disabled={disabled}
            className="h-10 min-w-0 w-full"
          />
        )}
      </div>
    </FieldFrame>
  )
}

export const Toggle = ({ label, value, onChange, title, disabled }: { label: string; value: boolean; onChange: (v: boolean) => void; title?: string; disabled?: boolean }) => (
  <label className={`flex items-center gap-2 ${disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}`} title={title}>
    <input type="checkbox" checked={value} onChange={e => onChange(e.target.checked)} disabled={disabled} className="w-4 h-4 rounded" />
    <span className="text-sm">{label}</span>
  </label>
)

export const Switch = ({ label, value, onChange, title, disabled, fieldKey }: { label: string; value: boolean; onChange: (v: boolean) => void; title?: string; disabled?: boolean; fieldKey?: FieldKey }) => {
  return (
    <FieldFrame label={label} fieldKey={fieldKey} title={title} disabled={disabled}>
      <button
        type="button"
        role="switch"
        aria-label={label}
        aria-checked={value}
        onClick={() => onChange(!value)}
        disabled={disabled}
        className="flex h-11 w-full items-center rounded-lg border border-slate-300 bg-white px-3 transition hover:border-slate-400 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400 disabled:cursor-not-allowed dark:border-slate-800 dark:bg-slate-950/60 dark:hover:border-slate-700"
      >
        <span className={`relative inline-flex h-5 w-9 shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out ${value ? 'bg-blue-600' : 'bg-slate-400 dark:bg-slate-700'}`}>
          <span className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200 ease-in-out ${value ? 'translate-x-4' : 'translate-x-0'}`} />
        </span>
      </button>
    </FieldFrame>
)}

export const Select = ({ label, value, onChange, options, title, disabled, defaultLabel, fieldKey }: { label: string; value: string; onChange: (v: string) => void; options: string[]; title?: string; disabled?: boolean; defaultLabel?: string; fieldKey?: FieldKey }) => {
  return (
    <FieldFrame label={label} fieldKey={fieldKey} title={title} disabled={disabled}>
      <SelectInput value={value} onChange={e => onChange(e.target.value)} disabled={disabled} className="h-10 w-full">
      {options.map(o => <option key={o} value={o}>{o || defaultLabel || '\u9ED8\u8BA4'}</option>)}
      </SelectInput>
    </FieldFrame>
)}

export const SearchTarget = ({ label, fieldKey, title, children, className = '', showLabel = true }: { label: string; fieldKey: FieldKey; title?: string; children: React.ReactNode; className?: string; showLabel?: boolean }) => {
  return (
    <FieldFrame label={label} fieldKey={fieldKey} title={title} className={className} showLabel={showLabel}>
      {children}
    </FieldFrame>
  )
}

// ━━━━━━━━━━━━━━━━━━━━━━ NEW COMPONENTS ━━━━━━━━━━━━━━━━━━━━━━

// Small reset button (↻ icon), used on sub-groups and container
export const ResetButton = ({ onClick, title }: { onClick: () => void; title?: string }) => (
  <button
    type="button"
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
  fieldKey,
}: {
  title: string
  defaultOpen?: boolean
  onReset?: () => void
  children: React.ReactNode
  disabled?: boolean
  id?: string
  summary?: React.ReactNode
  fieldKey?: FieldKey
}) => {
  const q = useSearchQuery()
  const fieldState = useFieldState(title, fieldKey)
  const [open, setOpen] = useState(defaultOpen || false)
  const hasSearch = !!q
  return (
    <div id={id} className="scroll-mt-6 overflow-hidden rounded-lg border border-slate-800 bg-slate-950/50" data-config-field={fieldKey} data-config-search-match={fieldState.match ? 'true' : undefined} data-config-emitted={fieldState.emitted ? 'true' : undefined}>
      <div className="flex items-center border-b border-slate-800 bg-slate-950/80 text-slate-200 transition hover:bg-slate-900">
        <button type="button" onClick={() => setOpen(!open)} className="flex min-w-0 flex-1 items-center gap-2 px-3 py-2.5 text-left">
          {(hasSearch || open) ? <ChevronDown className="h-3.5 w-3.5 shrink-0 text-slate-500" /> : <ChevronRight className="h-3.5 w-3.5 shrink-0 text-slate-500" />}
          <span className={`text-xs font-medium ${fieldState.match ? 'text-amber-300' : ''}`}>{title}</span>
          {summary && <span className="ml-auto text-xs text-slate-500">{summary}</span>}
        </button>
        {onReset && <ResetButton onClick={onReset} />}
      </div>
      {(hasSearch || open) && <div className={`config-layout-container px-3 py-3 ${disabled ? 'pointer-events-none opacity-50' : ''}`}>{children}</div>}
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
