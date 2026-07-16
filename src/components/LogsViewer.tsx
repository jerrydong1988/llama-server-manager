import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { confirm } from '@tauri-apps/plugin-dialog'
import { useVirtualizer } from '@tanstack/react-virtual'
import { Activity, ChevronDown, Pause, Play, Search, Trash2 } from 'lucide-react'
import { useAppStore } from '../store'
import { useI18n } from '../i18n'
import { getLogsLabels } from '../i18n/pageLabels'
import { Button, InsetSurface, MetricCard, SelectInput, Surface, TextInput } from './ui'

const ERROR_LOG_PATTERN = /error|fail|panic|fatal|\u9519\u8bef|\u5931\u8d25|\u5f02\u5e38|\u81f4\u547d/i
const WARNING_LOG_PATTERN = /warn|warning|\u8b66\u544a|\u8b66\u793a/i

const LogsViewer = () => {
  const instances = useAppStore(state => state.instances)
  const logs = useAppStore(state => state.logs)
  const clearLogs = useAppStore(state => state.clearLogs)
  const { t, lang } = useI18n()
  const [selectedInstanceId, setSelectedInstanceId] = useState<string>('')
  const [autoScroll, setAutoScroll] = useState(true)
  const [filterText, setFilterText] = useState('')
  const scrollRef = useRef<HTMLDivElement>(null)
  const userInteractedRef = useRef(false)
  const lastLogTimestampRef = useRef(0)
  const labels = useMemo(() => getLogsLabels(lang), [lang])

  const instanceLogs = selectedInstanceId ? (logs[selectedInstanceId] || []) : []
  const allLogs = useMemo(
    () => Object.values(logs).flat().sort((left, right) => left.timestamp - right.timestamp),
    [logs],
  )
  const sourceLogs = selectedInstanceId ? instanceLogs : allLogs
  const instanceNames = useMemo(
    () => new Map(instances.map(instance => [instance.id, instance.name])),
    [instances],
  )

  const displayLogs = useMemo(() => {
    const query = filterText.trim().toLowerCase()
    if (!query) {
      return sourceLogs
    }
    return sourceLogs.filter(entry => {
      const instanceName = instanceNames.get(entry.instanceId) || entry.instanceId
      return (
        entry.text.toLowerCase().includes(query) ||
        instanceName.toLowerCase().includes(query)
      )
    })
  }, [filterText, instanceNames, sourceLogs])

  const logVirtualizer = useVirtualizer({
    count: displayLogs.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 36,
    overscan: 16,
    getItemKey: index => {
      const entry = displayLogs[index]
      return entry ? `${entry.instanceId}-${entry.timestamp}-${index}` : index
    },
  })

  const hasLogs = displayLogs.length > 0
  const latestLogTimestamp = displayLogs[displayLogs.length - 1]?.timestamp || 0
  const hasRunningInstance = selectedInstanceId
    ? instances.some(instance => instance.id === selectedInstanceId && instance.status === 'running')
    : instances.some(instance => instance.status === 'running')

  const stats = useMemo(() => {
    const counts = { error: 0, warn: 0, info: 0 }
    for (const entry of sourceLogs) {
      if (ERROR_LOG_PATTERN.test(entry.text)) {
        counts.error += 1
      } else if (WARNING_LOG_PATTERN.test(entry.text)) {
        counts.warn += 1
      } else {
        counts.info += 1
      }
    }
    return counts
  }, [sourceLogs])

  useEffect(() => {
    if (latestLogTimestamp === lastLogTimestampRef.current) {
      return
    }
    lastLogTimestampRef.current = latestLogTimestamp
    if (autoScroll && scrollRef.current) {
      requestAnimationFrame(() => {
        if (displayLogs.length > 0) logVirtualizer.scrollToIndex(displayLogs.length - 1, { align: 'end' })
      })
    }
  }, [displayLogs.length, latestLogTimestamp, autoScroll, logVirtualizer])

  const handleInstanceChange = (id: string) => {
    setSelectedInstanceId(id)
    setAutoScroll(true)
    setFilterText('')
    userInteractedRef.current = false
    lastLogTimestampRef.current = 0
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
    if (displayLogs.length > 0) logVirtualizer.scrollToIndex(displayLogs.length - 1, { align: 'end' })
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
    lastLogTimestampRef.current = 0
    setAutoScroll(true)
  }

  return (
    <div className="space-y-5">
      <div className="flex flex-col gap-5 xl:flex-row xl:items-end xl:justify-between">
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <div className="rounded-lg border border-sky-500/20 bg-sky-500/10 p-3 text-sky-300">
              <Activity className="h-5 w-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-2xl font-semibold tracking-tight text-slate-50">{t.logs.title}</h1>
                <span className="rounded-full border border-slate-800 bg-slate-900 px-2.5 py-1 text-xs text-slate-400">
                  {sourceLogs.length} {t.logs.entries}
                </span>
              </div>
              <p className="mt-1 max-w-3xl text-sm text-slate-400">{labels.subtitle}</p>
            </div>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-3" data-guide="logs-clear">
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

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          { label: labels.visible, value: displayLogs.length, tone: 'text-sky-300 bg-sky-500/10 border-sky-500/20' },
          { label: labels.errors, value: stats.error, tone: 'text-red-300 bg-red-500/10 border-red-500/20' },
          { label: labels.warnings, value: stats.warn, tone: 'text-amber-300 bg-amber-500/10 border-amber-500/20' },
          { label: labels.info, value: stats.info, tone: 'text-emerald-300 bg-emerald-500/10 border-emerald-500/20' },
        ].map(card => (
          <MetricCard key={card.label} label={card.label} value={card.value} tone={card.tone} />
        ))}
      </div>

      <div className="grid gap-6 2xl:grid-cols-[320px,minmax(0,1fr)]">
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

        <Surface as="section" className="overflow-hidden">
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
            className="relative h-[560px] overflow-y-auto bg-[#050816] px-5 py-4 font-mono text-sm leading-7"
          >
            {!hasLogs ? (
              <div className="flex h-full flex-col items-center justify-center text-center">
                <Activity className="mb-4 h-10 w-10 text-slate-700" />
                <p className="text-slate-400">
                  {instances.length === 0 ? t.logs.noInstances : selectedInstanceId ? t.logs.noLogsForInstance : t.logs.noLogs}
                </p>
              </div>
            ) : (
              <div
                className="relative w-full"
                style={{ height: `${logVirtualizer.getTotalSize()}px` }}
              >
                {logVirtualizer.getVirtualItems().map(virtualRow => {
                  const entry = displayLogs[virtualRow.index]
                  const time = new Date(entry.timestamp).toLocaleTimeString()
                  const instanceName = instanceNames.get(entry.instanceId) || entry.instanceId
                  const text = entry.text
                  const lower = text.toLowerCase()

                  let tone = 'text-slate-300'
                  if (ERROR_LOG_PATTERN.test(text)) {
                    tone = 'text-red-300'
                  } else if (WARNING_LOG_PATTERN.test(text)) {
                    tone = 'text-amber-300'
                  } else if (/listening|ready|ok|success|loaded/.test(lower)) {
                    tone = 'text-emerald-300'
                  } else if (/token|speed|t\/s/.test(lower)) {
                    tone = 'text-sky-300'
                  }

                  return (
                    <div
                      key={virtualRow.key}
                      ref={logVirtualizer.measureElement}
                      data-index={virtualRow.index}
                      className="absolute left-0 top-0 w-full pb-1"
                      style={{ transform: `translateY(${virtualRow.start}px)` }}
                    >
                      <div className="grid grid-cols-[84px,minmax(120px,180px),minmax(0,1fr)] gap-4 rounded-lg px-2 py-1 hover:bg-white/5">
                        <span className="text-xs text-slate-600">{time}</span>
                        <span className="truncate text-xs text-slate-500">{instanceName}</span>
                        <span className={`${tone} whitespace-pre-wrap break-all`}>{text}</span>
                      </div>
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
