import { useMemo, useState } from 'react'
import { Crosshair, LoaderCircle, Undo2 } from 'lucide-react'
import type { InstanceConfig } from '../../store'
import type { Translations } from '../../i18n'
import type { getConfigPageLabels } from '../../i18n/configPageCopy'
import { Badge, Button, InsetSurface } from '../ui'
import { fieldLabel, type ConfigChange } from './configWorkspace'

type Labels = ReturnType<typeof getConfigPageLabels>
type PanelTab = 'changes' | 'emitted'

const SYSTEM_MANAGED_KEYS = new Set<keyof InstanceConfig>([
  'model_path', 'host', 'port', 'api_key', 'api_key_file',
  'metrics', 'props', 'slots_enabled', 'embedding', 'pooling', 'reranking',
])

export function ConfigChangePanel({
  changes,
  emittedKeys,
  currentOverrideKeys,
  baselineOverrideKeys,
  previewing,
  labels,
  t,
  onLocate,
  onUndo,
}: {
  changes: ConfigChange[]
  emittedKeys: Array<keyof InstanceConfig>
  currentOverrideKeys: Array<keyof InstanceConfig>
  baselineOverrideKeys: Array<keyof InstanceConfig>
  previewing: boolean
  labels: Labels
  t: Translations
  onLocate: (key: keyof InstanceConfig) => void
  onUndo: (key: keyof InstanceConfig) => void
}) {
  const [tab, setTab] = useState<PanelTab>('changes')
  const emitted = useMemo(() => new Set(emittedKeys), [emittedKeys])
  const currentOverrides = useMemo(() => new Set(currentOverrideKeys), [currentOverrideKeys])
  const baselineOverrides = useMemo(() => new Set(baselineOverrideKeys), [baselineOverrideKeys])
  const emittedRows = useMemo(() => emittedKeys
    .map(key => ({ key, label: fieldLabel(key, t) }))
    .sort((left, right) => left.label.localeCompare(right.label)), [emittedKeys, t])

  const statusFor = (key: keyof InstanceConfig) => {
    if (emitted.has(key)) return { text: labels.changeWillEmit, tone: 'blue' as const }
    if (SYSTEM_MANAGED_KEYS.has(key)) return { text: labels.changeSystemManaged, tone: 'slate' as const }
    if (!currentOverrides.has(key) && baselineOverrides.has(key)) return { text: labels.changeStopsEmitting, tone: 'amber' as const }
    if (currentOverrides.has(key)) return { text: labels.changeWaitingDependency, tone: 'slate' as const }
    return { text: labels.changeNotEmitted, tone: 'slate' as const }
  }

  return (
    <InsetSurface className="p-4">
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.changeReview}</p>
          <p className="mt-1 text-sm text-slate-500">{labels.changeReviewDesc}</p>
        </div>
        {previewing && <LoaderCircle className="mt-0.5 h-4 w-4 shrink-0 animate-spin text-blue-400" />}
      </div>

      <div className="mt-4 grid grid-cols-2 rounded-lg border border-slate-200 bg-slate-100 p-1 dark:border-slate-800 dark:bg-slate-950/60">
        {([
          ['changes', labels.changeTab, changes.length],
          ['emitted', labels.emittedTab, emittedKeys.length],
        ] as const).map(([value, title, count]) => (
          <button
            key={value}
            type="button"
            onClick={() => setTab(value)}
            className={`rounded-md px-2 py-2 text-xs font-medium transition ${tab === value ? 'bg-white text-slate-900 shadow-sm dark:bg-slate-800 dark:text-white' : 'text-slate-500 hover:text-slate-800 dark:hover:text-slate-200'}`}
          >
            {title} {count}
          </button>
        ))}
      </div>

      <div className="mt-3 max-h-[430px] space-y-2 overflow-y-auto pr-1">
        {tab === 'changes' && (changes.length === 0 ? (
          <p className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-3 text-sm text-slate-500 dark:border-slate-800 dark:bg-slate-950/40">{labels.noConfigDiff}</p>
        ) : changes.map(change => {
          const status = statusFor(change.key)
          return (
            <div key={change.key} className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-3 dark:border-slate-800 dark:bg-slate-950/40">
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium text-slate-800 dark:text-slate-200" title={change.label}>{change.label}</p>
                  <p className="mt-1 truncate font-mono text-[11px] text-slate-500" title={String(change.key)}>{String(change.key)}</p>
                </div>
                <Badge tone={status.tone} className="shrink-0 px-2 py-0.5 text-[10px]">{status.text}</Badge>
              </div>
              <div className="mt-2 grid min-w-0 grid-cols-[44px_minmax(0,1fr)] gap-x-2 gap-y-1 text-xs">
                <span className="text-slate-500">{labels.before}</span>
                <span className="min-w-0 truncate text-slate-500" title={change.before}>{change.before}</span>
                <span className="text-slate-500">{labels.after}</span>
                <span className="min-w-0 truncate text-slate-800 dark:text-slate-200" title={change.after}>{change.after}</span>
              </div>
              <div className="mt-3 flex justify-end gap-2">
                <Button onClick={() => onLocate(change.key)} variant="subtle" size="sm" icon={<Crosshair className="h-3.5 w-3.5" />}>{labels.locateParameter}</Button>
                <Button onClick={() => onUndo(change.key)} variant="secondary" size="sm" icon={<Undo2 className="h-3.5 w-3.5" />}>{labels.undoChange}</Button>
              </div>
            </div>
          )
        }))}

        {tab === 'emitted' && (emittedRows.length === 0 ? (
          <p className="rounded-lg border border-slate-200 bg-slate-50 px-3 py-3 text-sm text-slate-500 dark:border-slate-800 dark:bg-slate-950/40">{labels.noEmittedOverrides}</p>
        ) : emittedRows.map(row => (
          <button
            key={row.key}
            type="button"
            onClick={() => onLocate(row.key)}
            className="flex w-full items-center justify-between gap-3 rounded-lg border border-slate-200 bg-slate-50 px-3 py-3 text-left transition hover:border-blue-300 hover:bg-blue-50/50 dark:border-slate-800 dark:bg-slate-950/40 dark:hover:border-blue-500/30 dark:hover:bg-blue-500/5"
          >
            <span className="min-w-0">
              <span className="block truncate text-sm font-medium text-slate-800 dark:text-slate-200">{row.label}</span>
              <span className="mt-1 block truncate font-mono text-[11px] text-slate-500">{String(row.key)}</span>
            </span>
            <span className="shrink-0 text-xs text-blue-600 dark:text-blue-300">{labels.locateParameter}</span>
          </button>
        )))}
      </div>

      <p className="mt-3 text-xs leading-5 text-slate-500">{labels.emittedSystemHint}</p>
    </InsetSurface>
  )
}
