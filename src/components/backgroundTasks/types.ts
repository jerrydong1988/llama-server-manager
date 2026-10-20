export type TaskStatus = 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled' | 'interrupted'

export const isFinishedTask = (status: TaskStatus) => ['succeeded', 'failed', 'cancelled', 'interrupted'].includes(status)

export interface TaskRemovalResult {
  deleted_ids: string[]
  deleted_count: number
  retained_count: number
}

export interface ManagementTaskSummary {
  id: string
  operation: string
  status: TaskStatus
  created_at: number
  updated_at: number
}

export interface ManagementTask extends ManagementTaskSummary {
  result: unknown
  error: { code: string; message: string; retryable: boolean } | null
}
