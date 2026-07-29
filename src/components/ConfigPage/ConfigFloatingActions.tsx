import { useEffect, useState } from 'react'
import { ArrowUp, CheckCircle2, LoaderCircle, Sparkles } from 'lucide-react'
import { Button, IconButton } from '../ui'

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
  onSave: () => void | Promise<void>
}) {
  const [visible, setVisible] = useState(false)

  useEffect(() => {
    const topTarget = document.getElementById(topTargetId)
    if (!topTarget) return

    const observer = new IntersectionObserver(([entry]) => {
      setVisible(!entry.isIntersecting)
    })
    observer.observe(topTarget)
    return () => observer.disconnect()
  }, [topTargetId])

  if (!visible) return null

  const scrollToTop = () => {
    document.getElementById(topTargetId)?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  return (
    <div
      data-config-floating-actions
      className="fixed bottom-4 right-4 z-30 flex items-center gap-2 rounded-xl border border-slate-200 bg-white/95 p-2 shadow-xl shadow-slate-950/15 backdrop-blur dark:border-slate-700 dark:bg-slate-900/95 dark:shadow-slate-950/50 sm:bottom-14 sm:right-6"
    >
      <Button
        onClick={() => { void onSave() }}
        disabled={disabled}
        variant="primary"
        size="sm"
        aria-label={floatingSaveLabel}
        data-config-floating-save
        icon={saving ? <LoaderCircle className="h-4 w-4 animate-spin" /> : saved ? <CheckCircle2 className="h-4 w-4" /> : <Sparkles className="h-4 w-4" />}
        className="h-9 px-3 shadow-sm"
      >
        {saving ? savingLabel : saved ? savedLabel : saveLabel}
      </Button>
      <IconButton
        label={backToTopLabel}
        onClick={scrollToTop}
        data-config-back-to-top
        icon={<ArrowUp className="h-4 w-4" />}
        className="h-9 w-9 border-blue-300 text-blue-700 hover:border-blue-400 hover:bg-blue-50 dark:border-blue-500/40 dark:text-blue-200 dark:hover:border-blue-400 dark:hover:bg-blue-500/10"
      />
    </div>
  )
}
