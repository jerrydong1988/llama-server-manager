import { useState, useRef, useEffect, useLayoutEffect, useCallback, createContext, useContext, useId } from 'react'
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
import { SelectInput, TextInput } from '../ui'
import {
  FALLBACK_SPECULATIVE_TYPES,
  normalizeSpeculativeTypes,
  orderSpeculativeTypeOptions,
  parseSpeculativeTypes,
} from '../../speculativeTypes'

export const cacheTypes = ['', 'f32', 'f16', 'bf16', 'f8_e4m3', 'q8_0', 'q4_0', 'q4_1', 'iq4_nl', 'q5_0', 'q5_1']
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

const sourceText = (source: ParameterSource, labels?: FieldRuntimeLabels) => labels?.[source] ?? source

const FieldStatusMarker = ({ source, changed, labels }: { source: ParameterSource; changed: boolean; labels?: FieldRuntimeLabels }) => (
  <span className="inline-flex shrink-0 items-center gap-1.5">
    {changed && (
      <span
        data-config-status="changed"
        className="h-1.5 w-1.5 rounded-full bg-ui-warn"
        title={labels?.unsaved}
        aria-label={labels?.unsaved}
      />
    )}
    <span data-config-status="source" data-config-source={source} className="config-source-label">
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
  const panelRef = useRef<HTMLDivElement>(null)
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [open, setOpen] = useState(false)
  useLayoutEffect(() => {
    const panel = panelRef.current
    const trigger = buttonRef.current
    if (!open || !panel || !trigger) return
    const place = () => {
      const anchor = trigger.getBoundingClientRect()
      const box = panel.getBoundingClientRect()
      const below = anchor.bottom + 8
      const top = below + box.height <= window.innerHeight - 12 ? below : anchor.top - box.height - 8
      panel.style.top = `${Math.max(12, Math.min(top, window.innerHeight - box.height - 12))}px`
      panel.style.left = `${Math.max(12, Math.min(anchor.left, window.innerWidth - box.width - 12))}px`
    }
    place()
    const observer = new ResizeObserver(place)
    observer.observe(panel)
    window.addEventListener('resize', place)
    window.addEventListener('scroll', place, true)
    return () => {
      observer.disconnect()
      window.removeEventListener('resize', place)
      window.removeEventListener('scroll', place, true)
    }
  }, [open])

  const cancelClose = () => {
    if (closeTimer.current !== null) clearTimeout(closeTimer.current)
    closeTimer.current = null
  }
  const show = () => {
    cancelClose()
    setOpen(true)
  }
  const scheduleClose = () => {
    cancelClose()
    closeTimer.current = setTimeout(() => {
      if (!panelRef.current?.contains(document.activeElement)) setOpen(false)
    }, 140)
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
          if (event.key === 'Tab' && !event.shiftKey && open && canRestore && panelRef.current?.querySelector('button')) {
            event.preventDefault()
            panelRef.current.querySelector('button')?.focus()
          }
          if (event.key === 'Escape') {
            cancelClose()
            setOpen(false)
            buttonRef.current?.focus()
          }
        }}
        className="mt-0.5 shrink-0 rounded text-ui-muted transition hover:text-ui-action-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-ui-focus"
      >
        <CircleHelp className="h-3.5 w-3.5" />
      </button>
      {open && createPortal(
        <div
          id={id}
          ref={panelRef}
          role="tooltip"
          onMouseEnter={cancelClose}
          onMouseLeave={scheduleClose}
          onFocus={cancelClose}
          onBlur={scheduleClose}
          onKeyDown={event => {
            if (event.key === 'Escape' || (event.key === 'Tab' && event.shiftKey)) {
              event.preventDefault()
              buttonRef.current?.focus()
              setOpen(false)
            }
          }}
          className="console-overlay fixed z-[100] max-h-[calc(100vh-24px)] w-[min(360px,calc(100vw-24px))] overflow-y-auto rounded-xl border border-ui-line bg-ui-surface p-3 text-left shadow-2xl"
        >
          <p className="text-xs font-semibold text-ui-ink">{label}</p>
          <p className="mt-2 whitespace-pre-line text-xs leading-5 text-ui-muted">{title}</p>
          <dl className="mt-3 space-y-1.5 border-t border-ui-line pt-2 text-[11px]">
            <div className="flex gap-2">
              <dt className="shrink-0 text-ui-muted">{labels?.currentState}</dt>
              <dd className="text-ui-ink">{sourceText(source, labels)}</dd>
            </div>
            {engineDefault && (
              <div className="flex gap-2">
                <dt className="shrink-0 text-ui-muted">{labels?.engineDefault}</dt>
                <dd className="text-ui-ink">{engineDefault}</dd>
              </div>
            )}
            {flags.length > 0 && (
              <div className="flex gap-2">
                <dt className="shrink-0 text-ui-muted">{labels?.commandFlags}</dt>
                <dd className="break-all font-mono text-ui-info">{flags.join(' / ')}</dd>
              </div>
            )}
          </dl>
          {fieldKey && canRestore && onInherit && (
            <button
              type="button"
              onClick={() => { onInherit(reviewFieldKeys(fieldKey)); setOpen(false) }}
              className="mt-3 inline-flex items-center gap-1 rounded-md border border-ui-line px-2 py-1 text-[11px] text-ui-muted transition hover:border-ui-control hover:text-ui-action-ink"
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
  const dependencyActive = !!(!fieldKey || !runtime.config || parameterDependencyActive(fieldKey, runtime.config, runtime.isEmbedding, {
    version: runtime.engineVersion,
    capabilities: runtime.capabilities,
  }))
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
      className={`flex h-full min-w-0 flex-col justify-between ${disabled ? 'opacity-50' : ''} ${match ? 'flash-match rounded-lg ring-2 ring-ui-warn' : ''} ${className}`}
      data-config-field={fieldKey}
      data-config-search-match={match ? 'true' : undefined}
      data-config-emitted={emitted ? 'true' : undefined}
      data-config-source={source}
    >
      {showLabel && (
        <div className="mb-1 flex min-h-8 min-w-0 items-start justify-between gap-2">
          <span className="flex min-w-0 items-start gap-1">
            <label className={`config-parameter-label min-w-0 break-words font-medium leading-5 ${match ? 'text-ui-warn' : 'text-ui-body'}`}>
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
    <section id={id} className="console-surface scroll-mt-4 overflow-hidden">
      <button type="button" aria-expanded={isOpen} onClick={handleToggle} className="flex w-full items-center gap-2 border-b border-ui-line bg-ui-inset px-4 py-3 text-left text-ui-ink transition hover:bg-ui-hover">
        {isOpen ? <ChevronDown className="h-4 w-4 shrink-0 text-ui-muted" /> : <ChevronRight className="h-4 w-4 shrink-0 text-ui-muted" />}
        <span className="text-sm font-medium">{title}</span>
        {disabled && <span className="ml-1 text-xs text-ui-muted">{'\uD83D\uDED1'}</span>}
        {summary && <span className="ml-auto text-xs text-ui-muted">{summary}</span>}
        {onToggle !== undefined && (
          <label className={`${summary ? 'ml-2' : 'ml-auto'} flex items-center gap-1 cursor-pointer shrink-0`} onClick={e => e.stopPropagation()}>
            <span className="text-xs text-ui-muted">{toggled ? 'On' : 'Off'}</span>
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
      <TextInput aria-label={label} type={type || 'text'} value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder} disabled={disabled} className="h-10 w-full" />
    </FieldFrame>
)}

type NumericInputProps = {
  label?: string
  value: number
  onChange: (value: number) => void
  min?: number
  max?: number
  step?: number | 'any'
  disabled?: boolean
  className?: string
}

const NumericInput = ({ label, value, onChange, min, max, step, disabled, className }: NumericInputProps) => {
  // Preserve intermediate edits (empty, minus sign, decimal point/exponent).
  // Only finite numbers of the field's type update configuration; blur restores
  // the last valid value. External changes such as inheritance/undo take precedence.
  const [edit, setEdit] = useState<{ value: number; text: string } | null>(null)
  const text = edit && Object.is(edit.value, value) ? edit.text : String(value)
  return (
    <TextInput
      aria-label={label} type="number" value={text} min={min} max={max} step={step ?? 1}
      onChange={event => {
        const next = event.target.valueAsNumber
        const complete = Number.isFinite(next) && (step === 'any' || Number.isInteger(next))
        setEdit({ value: complete ? next : value, text: event.target.value })
        if (complete) onChange(next)
      }}
      onBlur={() => setEdit(null)}
      disabled={disabled} className={className}
    />
  )
}

export const Num = ({ label, title, fieldKey, ...input }: NumericInputProps & { label: string; title?: string; fieldKey?: FieldKey }) => (
  <FieldFrame label={label} fieldKey={fieldKey} title={title} disabled={input.disabled}>
    <NumericInput {...input} label={label} className="h-10 w-full" />
  </FieldFrame>
)

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
            className="flex h-10 min-w-0 items-center truncate rounded-lg border border-dashed border-ui-line bg-ui-inset px-3 text-xs text-ui-muted"
            title={engineDefault ?? inheritedLabel}
          >
            {engineDefault ?? inheritedLabel}
          </div>
        ) : (
          <NumericInput
            label={label}
            value={value}
            min={min}
            max={max}
            step={step ?? 1}
            onChange={onChange}
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
        className="console-switch my-2 disabled:cursor-not-allowed"
      />
    </FieldFrame>
)}

export const Select = ({ label, value, onChange, options, title, disabled, defaultLabel, fieldKey }: { label: string; value: string; onChange: (v: string) => void; options: string[]; title?: string; disabled?: boolean; defaultLabel?: string; fieldKey?: FieldKey }) => {
  return (
    <FieldFrame label={label} fieldKey={fieldKey} title={title} disabled={disabled}>
      <SelectInput aria-label={label} value={value} onChange={e => onChange(e.target.value)} disabled={disabled} className="h-10 w-full">
      {options.map(o => <option key={o} value={o}>{o || defaultLabel || '\u9ED8\u8BA4'}</option>)}
      </SelectInput>
    </FieldFrame>
)}

export const SpecTypeMultiSelect = ({
  label,
  value,
  onChange,
  title,
  disabled,
  defaultLabel,
  priorityLabel,
  fieldKey,
}: {
  label: string
  value: string
  onChange: (value: string) => void
  title?: string
  disabled?: boolean
  defaultLabel: string
  priorityLabel: string
  fieldKey?: FieldKey
}) => {
  const runtime = useContext(FieldRuntimeCtx)
  const popupId = useId()
  const triggerRef = useRef<HTMLButtonElement>(null)
  const popupRef = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false)
  const [popupPosition, setPopupPosition] = useState<{
    top?: number
    bottom?: number
    left: number
    width: number
    maxHeight: number
  }>({ top: 0, left: 0, width: 0, maxHeight: 360 })
  const normalized = normalizeSpeculativeTypes(value)
  const selected = parseSpeculativeTypes(normalized)
  const reported = runtime.capabilities?.speculativeTypes?.filter(Boolean) ?? []
  const sourceOptions = reported.length > 0 ? reported : [...FALLBACK_SPECULATIVE_TYPES]
  const options = orderSpeculativeTypeOptions([...sourceOptions, ...selected])
    .filter(option => option !== 'none')
  const reportedSet = new Set(reported)
  const toggle = (type: string) => {
    const next = new Set(selected)
    if (next.has(type)) next.delete(type)
    else next.add(type)
    onChange(normalizeSpeculativeTypes([...next].join(',')))
  }

  const updatePopupPosition = useCallback(() => {
    const rect = triggerRef.current?.getBoundingClientRect()
    if (!rect) return

    const viewportPadding = 12
    const gap = 6
    const availableWidth = Math.max(160, window.innerWidth - viewportPadding * 2)
    const width = Math.min(Math.max(rect.width, 280), availableWidth)
    const left = Math.max(
      viewportPadding,
      Math.min(rect.left, window.innerWidth - width - viewportPadding),
    )
    const spaceBelow = Math.max(0, window.innerHeight - rect.bottom - gap - viewportPadding)
    const spaceAbove = Math.max(0, rect.top - gap - viewportPadding)
    const placeAbove = spaceBelow < 240 && spaceAbove > spaceBelow
    const maxHeight = Math.max(96, Math.min(360, placeAbove ? spaceAbove : spaceBelow))

    setPopupPosition(placeAbove
      ? {
          bottom: window.innerHeight - rect.top + gap,
          left,
          width,
          maxHeight,
        }
      : {
          top: rect.bottom + gap,
          left,
          width,
          maxHeight,
        })
  }, [])

  useEffect(() => {
    if (!open) return

    updatePopupPosition()
    const closeOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target as Node | null
      if (!target) return
      if (triggerRef.current?.contains(target) || popupRef.current?.contains(target)) return
      setOpen(false)
    }
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      setOpen(false)
      triggerRef.current?.focus()
    }

    document.addEventListener('pointerdown', closeOnOutsidePointer)
    document.addEventListener('keydown', closeOnEscape)
    window.addEventListener('resize', updatePopupPosition)
    window.addEventListener('scroll', updatePopupPosition, true)
    return () => {
      document.removeEventListener('pointerdown', closeOnOutsidePointer)
      document.removeEventListener('keydown', closeOnEscape)
      window.removeEventListener('resize', updatePopupPosition)
      window.removeEventListener('scroll', updatePopupPosition, true)
    }
  }, [open, updatePopupPosition])

  return (
    <FieldFrame label={label} fieldKey={fieldKey} title={title} disabled={disabled}>
      <div
        className="min-w-0"
        data-spec-type-source={reported.length > 0 ? 'engine' : 'fallback'}
      >
        <button
          ref={triggerRef}
          type="button"
          aria-label={label}
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-controls={open ? popupId : undefined}
          onClick={() => {
            if (disabled) return
            if (!open) updatePopupPosition()
            setOpen(current => !current)
          }}
          disabled={disabled}
          className="flex h-10 w-full items-center gap-2 rounded-lg border border-ui-control bg-ui-surface px-3 text-left text-sm transition hover:border-ui-control focus:outline-none focus-visible:ring-2 focus-visible:ring-ui-focus disabled:cursor-not-allowed"
        >
          <span className="min-w-0 flex-1 truncate font-mono text-xs">
            {normalized || defaultLabel}
          </span>
          <ChevronDown className={`h-4 w-4 shrink-0 text-ui-muted transition ${open ? 'rotate-180' : ''}`} />
        </button>
        {open && createPortal(
          <div
            ref={popupRef}
            id={popupId}
            role="listbox"
            aria-label={label}
            aria-multiselectable="true"
            data-spec-type-popup="true"
            className="console-overlay fixed z-[110] overflow-y-auto rounded-lg border border-ui-line bg-ui-surface p-2 shadow-2xl"
            style={popupPosition}
          >
            <div className="mb-2 grid grid-cols-2 gap-2">
              <button
                type="button"
                onClick={() => onChange('')}
                disabled={disabled}
                className={`rounded-md border px-2 py-1 text-xs ${normalized === '' ? 'border-ui-action-line bg-ui-selected text-ui-action-ink' : 'border-ui-line text-ui-muted'}`}
              >
                {defaultLabel}
              </button>
              <button
                type="button"
                onClick={() => onChange('none')}
                disabled={disabled}
                className={`rounded-md border px-2 py-1 font-mono text-xs ${normalized === 'none' ? 'border-ui-action-line bg-ui-selected text-ui-action-ink' : 'border-ui-line text-ui-muted'}`}
              >
                none
              </button>
            </div>
            <div className="max-h-52 space-y-1 overflow-y-auto" role="group">
              {options.map(option => {
                const checked = selected.includes(option)
                const engineUnreported = reported.length > 0 && !reportedSet.has(option)
                return (
                  <label
                    key={option}
                    role="option"
                    aria-selected={checked}
                    className={`flex items-center gap-2 rounded-md px-2 py-1.5 text-xs hover:bg-ui-hover ${engineUnreported ? 'text-ui-warn' : 'text-ui-body'}`}
                  >
                    <input
                      type="checkbox"
                      aria-label={option}
                      checked={checked}
                      onChange={() => toggle(option)}
                      disabled={disabled}
                      className="h-3.5 w-3.5 rounded"
                    />
                    <span className="font-mono">{option}</span>
                    {engineUnreported && <span className="ml-auto text-[10px]">!</span>}
                  </label>
                )
              })}
            </div>
            <p className="mt-2 border-t border-ui-line pt-2 text-[11px] leading-4 text-ui-muted">{priorityLabel}</p>
          </div>,
          document.body,
        )}
      </div>
    </FieldFrame>
  )
}

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
    className="ml-auto shrink-0 rounded-lg p-1 text-ui-muted transition hover:bg-ui-danger-bg hover:text-ui-danger"
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
    <div id={id} className="scroll-mt-6 overflow-hidden rounded-lg border border-ui-line bg-ui-inset" data-config-field={fieldKey} data-config-search-match={fieldState.match ? 'true' : undefined} data-config-emitted={fieldState.emitted ? 'true' : undefined}>
      <div className="flex items-center border-b border-ui-line bg-ui-inset text-ui-body transition hover:bg-ui-hover">
        <button type="button" aria-expanded={hasSearch || open} onClick={() => setOpen(!open)} className="flex min-w-0 flex-1 items-center gap-2 px-3 py-2.5 text-left">
          {(hasSearch || open) ? <ChevronDown className="h-3.5 w-3.5 shrink-0 text-ui-muted" /> : <ChevronRight className="h-3.5 w-3.5 shrink-0 text-ui-muted" />}
          <span className={`text-xs font-medium ${fieldState.match ? 'text-ui-warn' : 'text-ui-body'}`}>{title}</span>
          {summary && <span className="ml-auto text-xs text-ui-muted">{summary}</span>}
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
    lookup_cache_static: '', lookup_cache_dynamic: '', lookup_cache_managed: false,
    cache_type_draft_k: '', cache_type_draft_v: '',
    spec_default: false, spec_draft_backend_sampling: true,
    spec_draft_threads: 0, spec_draft_threads_batch: 0,
    spec_draft_n_ubatch: 0, spec_draft_sampling: '',
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
    moe_cache_mib: 0,
    perf: false, check_tensors: false, fit: false, fit_mode: '', fit_target: '', fit_ctx: 4096,
    numa_mode: '',
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
    tools_runtime: '', mcp_servers_config: '', mcp_servers_json: '',
    rpc_servers: '', sse_ping_interval: 30, reuse_port: false,
  },
  advancedMulti: {
    models_dir: '', models_preset: '', models_max: 4, models_autoload: false,
    mmproj_url: '', mmproj_auto: false, mmproj_mode: '', image_min_tokens: 0, image_max_tokens: 0,
    media_path: '', tags: '', tools: '',
  },
}
