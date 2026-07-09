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
        className="flex w-full items-start justify-between gap-3 border-b border-slate-200 px-4 py-4 text-left dark:border-slate-800"
      >
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <Settings2 className="h-4 w-4 text-slate-500" />
            <span className="text-sm font-semibold text-slate-900 dark:text-slate-100">{copy.strategyTitle}</span>
          </div>
          <div className="mt-1 text-xs leading-5 text-slate-500 dark:text-slate-400">{copy.strategySub}</div>
        </div>
        {open ? <ChevronUp className="mt-0.5 h-4 w-4 shrink-0 text-slate-400" /> : <ChevronDown className="mt-0.5 h-4 w-4 shrink-0 text-slate-400" />}
      </button>

      {open && (
        <div className="space-y-5 px-4 py-4">
          {settingsError && (
            <div className="rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-xs leading-5 text-amber-800 dark:border-amber-900/60 dark:bg-amber-950/30 dark:text-amber-200">
              {settingsError}
            </div>
          )}

          <div className="grid grid-cols-2 gap-2">
            <div className="rounded-lg border border-slate-200 px-3 py-3 dark:border-slate-800">
              <div className="text-[11px] font-medium text-slate-500 dark:text-slate-400">{copy.strategy}</div>
              <div className="mt-1 text-sm font-semibold text-slate-900 dark:text-slate-100">{resumePolicyLabel}</div>
            </div>
            <div className="rounded-lg border border-slate-200 px-3 py-3 dark:border-slate-800">
              <div className="text-[11px] font-medium text-slate-500 dark:text-slate-400">{copy.bandwidth}</div>
              <div className="mt-1 text-sm font-semibold text-slate-900 dark:text-slate-100">{bandwidthSummary}</div>
            </div>
          </div>

          <div className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <label className="text-xs font-medium text-slate-500 dark:text-slate-400">{labels.resumePolicy}</label>
              <BackendBadge label={copy.backendSaved} />
            </div>
            <div className="grid grid-cols-2 gap-2 rounded-lg bg-slate-100 p-1 dark:bg-slate-800">
              <button
                type="button"
                onClick={() => onResumePolicyChange('manual')}
                className={`rounded-md px-3 py-2 text-xs font-medium transition ${resumePolicy === 'manual' ? 'bg-white text-slate-900 shadow-sm dark:bg-slate-950 dark:text-white' : 'text-slate-500 hover:text-slate-900 dark:text-slate-400 dark:hover:text-white'}`}
              >
                {labels.resumeManual}
              </button>
              <button
                type="button"
                onClick={() => onResumePolicyChange('auto_on_launch')}
                className={`rounded-md px-3 py-2 text-xs font-medium transition ${resumePolicy === 'auto_on_launch' ? 'bg-white text-slate-900 shadow-sm dark:bg-slate-950 dark:text-white' : 'text-slate-500 hover:text-slate-900 dark:text-slate-400 dark:hover:text-white'}`}
              >
                {labels.resumeAuto}
              </button>
            </div>
          </div>

          <div className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <label className="text-xs font-medium text-slate-500 dark:text-slate-400">{labels.maxConcurrent}</label>
              <BackendBadge label={copy.backendSaved} />
            </div>
            <div className="flex items-center gap-3">
              <input
                type="range"
                min={1}
                max={8}
                value={concurrency}
                onChange={event => onConcurrencyChange(parseInt(event.target.value, 10))}
                className="min-w-0 flex-1 accent-blue-600"
              />
              <input
                type="number"
                min={1}
                max={8}
                value={concurrency}
                onChange={event => onConcurrencyChange(parseInt(event.target.value, 10))}
                className="w-24 rounded-lg border border-slate-200 bg-white px-3 py-2.5 text-center text-sm text-slate-900 outline-none transition focus:border-blue-500 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-100"
              />
            </div>
            <div className="text-[11px] text-slate-400">1-8</div>
          </div>

          <div className="space-y-3 border-t border-slate-200 pt-4 dark:border-slate-800">
            <div className="flex items-center justify-between gap-3">
              <label className="text-xs font-medium text-slate-500 dark:text-slate-400">{copy.bandwidth}</label>
              <BackendBadge label={copy.backendSaved} />
            </div>
            <div className="grid grid-cols-[minmax(0,1fr)_96px] gap-2">
              <input
                type="number"
                min={0}
                step={bandwidthUnit === 'MiB/s' ? 1 : 64}
                value={bandwidthLimit}
                onChange={event => onBandwidthLimitChange(Number(event.target.value))}
                className="min-w-0 rounded-lg border border-slate-200 bg-white px-3 py-2.5 text-sm text-slate-900 outline-none transition focus:border-blue-500 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-100"
              />
              <select
                value={bandwidthUnit}
                onChange={event => onBandwidthUnitChange(event.target.value as DownloadBandwidthUnit)}
                aria-label={copy.displayUnit}
                className="rounded-lg border border-slate-200 bg-white px-2 py-2.5 text-sm text-slate-900 outline-none transition focus:border-blue-500 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-100"
              >
                <option value="MiB/s">MiB/s</option>
                <option value="KiB/s">KiB/s</option>
              </select>
            </div>
            <div className="text-[11px] leading-5 text-slate-500 dark:text-slate-400">{copy.limitHelp}</div>
          </div>

          <div className="space-y-3 border-t border-slate-200 pt-4 dark:border-slate-800">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <div className="text-xs font-medium text-slate-600 dark:text-slate-300">{copy.lowPriorityThrottle}</div>
                  <BackendBadge label={copy.backendSaved} />
                </div>
                <div className="mt-1 text-[11px] leading-5 text-slate-400">{copy.throttleHelp}</div>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={lowPriorityThrottle}
                onClick={() => onLowPriorityThrottleChange(!lowPriorityThrottle)}
                className={`relative mt-0.5 h-6 w-11 shrink-0 rounded-full transition ${lowPriorityThrottle ? 'bg-blue-600' : 'bg-slate-300 dark:bg-slate-700'}`}
              >
                <span className={`absolute top-1 h-4 w-4 rounded-full bg-white shadow-sm transition ${lowPriorityThrottle ? 'left-6' : 'left-1'}`} />
              </button>
            </div>
          </div>

          <button
            type="button"
            onClick={onResetDefaults}
            className="inline-flex w-full items-center justify-center gap-2 rounded-lg border border-slate-200 bg-white px-3 py-2.5 text-xs font-medium text-slate-600 transition hover:bg-slate-50 hover:text-slate-900 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-300 dark:hover:bg-slate-900 dark:hover:text-white"
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
