import { useEffect, useRef, useState } from 'react'
import { CheckCircle2, ChevronDown, ChevronUp, Compass, X } from 'lucide-react'
import { useAppStore } from '../../store'
import { formatMessage, useI18n } from '../../i18n'
import { getGuideTourCopy } from '../../i18n/guideTourCopy'
import { Button } from '../ui'
import { WorkspaceGuideActionsHost } from '../shell/WorkspaceActions'
import { GuideInstancePicker } from './GuideInstancePicker'
import { getGuideTourSteps } from './guideTour'
import { guideStepReady, useGuideTourStore } from './guideTourStore'
import { useGuideTarget } from './useGuideTarget'

export function GuideTourPanel() {
  const session = useGuideTourStore(state => state.session)
  return session ? <ActiveGuideTour key={session.id} /> : null
}

function ActiveGuideTour() {
  const { lang } = useI18n()
  const copy = getGuideTourCopy(lang)
  const { session, configPending, move, close, finish, returnToStep } = useGuideTourStore()
  const instances = useAppStore(state => state.instances)
  const activeTab = useAppStore(state => state.activeTab)
  // Re-evaluate readiness when resources or lifecycle state change, without
  // subscribing the coach to high-frequency system/telemetry updates.
  useAppStore(state => state.models)
  useAppStore(state => state.engines)
  useAppStore(state => state.instanceLifecycle)
  const [collapsed, setCollapsed] = useState(false)
  const [attempt, setAttempt] = useState(0)
  const panelRef = useRef<HTMLElement>(null)
  useEffect(() => { panelRef.current?.focus({ preventScroll: true }) }, [])
  const steps = getGuideTourSteps(lang, session?.mode)
  const step = steps[session?.index ?? 0]
  const instance = instances.find(item => item.id === session?.instanceId)
  const needsInstance = session?.mode === 'setup' && ['config', 'start', 'verify'].includes(step.id) && !instance
  const away = activeTab !== step.tab || Boolean(needsInstance)
  const targetStatus = useGuideTarget(step.selector, !away && !session?.complete,
    `${session?.index}:${session?.instanceId}:${attempt}`)
  const ready = session ? guideStepReady(session) : false

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.isComposing || event.defaultPrevented) return
      event.preventDefault()
      close(false, Boolean(document.querySelector('[aria-modal="true"]')))
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [close])

  if (!session) return null
  const progress = formatMessage(copy.progress, { current: session.index + 1, total: steps.length })
  const pending = configPending ? copy.needSave : step.id === 'models' ? copy.needModel : step.id === 'engines' ? copy.needEngine
    : step.id === 'start' ? copy.needStart : step.id === 'verify' ? copy.needVerify : copy.noInstance
  return (
    <section ref={panelRef} tabIndex={-1} aria-label={copy.title} data-guide-panel className="max-h-[42vh] shrink-0 overflow-y-auto border-t border-blue-300 bg-white text-slate-900 outline-none shadow-[0_-4px_16px_rgba(15,23,42,0.06)] dark:border-blue-800 dark:bg-slate-900 dark:text-slate-100">
      <div className="mx-auto w-full max-w-[1480px] px-4 py-2 sm:px-5">
        <div className="flex min-w-0 items-center justify-between gap-3">
          <div className="flex min-w-0 items-center gap-2">
            <Compass className="h-4 w-4 shrink-0 text-blue-600 dark:text-blue-400" />
            <span className="truncate text-xs font-semibold">{session.mode === 'setup' ? copy.setup : copy.advanced}</span>
            <span className="shrink-0 text-xs text-slate-500 dark:text-slate-400" data-guide-progress>{progress}</span>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            {!session.complete && <Button size="sm" variant="subtle" onClick={() => close(true)}>{copy.pause}</Button>}
            {!session.complete && <Button size="icon" variant="subtle" aria-label={collapsed ? copy.expand : copy.collapse} aria-expanded={!collapsed} onClick={() => setCollapsed(value => !value)}>
              {collapsed ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
            </Button>}
            <Button size="icon" variant="subtle" aria-label={copy.close} onClick={() => close()}><X className="h-4 w-4" /></Button>
          </div>
        </div>
        {session.complete ? (
          <div className="mt-2 flex flex-wrap items-center justify-between gap-3">
            <div className="min-w-0 flex-1" role="status">
              <h2 className="flex items-center gap-2 font-semibold"><CheckCircle2 className="h-5 w-5 text-emerald-600" />{session.mode === 'setup' ? copy.success : copy.explored}</h2>
              <p className="mt-1 text-sm text-slate-600 dark:text-slate-300">{session.mode === 'setup' ? copy.successDetail : copy.exploredDetail}</p>
            </div>
            <Button onClick={() => finish()}>{copy.returnGuide}</Button>
            <Button variant="primary" onClick={() => finish(true)}>{copy.stay}</Button>
          </div>
        ) : (
          <div className="mt-1 grid min-w-0 gap-x-5 gap-y-2 xl:grid-cols-[minmax(0,1fr)_minmax(0,auto)] xl:items-end">
            <div className="min-w-0">
              <h2 className="text-sm font-semibold" data-guide-step-title>{step.title}</h2>
              {!collapsed && <p className="mt-1 text-sm leading-5 text-slate-600 dark:text-slate-300">{step.description}</p>}
              <p role="status" className={`mt-1 text-xs leading-5 ${ready ? 'text-emerald-700 dark:text-emerald-300' : 'text-slate-500 dark:text-slate-400'}`}>
                {configPending ? copy.needSave : away ? (needsInstance ? copy.noInstance : copy.away) : targetStatus === 'missing' ? copy.missing : targetStatus === 'loading' ? copy.loading : ready ? copy.ready : pending}
              </p>
              <div className="mt-1 flex flex-wrap gap-2">
                {away && <Button size="sm" disabled={configPending} onClick={returnToStep}>{copy.returnToStep}</Button>}
                {!away && targetStatus === 'missing' && <Button size="sm" onClick={() => setAttempt(value => value + 1)}>{copy.retry}</Button>}
                {step.id === 'models' && <Button size="sm" variant="subtle" disabled={configPending} onClick={() => useAppStore.getState().setActiveTab('downloads')}>{copy.downloads}</Button>}
                {['start', 'verify'].includes(step.id) && <Button size="sm" variant="subtle" disabled={configPending} onClick={() => useAppStore.getState().setActiveTab('logs')}>{copy.logs}</Button>}
              </div>
            </div>
            <div className="flex min-w-0 flex-col gap-1 xl:max-w-lg">
              {session.mode === 'setup' && session.index >= 2 && <GuideInstancePicker key={session.index} alwaysExpanded={step.id === 'instances'} />}
              <div className="flex flex-wrap items-center justify-end gap-2" data-guide-navigation>
                {step.id === 'config' && !away && <WorkspaceGuideActionsHost />}
                <Button size="sm" disabled={session.index === 0 || configPending} onClick={() => move(-1)}>{copy.previous}</Button>
                <Button size="sm" variant={configPending ? 'secondary' : 'primary'} disabled={!ready || away || targetStatus !== 'ready'} onClick={() => move(1)}>
                  {session.index === steps.length - 1 ? copy.done : step.id === 'config' ? copy.review : copy.next}
                </Button>
              </div>
            </div>
          </div>
        )}
      </div>
    </section>
  )
}
