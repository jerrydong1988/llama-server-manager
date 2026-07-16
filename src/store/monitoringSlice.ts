import type { AppStoreSet } from './helpers'
import type { AppState, MonitoringFrame, PerfUpdateEvent } from './types'

const MAX_MONITORING_FRAMES = 3_600

function compareMonitoringFrameOrder(left: MonitoringFrame, right: MonitoringFrame) {
  const sessionOrder = left.sessionStartedAt - right.sessionStartedAt
  return sessionOrder || left.ts - right.ts
}

function sameMonitoringSession(left: MonitoringFrame, right: MonitoringFrame) {
  if (left.sessionId != null || right.sessionId != null) {
    return left.sessionId === right.sessionId
  }
  return left.sessionStartedAt === right.sessionStartedAt
}

export function appendMonitoringFrame(previous: MonitoringFrame[], frame: MonitoringFrame) {
  const previousLast = previous[previous.length - 1]
  if (
    previousLast
    && !sameMonitoringSession(previousLast, frame)
    && frame.sessionStartedAt < previousLast.sessionStartedAt
  ) {
    return previous
  }
  const sameSession = previousLast && sameMonitoringSession(previousLast, frame) ? previous : []
  const last = sameSession[sameSession.length - 1]
  if (!last || frame.ts > last.ts) {
    return [...sameSession, frame].slice(-MAX_MONITORING_FRAMES)
  }
  if (frame.ts === last.ts) {
    return [...sameSession.slice(0, -1), frame]
  }
  const byTimestamp = new Map(sameSession.map(item => [item.ts, item]))
  byTimestamp.set(frame.ts, frame)
  return [...byTimestamp.values()]
    .sort((left, right) => left.ts - right.ts)
    .slice(-MAX_MONITORING_FRAMES)
}

export function mergeMonitoringFrames(
  previous: MonitoringFrame[],
  incoming: MonitoringFrame[],
) {
  if (incoming.length === 0) return previous
  const latest = [...previous, ...incoming].reduce((current, frame) => (
    !current || compareMonitoringFrameOrder(frame, current) >= 0 ? frame : current
  ), null as MonitoringFrame | null)
  if (!latest) return previous
  const byTimestamp = new Map<number, MonitoringFrame>()
  for (const frame of [...previous, ...incoming]) {
    if (sameMonitoringSession(frame, latest)) byTimestamp.set(frame.ts, frame)
  }
  return [...byTimestamp.values()]
    .sort((left, right) => left.ts - right.ts)
    .slice(-MAX_MONITORING_FRAMES)
}

export function createMonitoringSlice(set: AppStoreSet): Pick<
  AppState,
  | 'ingestMonitoringFrame'
  | 'hydrateMonitoringFrames'
  | 'applyPerfUpdate'
> {
  const pendingFrames: MonitoringFrame[] = []
  let flushTimer: ReturnType<typeof setTimeout> | null = null
  const flushPendingFrames = () => {
    flushTimer = null
    const frames = pendingFrames.splice(0)
    if (frames.length === 0) return
    set((state) => {
      const nextFrames = { ...state.monitoringFramesByInstance }
      const nextCurrent = { ...state.monitoringCurrentByInstance }
      for (const frame of frames) {
        const timeline = appendMonitoringFrame(nextFrames[frame.instanceId] || [], frame)
        nextFrames[frame.instanceId] = timeline
        const current = timeline[timeline.length - 1]
        if (current) nextCurrent[frame.instanceId] = current
      }
      return {
        monitoringFramesByInstance: nextFrames,
        monitoringCurrentByInstance: nextCurrent,
      }
    })
  }

  return {
    ingestMonitoringFrame: (frame) => {
      pendingFrames.push(frame)
      flushTimer ??= setTimeout(flushPendingFrames, 50)
    },
    hydrateMonitoringFrames: (frames) => set((state) => {
      const nextFrames = { ...state.monitoringFramesByInstance }
      const nextCurrent = { ...state.monitoringCurrentByInstance }
      const grouped = new Map<string, MonitoringFrame[]>()
      for (const frame of frames) {
        const instanceFrames = grouped.get(frame.instanceId) || []
        instanceFrames.push(frame)
        grouped.set(frame.instanceId, instanceFrames)
        const current = nextCurrent[frame.instanceId]
        if (!current || compareMonitoringFrameOrder(frame, current) >= 0) {
          nextCurrent[frame.instanceId] = frame
        }
      }
      for (const [instanceId, instanceFrames] of grouped) {
        nextFrames[instanceId] = mergeMonitoringFrames(
          nextFrames[instanceId] || [],
          instanceFrames,
        )
      }
      return {
        monitoringFramesByInstance: nextFrames,
        monitoringCurrentByInstance: nextCurrent,
      }
    }),
    applyPerfUpdate: (event: PerfUpdateEvent) => set((state) => ({
      runningTasksByInstance: {
        ...state.runningTasksByInstance,
        [event.instanceId]: event.tasks,
      },
      lastCompletedTaskByInstance: event.lastCompleted ? {
        ...state.lastCompletedTaskByInstance,
        [event.instanceId]: event.lastCompleted,
      } : state.lastCompletedTaskByInstance,
    })),
  }
}
