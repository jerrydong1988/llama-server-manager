import { create } from 'zustand'
import { useAppStore } from '../../store'
import type { Instance } from '../../store/types'
import { getGuideTourSteps, type GuideTourMode } from './guideTour'

export type GuideSession = {
  id: number
  mode: GuideTourMode
  index: number
  instanceId: string | null
  originTab: string
  originInstanceId: string | null
  verified: string | null
  connectionAttempt?: number
  complete: boolean
}

let nextSessionId = 0
let nextConnectionAttempt = 0
type ConnectionToken = { sessionId: number; attempt: number }
export const guideReadingPosition = { scrollTop: 0, checklistOpen: false }

export function guideInstanceKey(instance: Instance): string {
  return JSON.stringify([instance.id, instance.config, instance.status, instance.startTime])
}

function selectedInstance(session: GuideSession) {
  return useAppStore.getState().instances.find(instance => instance.id === session.instanceId)
}

export function guideStepReady(session: GuideSession): boolean {
  if (session.mode === 'advanced') return true
  const app = useAppStore.getState()
  const instance = selectedInstance(session)
  switch (getGuideTourSteps('en', session.mode)[session.index]?.id) {
    case 'models': return app.models.some(model => model.file_type === 'model' && !model.capabilities?.is_mmproj)
    case 'engines': return app.engines.length > 0
    case 'instances': return Boolean(instance)
    case 'config': return Boolean(instance) && !useGuideTourStore.getState().configPending
    case 'start': return instance?.status === 'running' && !app.instanceLifecycle[instance.id]
    case 'verify': return Boolean(instance && instance.status === 'running' && session.verified === guideInstanceKey(instance))
    default: return false
  }
}

function navigate(session: GuideSession) {
  const step = getGuideTourSteps('en', session.mode)[session.index]
  const app = useAppStore.getState()
  if (step.tab === 'config') {
    if (!selectedInstance(session)) {
      app.setActiveTab('instances')
      return
    }
    app.setActiveConfigInstanceId(session.instanceId)
  }
  app.setActiveTab(step.tab)
}

function restore(session: GuideSession) {
  const app = useAppStore.getState()
  app.setActiveConfigInstanceId(app.instances.some(instance => instance.id === session.originInstanceId)
    ? session.originInstanceId : null)
  app.setActiveTab(session.originTab)
}

type GuideTourState = {
  session: GuideSession | null
  paused: GuideSession | null
  configPending: boolean
  start: (mode?: GuideTourMode) => void
  move: (direction: -1 | 1) => void
  returnToStep: () => void
  selectInstance: (id: string) => void
  beginConnection: (instanceId: string) => ConnectionToken | undefined
  recordConnection: (token: ConnectionToken | undefined, instance: Instance) => void
  close: (pause?: boolean, keepPage?: boolean) => void
  resume: () => void
  finish: (stay?: boolean) => void
}

export const useGuideTourStore = create<GuideTourState>((set, get) => ({
  session: null,
  paused: null,
  configPending: false,
  start: (mode = 'setup') => {
    if (get().session) return
    const app = useAppStore.getState()
    const session: GuideSession = {
      id: ++nextSessionId, mode, index: 0,
      instanceId: app.instances.some(instance => instance.id === app.activeConfigInstanceId) ? app.activeConfigInstanceId : null,
      originTab: app.activeTab, originInstanceId: app.activeConfigInstanceId,
      verified: null, complete: false,
    }
    set({ session, paused: null })
    navigate(session)
  },
  move: direction => {
    const current = get().session
    if (!current || current.complete || get().configPending || (direction === 1 && !guideStepReady(current))) return
    const index = current.index + direction
    if (index < 0) return
    if (index >= getGuideTourSteps('en', current.mode).length) {
      set({ session: { ...current, complete: true } })
      return
    }
    const session = { ...current, index }
    set({ session })
    navigate(session)
  },
  returnToStep: () => {
    const session = get().session
    if (session && !get().configPending) navigate(session)
  },
  selectInstance: id => {
    const session = get().session
    if (!session || get().configPending || session.mode !== 'setup' || session.instanceId === id) return
    if (!useAppStore.getState().instances.some(instance => instance.id === id)) return
    const next = { ...session, instanceId: id, verified: null, complete: false }
    set({ session: next })
    if (getGuideTourSteps('en', session.mode)[session.index].tab === 'config') navigate(next)
  },
  beginConnection: instanceId => {
    const session = get().session
    if (!session || session.instanceId !== instanceId) return
    const attempt = ++nextConnectionAttempt
    set({ session: { ...session, verified: null, connectionAttempt: attempt } })
    return { sessionId: session.id, attempt }
  },
  recordConnection: (token, instance) => {
    const session = get().session
    const current = useAppStore.getState().instances.find(item => item.id === instance.id)
    if (!session || !token || session.id !== token.sessionId || session.connectionAttempt !== token.attempt || session.instanceId !== instance.id || !current) return
    if (guideInstanceKey(current) !== guideInstanceKey(instance)) return
    set({ session: { ...session, verified: guideInstanceKey(instance) } })
  },
  close: (pause = false, keepPage = false) => {
    const session = get().session
    if (!session) return
    set({ session: null, paused: pause ? session : null })
    // Exiting the guide must not unmount a form with uncommitted user input.
    if (!keepPage && !get().configPending) restore(session)
  },
  resume: () => {
    const paused = get().paused
    if (!paused || get().session) return
    const session = { ...paused, id: ++nextSessionId }
    set({ session, paused: null })
    navigate(session)
  },
  finish: (stay = false) => {
    const session = get().session
    if (!session) return
    set({ session: null, paused: null })
    if (!stay) restore(session)
  },
}))
