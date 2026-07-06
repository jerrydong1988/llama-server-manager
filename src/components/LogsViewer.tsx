import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { confirm } from '@tauri-apps/plugin-dialog'
import { Activity, ChevronDown, Pause, Play, Search, Trash2 } from 'lucide-react'
import { useAppStore } from '../store'
import { useI18n } from '../i18n'
import { Button, InsetSurface, MetricCard, SelectInput, Surface, TextInput } from './ui'

const LogsViewer = () => {
  const { instances, logs, clearLogs } = useAppStore()
  const { t, lang } = useI18n()
  const [selectedInstanceId, setSelectedInstanceId] = useState<string>('')
  const [autoScroll, setAutoScroll] = useState(true)
  const [filterText, setFilterText] = useState('')
  const scrollRef = useRef<HTMLDivElement>(null)
  const userInteractedRef = useRef(false)
  const logCountRef = useRef(0)
  const zh = lang === 'zh-CN'
  const labels = {
    subtitle: zh
      ? '\u67e5\u770b\u5b9e\u65f6\u8fdb\u7a0b\u8f93\u51fa\uff0c\u8fc7\u6ee4\u566a\u58f0\u65e5\u5fd7\uff0c\u5e76\u5728\u8c03\u8bd5\u65f6\u4fdd\u6301\u5c3e\u90e8\u8ddf\u968f\u3002'
      : 'Inspect live process output, filter noisy streams, and keep the tail pinned when you are actively debugging.',
    visible: zh ? '\u5f53\u524d\u53ef\u89c1' : 'Visible',
    errors: zh ? '\u9519\u8bef' : 'Errors',
    warnings: zh ? '\u8b66\u544a' : 'Warnings',
    info: zh ? '\u4fe1\u606f' : 'Info',
    logScope: zh ? '\u65e5\u5fd7\u8303\u56f4' : 'Log Scope',
    logScopeDesc: zh ? '\u5728\u5355\u5b9e\u4f8b\u65e5\u5fd7\u6d41\u4e0e\u5168\u5c40\u5408\u5e76\u65f6\u95f4\u7ebf\u4e4b\u95f4\u5207\u6362\u3002' : 'Switch between a single instance stream and the combined server timeline.',
    filterPlaceholder: zh ? '\u6309\u65e5\u5fd7\u5185\u5bb9\u6216\u5b9e\u4f8b\u540d\u79f0\u8fc7\u6ee4...' : 'Filter by message or instance...',
    scope: zh ? '\u8303\u56f4' : 'Scope',
    liveTail: zh ? '\u5b9e\u65f6\u5c3e\u90e8' : 'Live tail',
    runningSource: zh ? '\u8fd0\u884c\u6765\u6e90' : 'Running source',
    yes: zh ? '\u662f' : 'Yes',
    no: zh ? '\u5426' : 'No',
    liveConsole: zh ? '\u5b9e\u65f6\u63a7\u5236\u53f0' : 'Live Console',
    focusedStream: zh ? '\u5f53\u524d\u805a\u7126\u5355\u4e2a\u5b9e\u4f8b\u65e5\u5fd7\u6d41\u3002' : 'Focused on one instance stream.',
    mergedTimeline: zh ? '\u5f53\u524d\u5c55\u793a\u6240\u6709\u5b9e\u4f8b\u7684\u5408\u5e76\u65f6\u95f4\u7ebf\u3002' : 'Merged timeline across all instances.',
  }

  const instanceLogs = selectedInstanceId ? (logs[selectedInstanceId] || []) : []
  const allLogs = useMemo(
    () => Object.values(logs).flat().sort((left, right) => left.timestamp - right.timestamp),
    [logs],
  )
  const sourceLogs = selectedInstanceId ? instanceLogs : allLogs

  const displayLogs = useMemo(() => {
    const query = filterText.trim().toLowerCase()
    if (!query) {
      return sourceLogs
    }
    return sourceLogs.filter(entry => {
      const instanceName = instances.find(instance => instance.id === entry.instanceId)?.name || entry.instanceId
      return (
        entry.text.toLowerCase().includes(query) ||
        instanceName.toLowerCase().includes(query)
      )
    })
  }, [filterText, instances, sourceLogs])

  const hasLogs = displayLogs.length > 0
  const hasRunningInstance = selectedInstanceId
    ? instances.some(instance => instance.id === selectedInstanceId && instance.status === 'running')
    : instances.some(instance => instance.status === 'running')

  const stats = useMemo(() => {
    const counts = { error: 0, warn: 0, info: 0 }
    for (const entry of sourceLogs) {
      const lower = entry.text.toLowerCase()
      if (/error|fail|panic|fatal/.test(lower)) {
        counts.error += 1
      } else if (/warn|warning/.test(lower)) {
        counts.warn += 1
      } else {
        counts.info += 1
      }
    }
    return counts
  }, [sourceLogs])

  useEffect(() => {
    if (displayLogs.length === logCountRef.current) {
      return
    }
    logCountRef.current = displayLogs.length
    if (autoScroll && scrollRef.current) {
      requestAnimationFrame(() => {
        if (scrollRef.current) {
          scrollRef.current.scrollTop = scrollRef.current.scrollHeight
        }
      })
    }
  }, [displayLogs.length, autoScroll])

  const handleInstanceChange = (id: string) => {
    setSelectedInstanceId(id)
    setAutoScroll(true)
    setFilterText('')
    userInteractedRef.current = false
    logCountRef.current = 0
  }

  const handleScroll = useCallback(() => {
    const element = scrollRef.current
    if (!element) {
      return
    }
    const atBottom = element.scrollHeight - element.scrollTop - element.clientHeight < 80
    if (!atBottom && autoScroll) {
      userInteractedRef.current = true
      setAutoScroll(false)
    }
    if (atBottom && !autoScroll) {
      userInteractedRef.current = false
      setAutoScroll(true)
    }
  }, [autoScroll])

  const scrollToBottom = () => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
    setAutoScroll(true)
    userInteractedRef.current = false
  }

  const toggleAutoScroll = () => {
    if (autoScroll) {
      setAutoScroll(false)
    } else {
      scrollToBottom()
    }
  }

  const handleClear = async () => {
    if (selectedInstanceId) {
      const confirmed = await confirm(t.logs.clearConfirm, { title: t.logs.clear, kind: 'warning' })
      if (!confirmed) {
        return
      }
      clearLogs(selectedInstanceId)
    } else {
      const confirmed = await confirm(t.logs.clearAllConfirm || t.logs.clearConfirm, { title: t.logs.clear, kind: 'warning' })
      if (!confirmed) {
        return
      }
      for (const id of Object.keys(logs)) {
        clearLogs(id)
      }
    }
    logCountRef.current = 0
    setAutoScroll(true)
  }

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="mb-6 flex flex-col gap-5 xl:flex-row xl:items-end xl:justify-between">
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <div className="rounded-2xl border border-sky-500/20 bg-sky-500/10 p-3 text-sky-300">
              <Activity className="h-5 w-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-3xl font-semibold tracking-tight text-slate-50">{t.logs.title}</h1>
                <span className="rounded-full border border-slate-800 bg-slate-900 px-2.5 py-1 text-xs text-slate-400">
                  {sourceLogs.length} {t.logs.entries}
                </span>
              </div>
              <p className="text-sm text-slate-400">{labels.subtitle}</p>
            </div>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <Button
            onClick={toggleAutoScroll}
            variant={autoScroll ? 'success' : 'secondary'}
            icon={autoScroll ? <Play className="h-4 w-4" /> : <Pause className="h-4 w-4" />}
          >
            {autoScroll ? t.logs.following : t.logs.paused}
          </Button>
          <Button
            onClick={handleClear}
            variant="danger"
            icon={<Trash2 className="h-4 w-4" />}
          >
            {t.logs.clear}
          </Button>
        </div>
      </div>

      <div className="mb-6 grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          { label: labels.visible, value: displayLogs.length, tone: 'text-sky-300 bg-sky-500/10 border-sky-500/20' },
          { label: labels.errors, value: stats.error, tone: 'text-red-300 bg-red-500/10 border-red-500/20' },
          { label: labels.warnings, value: stats.warn, tone: 'text-amber-300 bg-amber-500/10 border-amber-500/20' },
          { label: labels.info, value: stats.info, tone: 'text-emerald-300 bg-emerald-500/10 border-emerald-500/20' },
        ].map(card => (
          <MetricCard key={card.label} label={card.label} value={card.value} tone={card.tone} />
        ))}
      </div>

      <div className="grid gap-6 xl:grid-cols-[320px,minmax(0,1fr)]">
        <Surface as="aside" className="h-fit p-5">
          <div className="mb-4">
            <h2 className="text-lg font-semibold text-slate-50">{labels.logScope}</h2>
            <p className="mt-1 text-sm text-slate-400">{labels.logScopeDesc}</p>
          </div>

          <label className="mb-4 block">
            <span className="mb-2 block text-xs font-medium text-slate-400">{t.logs.selectInstance}</span>
            <SelectInput
              value={selectedInstanceId}
              onChange={event => handleInstanceChange(event.target.value)}
              className="w-full"
            >
              <option value="">{t.logs.allInstances}</option>
              {instances.map(instance => (
                <option key={instance.id} value={instance.id}>
                  {instance.name} {instance.status === 'running' ? t.logs.runningTag : ''}
                </option>
              ))}
            </SelectInput>
          </label>

          <TextInput
            value={filterText}
            onChange={event => setFilterText(event.target.value)}
            placeholder={labels.filterPlaceholder}
            leadingIcon={<Search className="h-4 w-4" />}
          />

          <InsetSurface className="mt-5 space-y-3 p-4">
            {[
              [labels.scope, selectedInstanceId ? instances.find(instance => instance.id === selectedInstanceId)?.name || selectedInstanceId : t.logs.allInstances],
              [labels.liveTail, autoScroll ? t.logs.following : t.logs.paused],
              [labels.runningSource, hasRunningInstance ? labels.yes : labels.no],
            ].map(([label, value]) => (
              <div key={label} className="flex items-center justify-between gap-3">
                <span className="text-sm text-slate-500">{label}</span>
                <span className="max-w-[160px] truncate text-right text-sm text-slate-200" title={value}>
                  {value}
                </span>
              </div>
            ))}
          </InsetSurface>

          <p className="mt-4 text-xs leading-6 text-slate-500">{t.logs.hint}</p>
        </Surface>

        <Surface as="section" className="min-h-[680px] overflow-hidden">
          <div className="flex items-center justify-between border-b border-slate-800 bg-slate-950/90 px-5 py-3">
            <div>
              <h2 className="text-lg font-semibold text-slate-50">{labels.liveConsole}</h2>
              <p className="mt-1 text-sm text-slate-400">
                {selectedInstanceId ? labels.focusedStream : labels.mergedTimeline}
              </p>
            </div>
            {!autoScroll && hasLogs && (
              <Button
                onClick={scrollToBottom}
                variant="primary"
                size="sm"
                icon={<ChevronDown className="h-4 w-4" />}
              >
                {t.logs.scrollToBottom}
              </Button>
            )}
          </div>

          <div
            ref={scrollRef}
            onScroll={handleScroll}
            className="relative h-[620px] overflow-y-auto bg-[#050816] px-5 py-4 font-mono text-sm leading-7"
          >
            {!hasLogs ? (
              <div className="flex h-full flex-col items-center justify-center text-center">
                <Activity className="mb-4 h-10 w-10 text-slate-700" />
                <p className="text-slate-400">
                  {instances.length === 0 ? t.logs.noInstances : selectedInstanceId ? t.logs.noLogsForInstance : t.logs.noLogs}
                </p>
              </div>
            ) : (
              <div className="space-y-1">
                {displayLogs.slice(-1200).map((entry, index) => {
                  const time = new Date(entry.timestamp).toLocaleTimeString()
                  const instanceName = instances.find(instance => instance.id === entry.instanceId)?.name || entry.instanceId
                  const text = entry.text
                  const lower = text.toLowerCase()

                  let tone = 'text-slate-300'
                  if (/error|fail|panic|fatal/.test(lower)) {
                    tone = 'text-red-300'
                  } else if (/warn|warning/.test(lower)) {
                    tone = 'text-amber-300'
                  } else if (/listening|ready|ok|success|loaded/.test(lower)) {
                    tone = 'text-emerald-300'
                  } else if (/token|speed|t\/s/.test(lower)) {
                    tone = 'text-sky-300'
                  }

                  return (
                    <div key={`${entry.timestamp}-${index}`} className="grid grid-cols-[84px,180px,minmax(0,1fr)] gap-4 rounded-lg px-2 py-1 hover:bg-white/5">
                      <span className="text-xs text-slate-600">{time}</span>
                      <span className="truncate text-xs text-slate-500">{instanceName}</span>
                      <span className={`${tone} whitespace-pre-wrap break-all`}>{text}</span>
                    </div>
                  )
                })}
              </div>
            )}

            {autoScroll && hasLogs && hasRunningInstance && (
              <div className="absolute bottom-3 left-4 inline-flex items-center gap-2 rounded-full border border-emerald-500/20 bg-emerald-500/10 px-3 py-1 text-xs text-emerald-200">
                <Play className="h-3 w-3" />
                {t.logs.following}
              </div>
            )}
          </div>
        </Surface>
      </div>
    </div>
  )
}

export default LogsViewer
