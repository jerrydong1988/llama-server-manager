import { ChevronDown, ChevronUp, RotateCcw, Settings2 } from 'lucide-react'
import { surfaceClassName } from '../ui'

export type DownloadResumePolicy = 'manual' | 'auto_on_launch'
export type DownloadBandwidthUnit = 'MiB/s' | 'KiB/s'

export type DownloadSettingsCopy = {
  strategyTitle: string
  strategySub: string
  strategy: string
  bandwidth: string
  backendSaved: string
  displayUnit: string
  limitHelp: string
  lowPriorityThrottle: string
  throttleHelp: string
  resetDefaults: string
}

type DownloadSettingsLabels = {
  resumePolicy: string
  resumeManual: string
  resumeAuto: string
  maxConcurrent: string
}

type DownloadSettingsPanelProps = {
  open: boolean
  onToggle: () => void
  copy: DownloadSettingsCopy
  labels: DownloadSettingsLabels
  settingsError: string
  resumePolicy: DownloadResumePolicy
  resumePolicyLabel: string
  bandwidthSummary: string
  concurrency: number
  bandwidthLimit: number
  bandwidthUnit: DownloadBandwidthUnit
  lowPriorityThrottle: boolean
  onResumePolicyChange: (policy: DownloadResumePolicy) => void
  onConcurrencyChange: (value: number) => void
  onBandwidthLimitChange: (value: number) => void
  onBandwidthUnitChange: (unit: DownloadBandwidthUnit) => void
  onLowPriorityThrottleChange: (enabled: boolean) => void
  onResetDefaults: () => void
}

export function DownloadSettingsPanel({
  open,
  onToggle,
  copy,
  labels,
  settingsError,
  resumePolicy,
  resumePolicyLabel,
  bandwidthSummary,
  concurrency,
  bandwidthLimit,
  bandwidthUnit,
  lowPriorityThrottle,
  onResumePolicyChange,
  onConcurrencyChange,
  onBandwidthLimitChange,
  onBandwidthUnitChange,
  onLowPriorityThrottleChange,
  onResetDefaults,
}: DownloadSettingsPanelProps) {
  return (
    <section className={`${surfaceClassName} min-w-0 overflow-hidden`}>
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-start justify-between gap-3 border-b border-ui-line px-4 py-4 text-left"
      >
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <Settings2 className="h-4 w-4 text-ui-muted" />
            <span className="text-sm font-semibold text-ui-ink">{copy.strategyTitle}</span>
          </div>
          <div className="mt-1 text-xs leading-5 text-ui-muted">{copy.strategySub}</div>
        </div>
        {open ? <ChevronUp className="mt-0.5 h-4 w-4 shrink-0 text-ui-muted" /> : <ChevronDown className="mt-0.5 h-4 w-4 shrink-0 text-ui-muted" />}
      </button>

      {open && (
        <div className="space-y-5 px-4 py-4">
          {settingsError && (
            <div className="rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-xs leading-5 text-amber-800 dark:border-amber-900/60 dark:bg-amber-950/30 dark:text-amber-200">
              {settingsError}
            </div>
          )}

          <div className="grid grid-cols-2 gap-2">
            <div className="rounded-lg border border-ui-line px-3 py-3">
              <div className="text-[11px] font-medium text-ui-muted">{copy.strategy}</div>
              <div className="mt-1 text-sm font-semibold text-ui-ink">{resumePolicyLabel}</div>
            </div>
            <div className="rounded-lg border border-ui-line px-3 py-3">
              <div className="text-[11px] font-medium text-ui-muted">{copy.bandwidth}</div>
              <div className="mt-1 text-sm font-semibold text-ui-ink">{bandwidthSummary}</div>
            </div>
          </div>

          <div className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <label className="text-xs font-medium text-ui-muted">{labels.resumePolicy}</label>
              <BackendBadge label={copy.backendSaved} />
            </div>
            <div className="grid grid-cols-2 gap-2 rounded-lg bg-ui-inset p-1">
              <button
                type="button"
                onClick={() => onResumePolicyChange('manual')}
                className={`rounded-md px-3 py-2 text-xs font-medium transition ${resumePolicy === 'manual' ? 'bg-ui-surface text-ui-ink shadow-sm' : 'text-ui-muted hover:text-ui-ink'}`}
              >
                {labels.resumeManual}
              </button>
              <button
                type="button"
                onClick={() => onResumePolicyChange('auto_on_launch')}
                className={`rounded-md px-3 py-2 text-xs font-medium transition ${resumePolicy === 'auto_on_launch' ? 'bg-ui-surface text-ui-ink shadow-sm' : 'text-ui-muted hover:text-ui-ink'}`}
              >
                {labels.resumeAuto}
              </button>
            </div>
          </div>

          <div className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <label className="text-xs font-medium text-ui-muted">{labels.maxConcurrent}</label>
              <BackendBadge label={copy.backendSaved} />
            </div>
            <div className="flex items-center gap-3">
              <input
                type="range"
                min={1}
                max={8}
                value={concurrency}
                onChange={event => onConcurrencyChange(parseInt(event.target.value, 10))}
                className="min-w-0 flex-1 accent-ui-positive"
              />
              <input
                type="number"
                min={1}
                max={8}
                value={concurrency}
                onChange={event => onConcurrencyChange(parseInt(event.target.value, 10))}
                className="w-24 rounded-lg border border-ui-line bg-ui-surface px-3 py-2.5 text-center text-sm text-ui-ink outline-none transition focus:border-ui-focus"
              />
            </div>
            <div className="text-[11px] text-ui-muted">1-8</div>
          </div>

          <div className="space-y-3 border-t border-ui-line pt-4">
            <div className="flex items-center justify-between gap-3">
              <label className="text-xs font-medium text-ui-muted">{copy.bandwidth}</label>
              <BackendBadge label={copy.backendSaved} />
            </div>
            <div className="grid grid-cols-[minmax(0,1fr)_96px] gap-2">
              <input
                type="number"
                min={0}
                step={bandwidthUnit === 'MiB/s' ? 1 : 64}
                value={bandwidthLimit}
                onChange={event => onBandwidthLimitChange(Number(event.target.value))}
                className="min-w-0 rounded-lg border border-ui-line bg-ui-surface px-3 py-2.5 text-sm text-ui-ink outline-none transition focus:border-ui-focus"
              />
              <select
                value={bandwidthUnit}
                onChange={event => onBandwidthUnitChange(event.target.value as DownloadBandwidthUnit)}
                aria-label={copy.displayUnit}
                className="rounded-lg border border-ui-line bg-ui-surface px-2 py-2.5 text-sm text-ui-ink outline-none transition focus:border-ui-focus"
              >
                <option value="MiB/s">MiB/s</option>
                <option value="KiB/s">KiB/s</option>
              </select>
            </div>
            <div className="text-[11px] leading-5 text-ui-muted">{copy.limitHelp}</div>
          </div>

          <div className="space-y-3 border-t border-ui-line pt-4">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <div className="text-xs font-medium text-ui-body">{copy.lowPriorityThrottle}</div>
                  <BackendBadge label={copy.backendSaved} />
                </div>
                <div className="mt-1 text-[11px] leading-5 text-ui-muted">{copy.throttleHelp}</div>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={lowPriorityThrottle}
                onClick={() => onLowPriorityThrottleChange(!lowPriorityThrottle)}
                className={`relative mt-0.5 h-6 w-11 shrink-0 rounded-full transition ${lowPriorityThrottle ? 'bg-ui-positive' : 'bg-ui-line'}`}
              >
                <span className={`absolute top-1 h-4 w-4 rounded-full bg-ui-surface shadow-sm transition ${lowPriorityThrottle ? 'left-6' : 'left-1'}`} />
              </button>
            </div>
          </div>

          <button
            type="button"
            onClick={onResetDefaults}
            className="inline-flex w-full items-center justify-center gap-2 rounded-lg border border-ui-line bg-ui-surface px-3 py-2.5 text-xs font-medium text-ui-body transition hover:bg-ui-hover hover:text-ui-ink"
          >
            <RotateCcw className="h-3.5 w-3.5" />
            <span>{copy.resetDefaults}</span>
          </button>
        </div>
      )}
    </section>
  )
}

function BackendBadge({ label }: { label: string }) {
  return (
    <span className="rounded-full bg-emerald-50 px-2 py-0.5 text-[11px] font-medium text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300">
      {label}
    </span>
  )
}
