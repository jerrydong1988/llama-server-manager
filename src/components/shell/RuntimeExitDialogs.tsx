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
        <div className="console-overlay fixed inset-0 z-[120] flex items-center justify-center bg-ui-scrim px-4 backdrop-blur-sm" role="presentation">
          <div className="w-full max-w-lg rounded-xl border border-ui-line bg-ui-surface p-6 shadow-2xl" role="dialog" aria-modal="true" aria-labelledby="proxy-exit-title">
            <div className="flex items-start justify-between gap-4">
              <div>
                <h2 id="proxy-exit-title" className="text-xl font-semibold text-ui-ink">
                  {copy.proxyRunning}
                </h2>
                <p className="mt-3 text-sm leading-6 text-ui-muted">
                  {copy.proxyExitDescription}
                </p>
              </div>
              <button
                type="button"
                onClick={onCloseConfirmation}
                disabled={busy}
                className="rounded-lg border border-ui-line p-2 text-ui-muted transition hover:bg-ui-hover hover:text-ui-ink disabled:cursor-not-allowed disabled:opacity-50"
                aria-label={copy.close}
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            {confirmationError ? (
              <div className="mt-4 rounded-lg border border-ui-line bg-ui-danger-bg px-3 py-2 text-sm text-ui-danger" role="alert">
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
        <div className="console-overlay fixed inset-0 z-[130] flex items-center justify-center bg-ui-scrim px-4 backdrop-blur-sm" role="presentation">
          <div className="w-full max-w-lg rounded-xl border border-ui-line bg-ui-surface p-6 shadow-2xl" role="alertdialog" aria-modal="true" aria-labelledby="background-detach-error-title">
            <div className="flex items-start justify-between gap-4">
              <div>
                <h2 id="background-detach-error-title" className="text-xl font-semibold text-ui-ink">
                  {copy.backgroundDetachFailed}
                </h2>
                <p className="mt-3 text-sm leading-6 text-ui-muted">
                  {copy.backgroundDetachFailureDescription}
                </p>
              </div>
              <button
                type="button"
                onClick={onCloseDetachError}
                disabled={busy}
                className="rounded-lg border border-ui-line p-2 text-ui-muted transition hover:bg-ui-hover hover:text-ui-ink disabled:cursor-not-allowed disabled:opacity-50"
                aria-label={copy.close}
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="mt-4 rounded-lg border border-ui-line bg-ui-danger-bg px-3 py-2 text-xs leading-5 text-ui-danger">
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
