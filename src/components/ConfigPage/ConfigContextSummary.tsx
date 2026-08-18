import { Settings } from 'lucide-react'
import type { Translations } from '../../i18n'
import { getConfigPageLabels } from '../../i18n/configPageCopy'
import type { Warning } from '../../validators'
import { InsetSurface, PathText } from '../ui'

type Labels = ReturnType<typeof getConfigPageLabels>

export function ConfigContextSummary({
  instanceName,
  endpoint,
  primaryModelPath,
  draftModelPath,
  engineName,
  engineDir,
  isEmbedding,
  modifiedCount,
  warningCounts,
  checkMessages,
  visibleWarnings,
  labels,
  t,
}: {
  instanceName?: string
  endpoint: string
  primaryModelPath: string
  draftModelPath: string
  engineName?: string
  engineDir?: string
  isEmbedding: boolean
  modifiedCount: number
  warningCounts: { high: number; medium: number; low: number }
  checkMessages: Array<{ tone: string; text: string }>
  visibleWarnings: Warning[]
  labels: Labels
  t: Translations
}) {
  const rows = [
    { label: labels.primaryModel, value: primaryModelPath || '--', path: Boolean(primaryModelPath) },
    { label: labels.draftModel, value: draftModelPath || '--', path: Boolean(draftModelPath) },
    { label: labels.engine, value: engineName || '--' },
    { label: labels.enginePath, value: engineDir || '--', path: Boolean(engineDir) },
    { label: labels.endpoint, value: endpoint },
    { label: labels.embeddingMode, value: isEmbedding ? labels.on : labels.off },
    { label: labels.modifiedParams, value: String(modifiedCount) },
  ]

  return (
    <>
      <InsetSurface className="p-4">
        <div className="flex items-start gap-3">
          <div className="rounded-lg border border-slate-200 bg-white p-3 text-slate-700 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-300">
            <Settings className="h-5 w-5" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-medium text-slate-900 dark:text-slate-100" title={instanceName}>{instanceName}</p>
            <PathText value={endpoint} maxLength={36} className="mt-1 text-slate-500" />
          </div>
        </div>
      </InsetSurface>

      <InsetSurface className="space-y-3 p-4">
        {rows.map(row => (
          <div key={row.label} className="grid min-w-0 grid-cols-[96px_minmax(0,1fr)] items-start gap-3">
            <span className="truncate text-sm text-slate-500" title={row.label}>{row.label}</span>
            {row.path ? (
              <PathText value={row.value} maxLength={44} className="text-right text-slate-700 dark:text-slate-200" />
            ) : (
              <span className="min-w-0 truncate text-right text-sm text-slate-700 dark:text-slate-200" title={row.value}>
                {row.value}
              </span>
            )}
          </div>
        ))}
      </InsetSurface>

      <InsetSurface className="p-4">
        <p className="text-sm font-medium text-slate-900 dark:text-slate-100">{labels.validationSummary}</p>
        <div className="mt-3 grid grid-cols-3 gap-2 text-center">
          {[
            [labels.high, warningCounts.high, 'text-red-300 border-red-500/20 bg-red-500/10'],
            [labels.medium, warningCounts.medium, 'text-amber-300 border-amber-500/20 bg-amber-500/10'],
            [labels.low, warningCounts.low, 'text-sky-300 border-sky-500/20 bg-sky-500/10'],
          ].map(([label, count, tone]) => (
            <div key={label} className={`rounded-lg border px-2 py-3 ${tone}`}>
              <p className="text-lg font-semibold">{count}</p>
              <p className="mt-1 text-[11px] uppercase tracking-[0.14em]">{label}</p>
            </div>
          ))}
        </div>

        <div className="mt-4 space-y-2">
          {checkMessages.length === 0 ? (
            <div className="rounded-lg border border-emerald-200 bg-emerald-50 px-3 py-2 text-sm text-emerald-700 dark:border-emerald-500/20 dark:bg-emerald-500/10 dark:text-emerald-200">
              {labels.checkPassed}
            </div>
          ) : checkMessages.map((message, index) => (
            <div
              key={`${message.text}-${index}`}
              className={`rounded-lg px-3 py-2 text-sm ${
                message.tone === 'red' ? 'bg-red-50 text-red-700 dark:bg-red-500/10 dark:text-red-200'
                  : message.tone === 'amber' ? 'bg-amber-50 text-amber-700 dark:bg-amber-500/10 dark:text-amber-200'
                    : 'bg-sky-50 text-sky-700 dark:bg-sky-500/10 dark:text-sky-200'
              }`}
            >
              {message.text}
            </div>
          ))}
        </div>

        {visibleWarnings.length > 0 && (
          <div className="mt-4 space-y-2">
            {visibleWarnings.slice(0, 6).map((warning, index) => (
              <div
                key={`${warning.key}-${index}`}
                className={`rounded-lg px-3 py-2 text-sm ${
                  warning.severity === 'high'
                    ? 'bg-red-50 text-red-700 dark:bg-red-500/10 dark:text-red-200'
                    : warning.severity === 'medium'
                      ? 'bg-amber-50 text-amber-700 dark:bg-amber-500/10 dark:text-amber-200'
                      : 'bg-sky-500/10 text-sky-200'
                }`}
              >
                {t.configPage[warning.key] || warning.key}
              </div>
            ))}
          </div>
        )}
      </InsetSurface>
    </>
  )
}
