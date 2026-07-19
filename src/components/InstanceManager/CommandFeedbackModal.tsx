import { AlertTriangle, X } from 'lucide-react'
import { Button, Surface } from '../ui'

export function MissingEngineBanner({ message, actionLabel, onAction }: { message: string; actionLabel: string; onAction: () => void }) {
  return (
    <div className="flex flex-col gap-3 rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700 dark:border-red-500/20 dark:bg-red-500/10 dark:text-red-200 sm:flex-row sm:items-center sm:justify-between" role="alert">
      <span className="flex min-w-0 items-start gap-2"><AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" /><span>{message}</span></span>
      <Button onClick={onAction} variant="danger" size="sm" className="shrink-0">{actionLabel}</Button>
    </div>
  )
}

export function CommandFeedbackModal({
  title,
  message,
  closeLabel,
  recoveryLabel,
  onClose,
  onRecover,
}: {
  title: string
  message: string
  closeLabel: string
  recoveryLabel?: string
  onClose: () => void
  onRecover?: () => void
}) {
  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 p-4" role="alertdialog" aria-modal="true" aria-labelledby="command-feedback-title">
      <Surface className="w-full max-w-lg overflow-hidden">
        <div className="flex items-center justify-between border-b border-red-200 bg-red-50 px-5 py-4 dark:border-red-500/20 dark:bg-red-500/10">
          <div className="flex items-center gap-2 text-red-700 dark:text-red-200">
            <AlertTriangle className="h-5 w-5 shrink-0" />
            <h3 id="command-feedback-title" className="text-base font-semibold">{title}</h3>
          </div>
          <Button onClick={onClose} variant="subtle" size="icon" aria-label={closeLabel}><X className="h-5 w-5" /></Button>
        </div>
        <p className="px-5 py-5 text-sm leading-6 text-slate-700 dark:text-slate-200">{message}</p>
        <div className="flex justify-end gap-2 border-t border-slate-200 bg-slate-50 px-5 py-4 dark:border-slate-800 dark:bg-slate-950/70">
          <Button onClick={onClose} variant="secondary">{closeLabel}</Button>
          {onRecover && recoveryLabel ? <Button onClick={onRecover} variant="primary">{recoveryLabel}</Button> : null}
        </div>
      </Surface>
    </div>
  )
}
