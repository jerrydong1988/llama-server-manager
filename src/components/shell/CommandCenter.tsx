import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { Search, X } from 'lucide-react'
import { useI18n } from '../../i18n'
import { Badge, Button, TextInput } from '../ui'
import { BackgroundTasksPanel } from '../backgroundTasks/BackgroundTasksPanel'
import { getBackgroundTaskLabels } from '../../i18n/backgroundTasks'

export type ProductIssue = {
  id: string
  title: string
  description: string
  severity: 'info' | 'warning' | 'critical'
  actionLabel: string
  action: () => void
}

export type CommandAction = {
  id: string
  title: string
  description: string
  group: string
  icon: ReactNode
  action: () => void
}

export function CommandCenter({
  open,
  issues,
  commands,
  onClose,
}: {
  open: boolean
  issues: ProductIssue[]
  commands: CommandAction[]
  onClose: () => void
}) {
  const { t, lang } = useI18n()
  const copy = t.commandCenter
  const taskCopy = getBackgroundTaskLabels(lang)
  const [query, setQuery] = useState('')
  const [view, setView] = useState<'shortcuts' | 'tasks'>('shortcuts')
  const dialog = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    setQuery('')
    const previousFocus = document.activeElement
    const handler = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
      if (event.key !== 'Tab') return
      const elements = Array.from(dialog.current?.querySelectorAll<HTMLElement>('button:not([disabled]), input, select, [tabindex="0"]') ?? [])
        .filter(element => element.getClientRects().length > 0)
      const first = elements[0], last = elements[elements.length - 1]
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
    }
    window.addEventListener('keydown', handler)
    if (!dialog.current?.contains(document.activeElement)) {
      dialog.current?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus()
    }
    return () => {
      window.removeEventListener('keydown', handler)
      if (previousFocus instanceof HTMLElement) previousFocus.focus()
    }
  }, [open, onClose])

  const filteredCommands = useMemo(() => {
    const normalized = query.trim().toLowerCase()
    if (!normalized) return commands
    return commands.filter(command =>
      command.title.toLowerCase().includes(normalized)
      || command.description.toLowerCase().includes(normalized)
      || command.group.toLowerCase().includes(normalized),
    )
  }, [commands, query])

  if (!open) return null

  const severityTone: Record<ProductIssue['severity'], 'blue' | 'amber' | 'red'> = {
    info: 'blue',
    warning: 'amber',
    critical: 'red',
  }
  const severityLabel: Record<ProductIssue['severity'], string> = {
    info: copy.severityInfo,
    warning: copy.severityWarning,
    critical: copy.severityCritical,
  }

  return (
    <div ref={dialog} tabIndex={-1} className="console-overlay fixed inset-0 z-50 flex items-start justify-center bg-ui-scrim px-4 py-6 sm:py-12 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="command-center-title">
      <div className={`w-full ${view === 'tasks' ? 'max-w-6xl' : 'max-w-4xl'} overflow-hidden rounded-lg border border-ui-line bg-ui-surface text-ui-ink shadow-2xl`}>
        <div className="flex items-start justify-between gap-4 border-b border-ui-line px-5 py-4">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <Search className="h-5 w-5 text-ui-info" />
              <h2 id="command-center-title" className="text-lg font-semibold">{copy.title}</h2>
              {issues.length > 0
                ? <Badge tone="amber">{issues.length} {copy.needAttention}</Badge>
                : <Badge tone="emerald">{copy.healthy}</Badge>}
            </div>
            <p className="mt-1 text-sm text-ui-muted">{copy.description}</p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label={copy.close}
            className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-ui-line text-ui-muted transition hover:bg-ui-hover hover:text-ui-ink"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div role="tablist" aria-label={copy.title} className="flex gap-2 border-b border-ui-line px-5 py-2"
          onKeyDown={event => {
            if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return
            event.preventDefault()
            const next = event.key === 'Home' ? 'shortcuts' : event.key === 'End' ? 'tasks' : view === 'tasks' ? 'shortcuts' : 'tasks'
            setView(next)
            document.getElementById(`command-tab-${next}`)?.focus()
          }}>
          {(['shortcuts', 'tasks'] as const).map(tab => <button type="button" role="tab" key={tab} id={`command-tab-${tab}`}
            aria-selected={view === tab} aria-controls={`command-panel-${tab}`} tabIndex={view === tab ? 0 : -1}
            onClick={() => setView(tab)} className={`min-h-11 rounded-md px-4 text-sm focus-visible:outline focus-visible:outline-2 focus-visible:outline-ui-focus ${view === tab ? 'bg-ui-action font-semibold text-ui-action-ink' : 'text-ui-muted'}`}>
            {tab === 'tasks' ? taskCopy.title : taskCopy.shortcuts}
          </button>)}
        </div>
        {view === 'tasks' ? <div role="tabpanel" id="command-panel-tasks" aria-labelledby="command-tab-tasks" className="max-h-[68vh] overflow-y-auto">
          <BackgroundTasksPanel onClose={onClose} />
        </div> : <div role="tabpanel" id="command-panel-shortcuts" aria-labelledby="command-tab-shortcuts" className="grid max-h-[68vh] overflow-y-auto lg:grid-cols-[minmax(0,1fr)_320px]">
          <div className="min-w-0 space-y-4 p-5">
            <TextInput
              value={query}
              onChange={event => setQuery(event.target.value)}
              placeholder={copy.search}
              leadingIcon={<Search className="h-4 w-4" />}
            />

            <section className="space-y-2">
              <div className="text-xs font-semibold uppercase tracking-wide text-ui-muted">{copy.commonTasks}</div>
              <div className="grid gap-2 md:grid-cols-2">
                {filteredCommands.map(command => (
                  <button
                    key={command.id}
                    type="button"
                    onClick={() => {
                      command.action()
                      onClose()
                    }}
                    className="flex min-w-0 items-start gap-3 rounded-lg border border-ui-line bg-ui-inset px-3 py-3 text-left transition hover:border-ui-control hover:bg-ui-hover"
                  >
                    <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-ui-line bg-ui-surface text-ui-info">
                      {command.icon}
                    </span>
                    <span className="min-w-0">
                      <span className="block truncate text-sm font-semibold text-ui-ink">{command.title}</span>
                      <span className="mt-1 line-clamp-2 block text-xs leading-5 text-ui-muted">{command.description}</span>
                    </span>
                  </button>
                ))}
              </div>
            </section>
          </div>

          <aside className="border-t border-ui-line bg-ui-inset p-5 lg:border-l lg:border-t-0">
            <div className="mb-3 flex items-center justify-between gap-3">
              <div className="text-sm font-semibold">{copy.attention}</div>
              <Badge tone={issues.length > 0 ? 'amber' : 'emerald'}>{issues.length}</Badge>
            </div>
            {issues.length === 0 ? (
              <div className="rounded-lg border border-ui-line bg-ui-surface px-4 py-5 text-sm text-ui-muted">
                {copy.noIssues}
              </div>
            ) : (
              <div className="space-y-2">
                {issues.map(issue => (
                  <div key={issue.id} className="rounded-lg border border-ui-line bg-ui-surface p-3">
                    <div className="mb-2 flex items-center gap-2">
                      <Badge tone={severityTone[issue.severity]}>{severityLabel[issue.severity]}</Badge>
                      <div className="min-w-0 truncate text-sm font-semibold text-ui-ink">{issue.title}</div>
                    </div>
                    <p className="text-xs leading-5 text-ui-muted">{issue.description}</p>
                    <Button
                      onClick={() => {
                        issue.action()
                        onClose()
                      }}
                      size="sm"
                      className="mt-3 w-full"
                    >
                      {issue.actionLabel}
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </aside>
        </div>}
      </div>
    </div>
  )
}
