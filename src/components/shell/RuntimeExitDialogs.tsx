import { X } from 'lucide-react'
import { Button } from '../ui'

type RuntimeExitCopy = {
  proxyRunning: string
  proxyExitDescription: string
  close: string
  backgroundDetachFailed: string
  keepInTray: string
  openProxySettings: string
  stopProxyAndQuit: string
  backgroundDetachPreparing: string
  enableBackgroundAndExit: string
  backgroundDetachFailureDescription: string
  cancelExit: string
  retryBackgroundDetach: string
}

type RuntimeExitDialogsProps = {
  confirmationOpen: boolean
  confirmationError: string
  detachError: string
  busy: boolean
  copy: RuntimeExitCopy
  onCloseConfirmation: () => void
  onKeepInTray: () => void
  onOpenSettingsFromConfirmation: () => void
  onStopAndQuit: () => void
  onEnableBackgroundAndQuit: () => void
  onCloseDetachError: () => void
  onOpenSettingsFromDetachError: () => void
  onRetryDetach: () => void
}

export function RuntimeExitDialogs({
  confirmationOpen,
  confirmationError,
  detachError,
  busy,
  copy,
  onCloseConfirmation,
  onKeepInTray,
  onOpenSettingsFromConfirmation,
  onStopAndQuit,
  onEnableBackgroundAndQuit,
  onCloseDetachError,
  onOpenSettingsFromDetachError,
  onRetryDetach,
}: RuntimeExitDialogsProps) {
  return (
    <>
      {confirmationOpen ? (
        <div className="fixed inset-0 z-[120] flex items-center justify-center bg-slate-950/60 px-4 backdrop-blur-sm" role="presentation">
          <div className="w-full max-w-lg rounded-xl border border-slate-200 bg-white p-6 shadow-2xl dark:border-slate-800 dark:bg-slate-950" role="dialog" aria-modal="true" aria-labelledby="proxy-exit-title">
            <div className="flex items-start justify-between gap-4">
              <div>
                <h2 id="proxy-exit-title" className="text-xl font-semibold text-slate-950 dark:text-slate-50">
                  {copy.proxyRunning}
                </h2>
                <p className="mt-3 text-sm leading-6 text-slate-600 dark:text-slate-300">
                  {copy.proxyExitDescription}
                </p>
              </div>
              <button
                type="button"
                onClick={onCloseConfirmation}
                disabled={busy}
                className="rounded-lg border border-slate-200 p-2 text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 disabled:cursor-not-allowed disabled:opacity-50 dark:border-slate-800 dark:text-slate-400 dark:hover:bg-slate-900 dark:hover:text-slate-100"
                aria-label={copy.close}
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            {confirmationError ? (
              <div className="mt-4 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-sm text-rose-700 dark:border-rose-900/60 dark:bg-rose-950/30 dark:text-rose-300" role="alert">
                <div className="font-semibold">{copy.backgroundDetachFailed}</div>
                <div className="mt-1 break-words text-xs leading-5">{confirmationError}</div>
              </div>
            ) : null}
            <div className="mt-6 flex flex-col-reverse gap-3 sm:flex-row sm:flex-wrap sm:justify-end">
              <Button disabled={busy} onClick={onKeepInTray}>{copy.keepInTray}</Button>
              <Button disabled={busy} onClick={onOpenSettingsFromConfirmation}>{copy.openProxySettings}</Button>
              <Button disabled={busy} variant="danger" onClick={onStopAndQuit}>{copy.stopProxyAndQuit}</Button>
              <Button disabled={busy} variant="primary" onClick={onEnableBackgroundAndQuit}>
                {busy ? copy.backgroundDetachPreparing : copy.enableBackgroundAndExit}
              </Button>
            </div>
          </div>
        </div>
      ) : null}
      {detachError ? (
        <div className="fixed inset-0 z-[130] flex items-center justify-center bg-slate-950/60 px-4 backdrop-blur-sm" role="presentation">
          <div className="w-full max-w-lg rounded-xl border border-slate-200 bg-white p-6 shadow-2xl dark:border-slate-800 dark:bg-slate-950" role="alertdialog" aria-modal="true" aria-labelledby="background-detach-error-title">
            <div className="flex items-start justify-between gap-4">
              <div>
                <h2 id="background-detach-error-title" className="text-xl font-semibold text-slate-950 dark:text-slate-50">
                  {copy.backgroundDetachFailed}
                </h2>
                <p className="mt-3 text-sm leading-6 text-slate-600 dark:text-slate-300">
                  {copy.backgroundDetachFailureDescription}
                </p>
              </div>
              <button
                type="button"
                onClick={onCloseDetachError}
                disabled={busy}
                className="rounded-lg border border-slate-200 p-2 text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 disabled:cursor-not-allowed disabled:opacity-50 dark:border-slate-800 dark:text-slate-400 dark:hover:bg-slate-900 dark:hover:text-slate-100"
                aria-label={copy.close}
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="mt-4 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs leading-5 text-rose-700 dark:border-rose-900/60 dark:bg-rose-950/30 dark:text-rose-300">
              {detachError}
            </div>
            <div className="mt-6 flex flex-col-reverse gap-3 sm:flex-row sm:justify-end">
              <Button disabled={busy} onClick={onCloseDetachError}>{copy.cancelExit}</Button>
              <Button disabled={busy} onClick={onOpenSettingsFromDetachError}>{copy.openProxySettings}</Button>
              <Button disabled={busy} variant="primary" onClick={onRetryDetach}>
                {busy ? copy.backgroundDetachPreparing : copy.retryBackgroundDetach}
              </Button>
            </div>
          </div>
        </div>
      ) : null}
    </>
  )
}
