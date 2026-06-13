import { useState, useEffect, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Trash2 } from 'lucide-react'
import { type SessionMeta, type DataPoint } from './types'
import SessionCard from './SessionCard'
import ComparePanel from './ComparePanel'

export default function HistoryPage() {
  const [sessions, setSessions] = useState<SessionMeta[]>([])
  const [loading, setLoading] = useState(true)
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [chartData, setChartData] = useState<Record<string, DataPoint[] | null>>({})
  const [chartLoading, setChartLoading] = useState<Record<string, boolean>>({})
  const [selectedIds, setSelectedIds] = useState<string[]>([])
  const [instanceFilter, setInstanceFilter] = useState<string>('')
  const [backendFilter, setBackendFilter] = useState<string>('')

  const loadSessions = useCallback(async () => {
    setLoading(true)
    try {
      const list = await invoke<SessionMeta[]>('list_sessions', {
        instanceId: instanceFilter || null,
      })
      setSessions(list)
    } catch (e) {
      console.error('Failed to load sessions:', e)
    }
    setLoading(false)
  }, [instanceFilter])

  useEffect(() => { loadSessions() }, [loadSessions])

  const toggleDetail = async (sessionId: string) => {
    if (expandedId === sessionId) {
      setExpandedId(null)
      return
    }
    setExpandedId(sessionId)

    if (!chartData[sessionId]) {
      setChartLoading(prev => ({ ...prev, [sessionId]: true }))
      try {
        const data = await invoke<DataPoint[]>('get_session_data', { sessionId })
        setChartData(prev => ({ ...prev, [sessionId]: data }))
      } catch (e) {
        console.error('Failed to load session data:', e)
        setChartData(prev => ({ ...prev, [sessionId]: null }))
      }
      setChartLoading(prev => ({ ...prev, [sessionId]: false }))
    }
  }

  const toggleSelect = (sessionId: string) => {
    setSelectedIds(prev => {
      if (prev.includes(sessionId)) {
        return prev.filter(id => id !== sessionId)
      }
      if (prev.length >= 2) {
        // Replace the oldest selection
        return [prev[1], sessionId]
      }
      return [...prev, sessionId]
    })
  }

  const deleteSession = async (sessionId: string) => {
    try {
      await invoke('delete_session', { sessionId })
      setSessions(prev => prev.filter(s => s.id !== sessionId))
      setSelectedIds(prev => prev.filter(id => id !== sessionId))
      if (expandedId === sessionId) setExpandedId(null)
    } catch (e) {
      console.error('Failed to delete session:', e)
    }
  }

  const clearAll = async () => {
    if (!window.confirm('Clear all history? This cannot be undone.')) return
    try {
      await invoke('clear_all_history')
      setSessions([])
      setSelectedIds([])
      setExpandedId(null)
      setChartData({})
    } catch (e) {
      console.error('Failed to clear history:', e)
    }
  }

  // Extract unique instance names and backends for filters
  const instanceNames = [...new Set(sessions.map(s => s.instance_name))]
  const backends = [...new Set(sessions.map(s => s.engine_backend))]

  // Apply backend filter client-side
  const filtered = backendFilter
    ? sessions.filter(s => s.engine_backend === backendFilter)
    : sessions

  // Selected sessions for comparison
  const selectedA = sessions.find(s => s.id === selectedIds[0])
  const selectedB = sessions.find(s => s.id === selectedIds[1])

  return (
    <div className="flex-1 p-6 overflow-y-auto">
      <div className="flex items-center justify-between mb-6">
        <h2 className="text-2xl font-bold">History</h2>
        <div className="flex items-center gap-3">
          {/* Filters */}
          {instanceNames.length > 1 && (
            <select value={instanceFilter} onChange={e => setInstanceFilter(e.target.value)}
              className="px-2 py-1 text-xs border dark:border-gray-700 rounded bg-white dark:bg-gray-900">
              <option value="">All instances</option>
              {instanceNames.map(n => <option key={n} value={n}>{n}</option>)}
            </select>
          )}
          {backends.length > 1 && (
            <select value={backendFilter} onChange={e => setBackendFilter(e.target.value)}
              className="px-2 py-1 text-xs border dark:border-gray-700 rounded bg-white dark:bg-gray-900">
              <option value="">All backends</option>
              {backends.map(b => <option key={b} value={b}>{b}</option>)}
            </select>
          )}
          {sessions.length > 0 && (
            <button onClick={clearAll}
              className="flex items-center gap-1 px-2 py-1 text-xs text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 rounded transition-colors">
              <Trash2 className="w-3 h-3" /> Clear All
            </button>
          )}
        </div>
      </div>

      {/* Compare panel */}
      {selectedA && selectedB && (
        <div className="mb-6">
          <ComparePanel
            sessionA={selectedA}
            dataA={chartData[selectedA.id] || null}
            sessionB={selectedB}
            dataB={chartData[selectedB.id] || null}
            onClear={() => setSelectedIds([])}
          />
        </div>
      )}

      {/* Session list */}
      {loading ? (
        <div className="text-center text-gray-500 py-12">Loading sessions...</div>
      ) : filtered.length === 0 ? (
        <div className="text-center text-gray-500 py-12">
          {sessions.length === 0
            ? 'No sessions recorded yet. Start an instance and performance data will appear here.'
            : 'No sessions match the current filters.'}
        </div>
      ) : (
        <div className="space-y-3">
          {filtered.map(session => (
            <div key={session.id} className="relative group">
              <SessionCard
                session={session}
                data={chartData[session.id] || null}
                loading={chartLoading[session.id] || false}
                expanded={expandedId === session.id}
                onToggleDetail={() => toggleDetail(session.id)}
                selected={selectedIds.includes(session.id)}
                onToggleSelect={() => toggleSelect(session.id)}
              />
              {/* Delete button (appears on hover) */}
              <button
                onClick={() => deleteSession(session.id)}
                className="absolute top-2 right-2 p-1 opacity-0 group-hover:opacity-100 transition-opacity hover:bg-red-50 dark:hover:bg-red-900/20 rounded text-red-400 hover:text-red-600"
                title="Delete session"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
