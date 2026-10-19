import type { ManagementTask } from '../src/components/backgroundTasks/types'

const tasks: ManagementTask[] = ['queued', 'running', 'succeeded', 'failed', 'cancelled', 'interrupted'].map((status, index) => ({
  id: `task-${status}`, operation: index < 2 ? 'engines.probe_all' : 'models.refresh',
  status: status as ManagementTask['status'], created_at: Date.now() - (index + 1) * 1000,
  updated_at: Date.now(), result: status === 'succeeded' ? { models: 28, text: '<script>unsafe()</script>' } : null,
  error: status === 'failed' ? { code: 'IO', message: 'scan unavailable', retryable: true }
    : status === 'interrupted' ? { code: 'INTERRUPTED', message: 'host exited', retryable: false } : null,
}))
let listFailures = 0
let cancelRace = ''
let detailDelay = 0
declare global {
  interface Window {
    __MANAGEMENT_TASK_TEST__: {
      tasks: ManagementTask[]
      failList: () => void
      race: (status: string) => void
      delayDetail: (ms: number) => void
    }
  }
}
window.__MANAGEMENT_TASK_TEST__ = { tasks, failList: () => { listFailures++ }, race: status => { cancelRace = status }, delayDetail: ms => { detailDelay = ms } }
export function managementTasksMock(command: string, args: Record<string, unknown>): unknown {
  if (command === 'list_management_tasks') {
    if (listFailures > 0) { listFailures--; throw new Error('Task service unavailable') }
    return structuredClone(tasks.map(({ id, operation, status, created_at, updated_at }) => ({ id, operation, status, created_at, updated_at })))
  }
  const task = tasks.find(row => row.id === args.id)
  if (!task) throw { code: 'NOT_FOUND', message: 'task not found', retryable: false }
  if (command === 'get_management_task') {
    const snapshot = structuredClone(task)
    return new Promise(resolve => setTimeout(() => resolve(snapshot), detailDelay))
  }
  if (cancelRace) { task.status = cancelRace as ManagementTask['status']; cancelRace = '' }
  if (task.status === 'running') throw { code: 'TASK_RUNNING', message: 'operation has started', retryable: false }
  if (task.status === 'queued') {
    task.status = 'cancelled'
    task.updated_at = Date.now()
    task.error = { code: 'CANCELLED', message: 'cancelled before side effects started', retryable: false }
  }
  return { id: task.id, status: task.status }
}
