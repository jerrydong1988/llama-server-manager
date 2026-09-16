import { useState } from 'react'
import { useI18n } from '../../i18n'
import { getRouterManagementLabels } from '../../i18n/routerManagement'
import { TextInput } from '../ui'

type Limits = { dailyTokenLimit: number; monthlyTokenLimit: number }

export function QuotaSettings({ value, onChange }: { value: Limits; onChange: (patch: Partial<Limits>) => void }) {
  const { lang } = useI18n()
  const labels = getRouterManagementLabels(lang)
  const [expanded, setExpanded] = useState(value.dailyTokenLimit > 0 || value.monthlyTokenLimit > 0)
  return <details className="mt-3 rounded-lg border border-slate-200 p-3 dark:border-slate-700/60" open={expanded} onToggle={event => setExpanded(event.currentTarget.open)}>
    <summary className="cursor-pointer text-xs font-medium">{labels.hardQuota}</summary>
    <p className="mt-2 text-xs leading-5 text-slate-500 dark:text-slate-400">{labels.quotaHint}</p>
    <div className="mt-3 grid gap-3 sm:grid-cols-2">
      {([['dailyTokenLimit', labels.dayLimit], ['monthlyTokenLimit', labels.monthLimit]] as const).map(([field, title]) => <label key={field} className="text-xs">
        <span className="mb-1 block text-slate-500 dark:text-slate-400">{title}</span>
        <TextInput aria-label={title} type="number" min={0} max={Number.MAX_SAFE_INTEGER} step={1} value={value[field]} onChange={event => onChange({ [field]: Math.min(Number.MAX_SAFE_INTEGER, Math.max(0, Math.floor(Number(event.target.value) || 0))) })} />
      </label>)}
    </div>
  </details>
}
