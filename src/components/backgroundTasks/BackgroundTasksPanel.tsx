import { useRef, useState } from 'react'
import { RefreshCw, Trash2 } from 'lucide-react'
import { confirm } from '@tauri-apps/plugin-dialog'
import { useI18n } from '../../i18n'
import { getBackgroundTaskLabels } from '../../i18n/backgroundTasks'
import { useAppStore } from '../../store'
import { Badge, Button, SelectInput, TextInput } from '../ui'
import { useManagementTasks } from './useManagementTasks'
import { isFinishedTask, type TaskStatus } from './types'

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
  const confirmationLock = useRef(false)
  const [confirming, setConfirming] = useState(false)
  const normalized = query.trim().toLowerCase()
  const rows = model.tasks.filter(task => (!status || task.status === status)
    && (!normalized || `${task.id} ${task.operation}`.toLowerCase().includes(normalized)))
  const detail = model.detail
  const feedback = model.feedback?.id === model.selectedId ? model.feedback : null
  const resourceTab = detail && resourceTabs[detail.operation.split('.')[0]]
  const date = (value: number) => new Date(value).toLocaleString(lang)
  const badge = (state: TaskStatus) => <Badge tone={tones[state] ?? 'slate'}>{copy.states[state] ?? state}</Badge>
  const busy = confirming || !!model.cancelling || !!model.removing
  const removeHistory = async (id?: string) => {
    if (confirmationLock.current || busy) return
    confirmationLock.current = true
    setConfirming(true)
    try {
      const accepted = await confirm(id ? `${copy.deleteConfirm}\n\n${id}` : copy.clearConfirm,
        { title: id ? copy.delete : copy.clear, kind: 'warning', okLabel: copy.confirmDelete, cancelLabel: copy.cancelDelete })
      if (accepted) await model.removeHistory(id)
    } finally { confirmationLock.current = false; setConfirming(false) }
  }

  return <section aria-label={copy.title} className="space-y-4 p-5" data-testid="background-tasks">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <p className="max-w-2xl text-sm text-ui-muted">{copy.description}</p>
      <div className="flex flex-wrap gap-2"><Button variant="danger" disabled={busy || !!model.listError || !model.tasks.some(task => isFinishedTask(task.status))}
        onClick={() => void removeHistory()}><Trash2 aria-hidden="true" className="h-4 w-4" />{model.removing === '*' ? copy.removing : copy.clear}</Button>
      <Button onClick={model.refresh} disabled={model.loading}>
        <RefreshCw aria-hidden="true" className={`h-4 w-4 ${model.loading ? 'animate-spin motion-reduce:animate-none' : ''}`} />{copy.refresh}
      </Button></div>
    </div>
    <p className="text-xs leading-5 text-ui-muted">{copy.scope}</p>
    <div className="flex flex-col gap-3 sm:flex-row">
      <TextInput aria-label={copy.search} placeholder={copy.search} value={query} onChange={event => setQuery(event.target.value)} />
      <SelectInput aria-label={copy.filter} value={status} onChange={event => setStatus(event.target.value)}>
        <option value="">{copy.all}</option>
        {Object.entries(copy.states).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
      </SelectInput>
    </div>
    {model.listError && <p role="alert" className="break-words text-sm text-ui-danger">{copy.stale} {model.listError}</p>}
    {model.historyFeedback && <p role={model.historyFeedback.error && model.historyFeedback.code !== 'NOT_FOUND' ? 'alert' : 'status'} className="break-words text-sm">
      {model.historyFeedback.code === 'TASK_ACTIVE' ? copy.activeProtected : model.historyFeedback.code === 'NOT_FOUND' ? copy.alreadyDeleted
        : model.historyFeedback.error ? `${model.historyFeedback.code}: ${model.historyFeedback.error}` : `${copy.deleted} ${model.historyFeedback.count}`}
    </p>}
    <div className="text-xs text-ui-muted">{rows.length} / {model.tasks.length} {copy.records}</div>
    <div className="grid min-w-0 gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <div className="max-h-[48vh] min-w-0 space-y-2 overflow-y-auto p-1" aria-busy={model.loading}>
        {!rows.length && <p role="status" className="p-4 text-sm text-ui-muted">{model.loading ? copy.loading : model.tasks.length ? copy.noMatches : copy.empty}</p>}
        {rows.map(task => <button type="button" key={task.id} data-testid={`task-${task.id}`} aria-pressed={model.selectedId === task.id} data-selected={model.selectedId === task.id}
          onClick={() => model.selectTask(task.id)}
          className="console-list-row w-full rounded-lg border border-ui-line p-3 text-left">
          <span className="flex flex-wrap items-center justify-between gap-2"><span className="break-all text-sm font-semibold">{task.operation}</span>{badge(task.status)}</span>
          <span className="mt-2 block break-all font-mono text-xs text-ui-muted">{task.id}</span>
          <span className="mt-1 block text-xs text-ui-muted">{copy.updated}: {date(task.updated_at)}</span>
        </button>)}
      </div>
      <div className="min-w-0 space-y-4 rounded-lg border border-ui-line p-4" data-testid="task-detail">
        {model.detailError && <p role="alert" className="break-words text-sm text-ui-danger">{model.detailError}</p>}
        {!detail && !model.detailError && <p className="text-sm">{model.selectedId ? copy.loading : copy.select}</p>}
        {detail && <>
          <div className="flex flex-wrap items-center justify-between gap-2"><h3 className="break-all font-semibold">{detail.operation}</h3>{badge(detail.status)}</div>
          <dl className="space-y-2 text-xs">
            <div><dt className="text-ui-muted">{copy.id}</dt><dd className="select-text break-all font-mono">{detail.id}</dd></div>
            <div><dt className="text-ui-muted">{copy.created}</dt><dd>{date(detail.created_at)}</dd></div>
            <div><dt className="text-ui-muted">{copy.updated}</dt><dd>{date(detail.updated_at)}</dd></div>
          </dl>
          <p className="text-xs leading-5 text-ui-muted">{copy.cancelHint}</p>
          <p className="text-xs leading-5 text-ui-muted">{copy.cleanupHint}</p>
          <div className="flex flex-wrap gap-2">
            <Button disabled={detail.status !== 'queued' || busy || !!model.listError}
              onClick={() => void model.cancel(detail.id)}>{model.cancelling === detail.id ? copy.cancelling : copy.cancel}</Button>
            <Button variant="danger" disabled={!isFinishedTask(detail.status) || busy || !!model.listError}
              onClick={() => void removeHistory(detail.id)}>{model.removing === detail.id ? copy.removing : copy.delete}</Button>
            {resourceTab && <Button onClick={() => { setActiveTab(resourceTab); onClose() }}>{copy.resource}</Button>}
          </div>
          {feedback && <p role={feedback.error ? 'alert' : 'status'} className="break-words text-sm">
            {feedback.code === 'TASK_RUNNING' ? copy.runningRace : feedback.error
              ? `${feedback.code}: ${feedback.error}` : feedback.status === 'cancelled' ? copy.cancelled : copy.alreadyFinished}
          </p>}
          {detail.status === 'interrupted' && <p className="text-sm text-ui-warn">{copy.interrupted}</p>}
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
                ? <pre tabIndex={0} className="max-h-64 overflow-auto whitespace-pre-wrap break-all rounded-md bg-ui-inset p-3 text-xs">{JSON.stringify(detail.result, null, 2)}</pre>
                : <p className="text-xs text-ui-muted">{copy.noResult}</p>}
          </section>
        </>}
      </div>
    </div>
  </section>
}
