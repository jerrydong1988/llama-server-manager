import { useState } from 'react'
import { RefreshCw } from 'lucide-react'
import { useI18n } from '../../i18n'
import { getBackgroundTaskLabels } from '../../i18n/backgroundTasks'
import { useAppStore } from '../../store'
import { Badge, Button, SelectInput, TextInput } from '../ui'
import { useManagementTasks } from './useManagementTasks'
import type { TaskStatus } from './types'

const tones = { queued: 'blue', running: 'amber', succeeded: 'emerald', failed: 'red', cancelled: 'slate', interrupted: 'violet' } as const
const resourceTabs: Record<string, string> = {
  models: 'model-repo', engines: 'engine', instances: 'instances', checkpoints: 'instances', presets: 'config',
  downloads: 'downloads', routes: 'proxy', usage: 'proxy', runtime: 'dashboard', storage: 'storage', monitoring: 'perf',
}

export function BackgroundTasksPanel({ onClose }: { onClose: () => void }) {
  const { lang } = useI18n()
  const copy = getBackgroundTaskLabels(lang)
  const model = useManagementTasks()
  const setActiveTab = useAppStore(state => state.setActiveTab)
  const [query, setQuery] = useState('')
  const [status, setStatus] = useState('')
  const normalized = query.trim().toLowerCase()
  const rows = model.tasks.filter(task => (!status || task.status === status)
    && (!normalized || `${task.id} ${task.operation}`.toLowerCase().includes(normalized)))
  const detail = model.detail
  const feedback = model.feedback?.id === model.selectedId ? model.feedback : null
  const resourceTab = detail && resourceTabs[detail.operation.split('.')[0]]
  const date = (value: number) => new Date(value).toLocaleString(lang)
  const badge = (state: TaskStatus) => <Badge tone={tones[state] ?? 'slate'}>{copy.states[state] ?? state}</Badge>

  return <section aria-label={copy.title} className="space-y-4 p-5" data-testid="background-tasks">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <p className="max-w-2xl text-sm text-slate-600 dark:text-slate-300">{copy.description}</p>
      <Button onClick={model.refresh} disabled={model.loading}>
        <RefreshCw aria-hidden="true" className={`h-4 w-4 ${model.loading ? 'animate-spin motion-reduce:animate-none' : ''}`} />{copy.refresh}
      </Button>
    </div>
    <p className="text-xs leading-5 text-slate-600 dark:text-slate-400">{copy.scope}</p>
    <div className="flex flex-col gap-3 sm:flex-row">
      <TextInput aria-label={copy.search} placeholder={copy.search} value={query} onChange={event => setQuery(event.target.value)} />
      <SelectInput aria-label={copy.filter} value={status} onChange={event => setStatus(event.target.value)}>
        <option value="">{copy.all}</option>
        {Object.entries(copy.states).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
      </SelectInput>
    </div>
    {model.listError && <p role="alert" className="break-words text-sm text-red-700 dark:text-red-300">{copy.stale} {model.listError}</p>}
    <div className="text-xs text-slate-600 dark:text-slate-400">{rows.length} / {model.tasks.length} {copy.records}</div>
    <div className="grid min-w-0 gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <div className="max-h-[48vh] min-w-0 space-y-2 overflow-y-auto p-1" aria-busy={model.loading}>
        {!rows.length && <p role="status" className="p-4 text-sm text-slate-600 dark:text-slate-300">{model.loading ? copy.loading : model.tasks.length ? copy.noMatches : copy.empty}</p>}
        {rows.map(task => <button type="button" key={task.id} data-testid={`task-${task.id}`} aria-pressed={model.selectedId === task.id}
          onClick={() => model.selectTask(task.id)}
          className={`w-full rounded-lg border p-3 text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-blue-500 ${model.selectedId === task.id ? 'border-blue-500 bg-blue-50 dark:bg-blue-500/10' : 'border-slate-200 hover:bg-slate-50 dark:border-slate-700 dark:hover:bg-slate-900'}`}>
          <span className="flex flex-wrap items-center justify-between gap-2"><span className="break-all text-sm font-semibold">{task.operation}</span>{badge(task.status)}</span>
          <span className="mt-2 block break-all font-mono text-xs text-slate-600 dark:text-slate-400">{task.id}</span>
          <span className="mt-1 block text-xs text-slate-600 dark:text-slate-400">{copy.updated}: {date(task.updated_at)}</span>
        </button>)}
      </div>
      <div className="min-w-0 space-y-4 rounded-lg border border-slate-200 p-4 dark:border-slate-700" data-testid="task-detail">
        {model.detailError && <p role="alert" className="break-words text-sm text-red-700 dark:text-red-300">{model.detailError}</p>}
        {!detail && !model.detailError && <p className="text-sm">{model.selectedId ? copy.loading : copy.select}</p>}
        {detail && <>
          <div className="flex flex-wrap items-center justify-between gap-2"><h3 className="break-all font-semibold">{detail.operation}</h3>{badge(detail.status)}</div>
          <dl className="space-y-2 text-xs">
            <div><dt className="text-slate-600 dark:text-slate-400">{copy.id}</dt><dd className="select-text break-all font-mono">{detail.id}</dd></div>
            <div><dt className="text-slate-600 dark:text-slate-400">{copy.created}</dt><dd>{date(detail.created_at)}</dd></div>
            <div><dt className="text-slate-600 dark:text-slate-400">{copy.updated}</dt><dd>{date(detail.updated_at)}</dd></div>
          </dl>
          <p className="text-xs leading-5 text-slate-600 dark:text-slate-400">{copy.cancelHint}</p>
          <div className="flex flex-wrap gap-2">
            <Button disabled={detail.status !== 'queued' || !!model.cancelling || !!model.listError}
              onClick={() => void model.cancel(detail.id)}>{model.cancelling === detail.id ? copy.cancelling : copy.cancel}</Button>
            {resourceTab && <Button onClick={() => { setActiveTab(resourceTab); onClose() }}>{copy.resource}</Button>}
          </div>
          {feedback && <p role={feedback.error ? 'alert' : 'status'} className="break-words text-sm">
            {feedback.code === 'TASK_RUNNING' ? copy.runningRace : feedback.error
              ? `${feedback.code}: ${feedback.error}` : feedback.status === 'cancelled' ? copy.cancelled : copy.alreadyFinished}
          </p>}
          {detail.status === 'interrupted' && <p className="text-sm text-amber-800 dark:text-amber-200">{copy.interrupted}</p>}
          {detail.error && <section className="space-y-2 break-words text-sm" aria-label={copy.error}>
            <h4 className="font-semibold">{copy.error}</h4>
            <p className="font-mono">{detail.error.code}</p><p>{detail.error.message}</p>
            <p>{copy.retryable}: {detail.error.retryable ? copy.yes : copy.no}</p>
          </section>}
          <section aria-label={copy.result} className="min-w-0 space-y-2">
            <h4 className="text-sm font-semibold">{copy.result}</h4>
            {detail.result && typeof detail.result === 'object' && 'result_omitted' in detail.result && detail.result.result_omitted === true
              ? <p className="text-sm">{copy.omitted}</p>
              : detail.result !== null && detail.result !== undefined
                ? <pre tabIndex={0} className="max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-md bg-slate-100 p-3 text-xs dark:bg-slate-900">{JSON.stringify(detail.result, null, 2)}</pre>
                : <p className="text-xs text-slate-600 dark:text-slate-400">{copy.noResult}</p>}
          </section>
        </>}
      </div>
    </div>
  </section>
}
