import { useEffect, useState } from 'react'
import { invokeApp } from '../../lib/ipc'

export function useRouterReport<T>(command: string, args: Record<string, unknown>, interval = 15_000, enabled = true, revision = 0) {
  const [report, setReport] = useState<T | null>(null)
  const [error, setError] = useState('')
  const serialized = JSON.stringify(args)
  useEffect(() => {
    let disposed = false
    let busy = false
    setReport(null); setError('')
    if (!enabled) return
    const load = async () => {
      if (busy || disposed) return
      busy = true
      try {
        const result = await invokeApp<T>(command, JSON.parse(serialized))
        if (!disposed) { setReport(result); setError('') }
      } catch (e) { if (!disposed) { setReport(null); setError(String(e)) } }
      finally { busy = false }
    }
    void load()
    const timer = window.setInterval(() => { if (!document.hidden) void load() }, interval)
    return () => { disposed = true; window.clearInterval(timer) }
  }, [command, serialized, interval, enabled, revision])
  return { report, error }
}
