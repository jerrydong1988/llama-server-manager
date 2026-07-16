import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { Search, X } from 'lucide-react'
import { useI18n } from '../../i18n'
import { Badge, Button, TextInput } from '../ui'

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
  const { t } = useI18n()
  const copy = t.commandCenter
  const [query, setQuery] = useState('')

  useEffect(() => {
    if (!open) return
    setQuery('')
    const handler = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
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
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-slate-950/55 px-4 py-12 backdrop-blur-sm" role="dialog" aria-modal="true">
      <div className="w-full max-w-4xl overflow-hidden rounded-lg border border-slate-200 bg-white text-slate-900 shadow-2xl dark:border-slate-800 dark:bg-slate-950 dark:text-slate-100">
        <div className="flex items-start justify-between gap-4 border-b border-slate-200 px-5 py-4 dark:border-slate-800">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <Search className="h-5 w-5 text-blue-500" />
              <h2 className="text-lg font-semibold">{copy.title}</h2>
              {issues.length > 0
                ? <Badge tone="amber">{issues.length} {copy.needAttention}</Badge>
                : <Badge tone="emerald">{copy.healthy}</Badge>}
            </div>
            <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">{copy.description}</p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label={copy.close}
            className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-slate-200 text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 dark:border-slate-800 dark:hover:bg-slate-900 dark:hover:text-white"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="grid max-h-[74vh] overflow-y-auto lg:grid-cols-[minmax(0,1fr)_320px]">
          <div className="min-w-0 space-y-4 p-5">
            <TextInput
              value={query}
              onChange={event => setQuery(event.target.value)}
              placeholder={copy.search}
              leadingIcon={<Search className="h-4 w-4" />}
              autoFocus
            />

            <section className="space-y-2">
              <div className="text-xs font-semibold uppercase tracking-wide text-slate-500 dark:text-slate-400">{copy.commonTasks}</div>
              <div className="grid gap-2 md:grid-cols-2">
                {filteredCommands.map(command => (
                  <button
                    key={command.id}
                    type="button"
                    onClick={() => {
                      command.action()
                      onClose()
                    }}
                    className="flex min-w-0 items-start gap-3 rounded-lg border border-slate-200 bg-slate-50 px-3 py-3 text-left transition hover:border-blue-300 hover:bg-blue-50 dark:border-slate-800 dark:bg-slate-900/70 dark:hover:border-blue-500/40 dark:hover:bg-blue-500/10"
                  >
                    <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-slate-200 bg-white text-blue-600 dark:border-slate-800 dark:bg-slate-950 dark:text-blue-300">
                      {command.icon}
                    </span>
                    <span className="min-w-0">
                      <span className="block truncate text-sm font-semibold text-slate-900 dark:text-slate-100">{command.title}</span>
                      <span className="mt-1 line-clamp-2 block text-xs leading-5 text-slate-500 dark:text-slate-400">{command.description}</span>
                    </span>
                  </button>
                ))}
              </div>
            </section>
          </div>

          <aside className="border-t border-slate-200 bg-slate-50 p-5 dark:border-slate-800 dark:bg-slate-900/55 lg:border-l lg:border-t-0">
            <div className="mb-3 flex items-center justify-between gap-3">
              <div className="text-sm font-semibold">{copy.attention}</div>
              <Badge tone={issues.length > 0 ? 'amber' : 'emerald'}>{issues.length}</Badge>
            </div>
            {issues.length === 0 ? (
              <div className="rounded-lg border border-slate-200 bg-white px-4 py-5 text-sm text-slate-500 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-400">
                {copy.noIssues}
              </div>
            ) : (
              <div className="space-y-2">
                {issues.map(issue => (
                  <div key={issue.id} className="rounded-lg border border-slate-200 bg-white p-3 dark:border-slate-800 dark:bg-slate-950">
                    <div className="mb-2 flex items-center gap-2">
                      <Badge tone={severityTone[issue.severity]}>{severityLabel[issue.severity]}</Badge>
                      <div className="min-w-0 truncate text-sm font-semibold text-slate-900 dark:text-slate-100">{issue.title}</div>
                    </div>
                    <p className="text-xs leading-5 text-slate-500 dark:text-slate-400">{issue.description}</p>
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
        </div>
      </div>
    </div>
  )
}
