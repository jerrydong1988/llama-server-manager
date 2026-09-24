import { useEffect, useMemo, useRef, type KeyboardEvent } from 'react'
import { ChevronRight, Copy, Play, X } from 'lucide-react'
import { useI18n } from '../../i18n'
import { exportShellCommand, type ExportShell } from '../../store/commandFormatting'
import { Button, surfaceClassName } from '../ui'
import { buildCommandPreview } from './commandPreview'

export function CommandPreviewModal({ command, shell, copied, onCopy, onStart, onClose }: {
  command: string[]
  shell: ExportShell
  copied: boolean
  onCopy: () => void
  onStart: () => void
  onClose: () => void
}) {
  const { t } = useI18n()
  const labels = t.commandPreview
  const preview = useMemo(() => buildCommandPreview(command), [command])
  const dialog = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null
    dialog.current?.querySelector<HTMLButtonElement>('button')?.focus()
    return () => { if (previous?.isConnected) previous.focus() }
  }, [])

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') { event.stopPropagation(); onClose() }
    if (event.key !== 'Tab') return
    const focusable = Array.from(dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled), summary') ?? [])
      .filter(element => element.getClientRects().length > 0)
    const first = focusable[0], last = focusable[focusable.length - 1]
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus() }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus() }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <div ref={dialog} role="dialog" aria-modal="true" aria-labelledby="command-preview-title" onKeyDown={handleKeyDown}
        className={`${surfaceClassName} flex max-h-[calc(100dvh-2rem)] w-full min-w-0 max-w-4xl flex-col overflow-hidden`}>
        <header className="flex shrink-0 items-center justify-between gap-3 border-b border-slate-200 px-5 py-4 dark:border-slate-800">
          <div className="min-w-0">
            <h3 id="command-preview-title" className="text-lg font-semibold">{t.instance.genCommandTitle}</h3>
            <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">{labels.description}</p>
          </div>
          <Button onClick={onClose} variant="subtle" size="icon" className="shrink-0" aria-label={t.appShell.close}><X className="h-5 w-5" /></Button>
        </header>
        <div data-command-preview-content className="min-h-0 space-y-4 overflow-y-auto px-5 py-4">
          <div>
            <p className="mb-1.5 text-xs font-medium text-slate-500 dark:text-slate-400">{labels.executable}</p>
            <code className="block whitespace-pre-wrap text-sm [overflow-wrap:anywhere]">{preview.executable}</code>
          </div>
          {preview.groups.map(group => (
            <section key={group.id} aria-labelledby={`command-group-${group.id}`} className="overflow-hidden rounded-lg border border-slate-200 dark:border-slate-800">
              <h4 id={`command-group-${group.id}`} className="bg-slate-50 px-3 py-2 text-sm font-semibold dark:bg-slate-800/60">{labels.groups[group.id]}</h4>
              <dl className="divide-y divide-slate-100 dark:divide-slate-800">
                {group.rows.map(row => (
                  <div key={row.index} data-command-parameter className="grid min-w-0 gap-1 px-3 py-2.5 sm:grid-cols-[minmax(10rem,0.42fr)_minmax(0,1fr)] sm:gap-4">
                    <dt className="min-w-0 font-mono text-xs font-medium leading-5 text-blue-700 [overflow-wrap:anywhere] dark:text-blue-300">{row.flag ?? labels.arguments}</dt>
                    <dd className="min-w-0 space-y-1 font-mono text-xs leading-5 text-slate-800 dark:text-slate-200">
                      {row.values.length ? row.values.map((value, index) => <span key={index} className="block whitespace-pre-wrap [overflow-wrap:anywhere]">{value === '' ? '""' : value}</span>) : <span className="text-slate-400">—</span>}
                    </dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
          <details className="group rounded-lg border border-slate-200 dark:border-slate-800">
            <summary className="flex cursor-pointer list-none items-center gap-2 rounded-lg px-3 py-3 text-sm font-medium focus-visible:outline focus-visible:outline-2 focus-visible:outline-blue-500 [&::-webkit-details-marker]:hidden">
              <ChevronRight className="h-4 w-4 shrink-0 transition-transform group-open:rotate-90" />{labels.fullCommand}
            </summary>
            <div className="border-t border-slate-200 p-3 dark:border-slate-800">
              <p className="mb-2 text-xs text-slate-500 dark:text-slate-400">{shell === 'powershell' ? 'PowerShell 7.3+' : 'POSIX shell (sh / bash / zsh)'}</p>
              <pre className="whitespace-pre-wrap rounded-lg bg-slate-100 p-3 text-xs leading-6 text-slate-800 [overflow-wrap:anywhere] dark:bg-slate-950 dark:text-slate-200">{exportShellCommand(preview.maskedCommand, shell)}</pre>
            </div>
          </details>
        </div>
        <footer className="shrink-0 space-y-3 border-t border-slate-200 px-5 py-4 dark:border-slate-800">
          {preview.hasSecrets && <p className="text-xs leading-5 text-amber-700 dark:text-amber-300">{t.instance.commandSecretWarning}</p>}
          <div className="flex flex-wrap items-center gap-2">
            <Button onClick={onCopy} variant="primary" icon={<Copy className="h-4 w-4" />}>{labels.copyCommand}</Button>
            <Button onClick={onStart} variant="success" icon={<Play className="h-4 w-4" />}>{t.instance.directStart}</Button>
            {copied && <span role="status" className="text-xs font-medium text-emerald-600 dark:text-emerald-400">{t.common.copySuccess}</span>}
          </div>
        </footer>
      </div>
    </div>
  )
}
