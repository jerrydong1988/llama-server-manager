import { useState, useRef, useEffect, useCallback, useMemo } from 'react'
import { useAppStore } from '../store'
import { Trash2, ChevronDown, Pause, Play } from 'lucide-react'
import { useI18n } from '../i18n'
import { confirm } from '@tauri-apps/plugin-dialog'

const LogsViewer = () => {
  const { instances, logs, clearLogs } = useAppStore()
  const { t } = useI18n()
  const [selectedInstanceId, setSelectedInstanceId] = useState<string>('')
  const [autoScroll, setAutoScroll] = useState(true)
  const scrollRef = useRef<HTMLDivElement>(null)
  const userInteractedRef = useRef(false)
  const logCountRef = useRef(0)

  const instanceLogs = selectedInstanceId ? (logs[selectedInstanceId] || []) : []
  const allLogs = useMemo(() => Object.values(logs).flat().sort((a, b) => a.timestamp - b.timestamp), [logs])
  const displayLogs = selectedInstanceId ? instanceLogs : allLogs

  // Auto-scroll when new logs arrive (if autoScroll is enabled)
  useEffect(() => {
    if (displayLogs.length === logCountRef.current) return
    logCountRef.current = displayLogs.length
    if (autoScroll && scrollRef.current) {
      requestAnimationFrame(() => {
        if (scrollRef.current) {
          scrollRef.current.scrollTop = scrollRef.current.scrollHeight
        }
      })
    }
  }, [displayLogs.length, autoScroll])

  // Reset auto-scroll when switching instance
  const handleInstanceChange = (id: string) => {
    setSelectedInstanceId(id)
    setAutoScroll(true)
    userInteractedRef.current = false
    logCountRef.current = 0
  }

  // Detect manual scroll position changes
  const handleScroll = useCallback(() => {
    const el = scrollRef.current
    if (!el) return
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 80
    // user interacted: if they scrolled up, disable auto-scroll
    if (!atBottom && autoScroll) {
      userInteractedRef.current = true
      setAutoScroll(false)
    }
    // if they scrolled back to bottom, re-enable auto-scroll
    if (atBottom && !autoScroll) {
      userInteractedRef.current = false
      setAutoScroll(true)
    }
  }, [autoScroll])

  // Jump to bottom
  const scrollToBottom = () => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
    setAutoScroll(true)
    userInteractedRef.current = false
  }

  // Toggle pause/resume
  const toggleAutoScroll = () => {
    if (autoScroll) {
      setAutoScroll(false)
    } else {
      scrollToBottom()
    }
  }

  const hasLogs = displayLogs.length > 0
  const hasRunningInstance = selectedInstanceId
    ? instances.some(i => i.id === selectedInstanceId && i.status === 'running')
    : instances.some(i => i.status === 'running')

  return (
    <div className="h-screen flex flex-col">
      {/* Toolbar */}
      <div className="flex items-center gap-4 p-4 shrink-0">
        <label className="text-sm font-medium">{t.logs.selectInstance}</label>
        <select value={selectedInstanceId} onChange={(e) => handleInstanceChange(e.target.value)}
          className="select-custom pl-3 pr-8 py-2 text-gray-900 dark:text-gray-100 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800 min-w-[250px]">
          <option value="">{t.logs.allInstances}</option>
          {instances.map((inst) => (
            <option key={inst.id} value={inst.id}>
              {inst.name} {inst.status === 'running' ? t.logs.runningTag : ''}
            </option>
          ))}
        </select>
        {selectedInstanceId && (
          <button onClick={async () => { if (!await confirm(t.logs.clearConfirm, { title: t.logs.clear, kind: 'warning' })) return; clearLogs(selectedInstanceId); logCountRef.current = 0; setAutoScroll(true) }}
            className="flex items-center gap-1 px-3 py-2 text-red-600 hover:bg-red-50 dark:hover:bg-red-900/20 rounded-lg transition-colors text-sm">
            <Trash2 className="w-4 h-4" /> {t.logs.clear}
          </button>
        )}
      </div>

      {/* Log viewer — fills remaining vertical space */}
      <div className="flex-1 overflow-hidden px-4 pb-2">
        <div ref={scrollRef} onScroll={handleScroll}
          className="h-full bg-gray-900 dark:bg-gray-950 p-4 rounded-lg overflow-y-auto font-mono text-sm leading-relaxed relative">
          {!hasLogs ? (
            <p className="text-gray-500 text-center py-8">
              {instances.length === 0 ? t.logs.noInstances : selectedInstanceId ? t.logs.noLogsForInstance : t.logs.noLogs}
            </p>
          ) : (
            displayLogs.slice(-1000).map((entry, idx) => {
              const time = new Date(entry.timestamp).toLocaleTimeString()
              const instName = instances.find((i) => i.id === entry.instanceId)?.name || entry.instanceId
              const text = entry.text
              let colorClass = 'text-gray-300'
              const lower = text.toLowerCase()
              if (/error|fail|panic|fatal/.test(lower)) colorClass = 'text-red-400'
              else if (/warn|warning/.test(lower)) colorClass = 'text-yellow-400'
              else if (/listening|ready|ok|success|loaded/.test(lower)) colorClass = 'text-green-400'
              else if (/token|speed|t\/s/.test(lower)) colorClass = 'text-cyan-400'
              return (
                <div key={`${entry.timestamp}-${idx}`} className={`${colorClass} whitespace-pre-wrap break-all`}>
                  {!selectedInstanceId && <span className="text-gray-500">[{time}] [{instName}] </span>}
                  {text}
                </div>
              )
            })
          )}

          {/* Pause indicator + Resume button (top-right) */}
          {!autoScroll && hasLogs && (
            <button onClick={toggleAutoScroll}
              className="absolute top-2 right-2 flex items-center gap-1.5 px-2.5 py-1.5 bg-gray-700/90 hover:bg-gray-600 text-white rounded-lg text-xs shadow-lg transition-colors">
              <Pause className="w-3 h-3" /> ⏸ {t.logs.paused || '已暂停'}
            </button>
          )}

          {/* Scroll to bottom button (bottom-right) */}
          {!autoScroll && hasLogs && (
            <button onClick={scrollToBottom}
              className="absolute bottom-2 right-2 flex items-center gap-1.5 px-3 py-2 bg-blue-600/90 hover:bg-blue-500 text-white rounded-lg text-xs shadow-lg transition-colors">
              <ChevronDown className="w-4 h-4" /> {t.logs.scrollToBottom || '最新'}
            </button>
          )}

          {/* Auto-scroll indicator (when actively following) */}
          {autoScroll && hasLogs && hasRunningInstance && (
            <div className="absolute bottom-2 left-2 flex items-center gap-1 px-2 py-1 bg-gray-700/70 text-gray-300 rounded text-xs">
              <Play className="w-3 h-3 text-green-400" /> {t.logs.following || '实时'}
            </div>
          )}
        </div>
      </div>

      <div className="flex items-center justify-between text-xs text-gray-500 dark:text-gray-400 px-4 pb-4 shrink-0">
        <span>{t.logs.hint}</span>
        <span>{displayLogs.length} {t.logs.entries || '条记录'}</span>
      </div>
    </div>
  )
}

export default LogsViewer
