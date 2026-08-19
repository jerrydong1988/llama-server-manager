import { AlertTriangle, Gauge, LoaderCircle, RefreshCw } from 'lucide-react'
import type { ResourceBudget, ResourcePlan, ResourcePlanStatus, ResourceRange } from '../../store'
import { getResourcePlanLabels, getResourcePlanReason, type ResourcePlanLabels } from '../../i18n/resourcePlanCopy'
import { formatSize } from '../../utils/format'
import { Badge, Button, InsetSurface } from '../ui'

const statusTone: Record<ResourcePlanStatus, 'emerald' | 'amber' | 'red' | 'slate'> = {
  feasible: 'emerald',
  constrained: 'amber',
  infeasible: 'red',
  unknown: 'slate',
}

function formatRange(range: ResourceRange) {
  return `${formatSize(range.expectedBytes)} / ${formatSize(range.minBytes)}–${formatSize(range.maxBytes)}`
}

function formatSignedSize(value: number | null) {
  if (value === null) return '--'
  return `${value < 0 ? '−' : '+'}${formatSize(Math.abs(value))}`
}

function BudgetCard({ title, budget, labels }: { title: string; budget: ResourceBudget; labels: ResourcePlanLabels }) {
  return (
    <InsetSurface className="p-3">
      <p className="text-xs font-semibold uppercase tracking-wide text-slate-500">{title}</p>
      <dl className="mt-2 space-y-1.5 text-xs">
        <div className="flex items-start justify-between gap-3"><dt className="text-slate-500">{labels.required}</dt><dd className="text-right font-medium text-slate-800 dark:text-slate-200">{formatRange(budget.required)}</dd></div>
        <div className="flex items-start justify-between gap-3"><dt className="text-slate-500">{labels.available}</dt><dd className="font-medium text-slate-800 dark:text-slate-200">{budget.availableBytes === null ? '--' : formatSize(budget.availableBytes)}</dd></div>
        <div className="flex items-start justify-between gap-3"><dt className="text-slate-500">{labels.reserve}</dt><dd className="font-medium text-slate-800 dark:text-slate-200">{formatSize(budget.reservedBytes)}</dd></div>
        <div className="flex items-start justify-between gap-3"><dt className="text-slate-500">{labels.headroom}</dt><dd className={budget.expectedHeadroomBytes !== null && budget.expectedHeadroomBytes < 0 ? 'font-medium text-red-500' : 'font-medium text-emerald-500'}>{formatSignedSize(budget.expectedHeadroomBytes)}</dd></div>
      </dl>
    </InsetSurface>
  )
}

export function ResourcePlanPanel({
  plan,
  loading,
  error,
  lang,
  onRefresh,
}: {
  plan: ResourcePlan | null
  loading: boolean
  error: boolean
  lang: string
  onRefresh: () => void
}) {
  const labels = getResourcePlanLabels(lang)
  const localized = (code: string) => getResourcePlanReason(lang, code)

  return (
    <InsetSurface className="p-4" data-guide="resource-plan">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-3">
          <div className="rounded-lg border border-violet-500/20 bg-violet-500/10 p-2.5 text-violet-400"><Gauge className="h-5 w-5" /></div>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <p className="text-sm font-semibold text-slate-900 dark:text-slate-100">{labels.title}</p>
              {plan && <Badge tone={statusTone[plan.status]}>{labels.statuses[plan.status]}</Badge>}
              {plan && <Badge tone="slate">{labels.confidence}: {labels.confidences[plan.confidence]}</Badge>}
            </div>
            <p className="mt-1 text-xs leading-5 text-slate-500">{labels.description}</p>
          </div>
        </div>
        <Button size="sm" variant="subtle" onClick={onRefresh} disabled={loading} icon={loading ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}>
          {labels.refresh}
        </Button>
      </div>

      {!plan ? (
        <div className={`mt-3 flex items-center gap-2 rounded-lg border px-3 py-2 text-xs ${error ? 'border-red-500/20 bg-red-500/10 text-red-500' : 'border-slate-200 bg-white text-slate-500 dark:border-slate-800 dark:bg-slate-900'}`}>
          {error ? <AlertTriangle className="h-4 w-4" /> : loading ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <Gauge className="h-4 w-4" />}
          {loading ? labels.loading : labels.unavailable}
        </div>
      ) : (
        <>
          <div className="mt-3 grid gap-2 sm:grid-cols-2">
            <BudgetCard title={labels.ram} budget={plan.ram} labels={labels} />
            <BudgetCard title={labels.vram} budget={plan.vram} labels={labels} />
          </div>
          <div className="mt-3 grid grid-cols-2 gap-2 text-xs text-slate-500">
            <span>{labels.context}: <strong className="text-slate-800 dark:text-slate-200">{plan.facts.contextTokens.toLocaleString()}</strong></span>
            <span>{labels.slots}: <strong className="text-slate-800 dark:text-slate-200">{plan.facts.parallelSlots}</strong></span>
            <span>{labels.offload}: <strong className="text-slate-800 dark:text-slate-200">{plan.facts.gpuOffloadPercent}%</strong></span>
            <span>{labels.shards}: <strong className="text-slate-800 dark:text-slate-200">{plan.facts.modelShardsFound}/{plan.facts.modelShardsExpected}</strong></span>
          </div>
          {plan.reasons.length > 0 && (
            <div className="mt-3">
              <p className="text-xs font-semibold text-slate-700 dark:text-slate-300">{labels.reasons}</p>
              <ul className="mt-1 space-y-1 text-xs leading-5 text-slate-500">{plan.reasons.map(code => <li key={code}>• {localized(code)}</li>)}</ul>
            </div>
          )}
          {plan.assumptions.length > 0 && (
            <details className="mt-3 text-xs text-slate-500">
              <summary className="cursor-pointer font-semibold text-slate-700 dark:text-slate-300">{labels.assumptions} ({plan.assumptions.length})</summary>
              <ul className="mt-1 space-y-1 leading-5">{plan.assumptions.map(code => <li key={code}>• {localized(code)}</li>)}</ul>
            </details>
          )}
        </>
      )}
    </InsetSurface>
  )
}
