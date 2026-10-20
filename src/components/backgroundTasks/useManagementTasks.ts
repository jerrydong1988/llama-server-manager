import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { invokeApp as invoke, normalizeInvokeError } from '../../lib/ipc'
import type { ManagementTask, ManagementTaskSummary, TaskRemovalResult } from './types'

export function useManagementTasks() {
  const [tasks, setTasks] = useState<ManagementTaskSummary[]>([])
  const [selectedId, selectTask] = useState('')
  const [detail, setDetail] = useState<ManagementTask | null>(null)
  const [loading, setLoading] = useState(true)
  const [listError, setListError] = useState('')
  const [detailError, setDetailError] = useState('')
  const [cancelling, setCancelling] = useState('')
  const [removing, setRemoving] = useState('')
  const [historyFeedback, setHistoryFeedback] = useState<{ count?: number; error?: string; code?: string } | null>(null)
  const [feedback, setFeedback] = useState<{ id: string; status?: string; error?: string; code?: string } | null>(null)
  const refreshRef = useRef<() => void>(() => {})
  const cancelLock = useRef(false)
  const historyGeneration = useRef(0)
  const mounted = useRef(false)
  useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])

  useEffect(() => {
    let disposed = false
    let pending = false
    let again = false
    let unlisten: (() => void) | undefined
    const refresh = async () => {
      if (disposed || document.hidden) return
      if (pending) { again = true; return }
      pending = true
      setLoading(true)
      do {
        again = false
        const generation = historyGeneration.current
        try {
          const rows = await invoke<ManagementTaskSummary[]>('list_management_tasks')
          if (disposed) return
          if (generation !== historyGeneration.current) { again = true; continue }
          setTasks(rows)
          setListError('')
          // The selected row may have been removed by CLI/MCP or cleanup here.
          if (!rows.some(task => task.id === selectedId)) {
            selectTask(rows[0]?.id ?? '')
            setDetail(null)
            setDetailError('')
            continue
          }
        } catch (error) {
          if (!disposed) setListError(String(error))
        }
        if (selectedId && !disposed) {
          try {
            const task = await invoke<ManagementTask>('get_management_task', { id: selectedId })
            if (disposed) return
            if (generation !== historyGeneration.current) { again = true; continue }
            setDetail(task)
            setDetailError('')
          } catch (error) {
            if (!disposed) {
              setDetail(null)
              if (normalizeInvokeError(error).code === 'NOT_FOUND') {
                selectTask('')
                setDetailError('')
              } else setDetailError(String(error))
            }
          }
        }
      } while (again && !disposed)
      pending = false
      if (!disposed) setLoading(false)
    }
    const requestRefresh = () => { void refresh() }
    refreshRef.current = requestRefresh
    requestRefresh()
    // Events update promptly; polling also covers missed events / reconnects.
    void listen('management-task-changed', requestRefresh).then(cleanup => {
      if (disposed) cleanup()
      else unlisten = cleanup
    }).catch(() => {})
    const timer = window.setInterval(requestRefresh, 3000)
    window.addEventListener('focus', requestRefresh)
    document.addEventListener('visibilitychange', requestRefresh)
    return () => {
      disposed = true
      unlisten?.()
      window.clearInterval(timer)
      window.removeEventListener('focus', requestRefresh)
      document.removeEventListener('visibilitychange', requestRefresh)
    }
  }, [selectedId])

  const cancel = async (id: string) => {
    if (cancelLock.current) return
    cancelLock.current = true
    setCancelling(id)
    setFeedback(null)
    try {
      const result = await invoke<{ id: string; status: string }>('cancel_management_task', { id })
      if (mounted.current) setFeedback({ id, status: result.status })
    } catch (error) {
      const failure = normalizeInvokeError(error)
      if (mounted.current) setFeedback({ id, code: failure.code, error: failure.message })
    } finally {
      cancelLock.current = false
      if (mounted.current) { setCancelling(''); refreshRef.current() }
    }
  }

  const removeHistory = async (id?: string) => {
    if (cancelLock.current) return
    cancelLock.current = true
    setRemoving(id ?? '*')
    setHistoryFeedback(null)
    try {
      const result = await invoke<TaskRemovalResult>(id ? 'delete_management_task' : 'clear_management_tasks', id ? { id } : {})
      historyGeneration.current++
      if (mounted.current) {
        setTasks(rows => rows.filter(task => !result.deleted_ids.includes(task.id)))
        selectTask(selected => result.deleted_ids.includes(selected) ? '' : selected)
        setDetail(task => task && result.deleted_ids.includes(task.id) ? null : task)
        setDetailError('')
        setHistoryFeedback({ count: result.deleted_count })
      }
    } catch (error) {
      const failure = normalizeInvokeError(error)
      if (mounted.current) setHistoryFeedback({ code: failure.code, error: failure.message })
    } finally {
      cancelLock.current = false
      if (mounted.current) { setRemoving(''); refreshRef.current() }
    }
  }
  return { tasks, selectedId, selectTask, detail: detail?.id === selectedId ? detail : null, loading, listError, detailError,
    cancelling, feedback, cancel, removing, historyFeedback, removeHistory, refresh: () => refreshRef.current() }
}
