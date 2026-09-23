import { useEffect, useState } from 'react'
import { createPortal } from 'react-dom'
import { ArrowUp, CheckCircle2, LoaderCircle, Sparkles } from 'lucide-react'
import { Button, IconButton, joinClassNames } from '../ui'
import { useWorkspaceActions } from '../shell/WorkspaceActions'

export function ConfigFloatingActions({
  topTargetId,
  saveLabel,
  floatingSaveLabel,
  savingLabel,
  savedLabel,
  backToTopLabel,
  saving,
  saved,
  disabled,
  hasChanges,
  error,
  onSave,
}: {
  topTargetId: string
  saveLabel: string
  floatingSaveLabel: string
  savingLabel: string
  savedLabel: string
  backToTopLabel: string
  saving: boolean
  saved: boolean
  disabled: boolean
  hasChanges: boolean
  error: string | null
  onSave: () => void | Promise<void>
}) {
  const [visible, setVisible] = useState(false)
  const { floatingTarget, guideTarget } = useWorkspaceActions()
  const target = guideTarget ?? floatingTarget

  useEffect(() => {
    const topTarget = document.getElementById(topTargetId)
    if (!topTarget) return

    const observer = new IntersectionObserver(([entry]) => {
      setVisible(!entry.isIntersecting)
    })
    observer.observe(topTarget)
    return () => observer.disconnect()
  }, [topTargetId])

  if (!target || (!guideTarget && !visible)) return null

  const scrollToTop = () => {
    document.getElementById(topTargetId)?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  return createPortal(
    <div
      data-config-floating-actions={guideTarget ? undefined : ''}
      data-config-guide-actions={guideTarget ? '' : undefined}
      className={joinClassNames('pointer-events-auto flex flex-wrap items-center gap-2', !guideTarget && 'rounded-xl border border-slate-200 bg-white/95 p-2 shadow-xl shadow-slate-950/15 backdrop-blur dark:border-slate-700 dark:bg-slate-900/95 dark:shadow-slate-950/50')}
    >
      <Button
        onClick={() => { void onSave() }}
        disabled={disabled}
        variant={!guideTarget || hasChanges ? 'primary' : 'secondary'}
        size="sm"
        aria-label={floatingSaveLabel}
        data-config-floating-save
        data-guide="config-actions"
        icon={saving ? <LoaderCircle className="h-4 w-4 animate-spin" /> : saved ? <CheckCircle2 className="h-4 w-4" /> : <Sparkles className="h-4 w-4" />}
        className="h-9 px-3 shadow-sm"
      >
        {saving ? savingLabel : saved ? savedLabel : saveLabel}
      </Button>
      {visible && <IconButton
        label={backToTopLabel}
        onClick={scrollToTop}
        data-config-back-to-top
        icon={<ArrowUp className="h-4 w-4" />}
        className="h-9 w-9 border-blue-300 text-blue-700 hover:border-blue-400 hover:bg-blue-50 dark:border-blue-500/40 dark:text-blue-200 dark:hover:border-blue-400 dark:hover:bg-blue-500/10"
      />}
      {error && <p role="alert" className="max-w-xs basis-full text-xs leading-5 text-red-700 dark:text-red-300">{error}</p>}
    </div>, target
  )
}
