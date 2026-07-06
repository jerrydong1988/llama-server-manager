import { useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Activity, BarChart3, Cpu, Gauge, Layers3, Server } from 'lucide-react'
import { useAppStore } from '../../store'
import { useI18n } from '../../i18n'
import type { SystemMetrics } from '../../store/types'
import PerfAnalysis from './PerfAnalysis'
import GaugeMeter from './GaugeMeter'
import { Badge, Button, EmptyState, InsetSurface, MetricCard, SectionHeader, SelectInput, Surface } from '../ui'

interface SlotInfo {
  id: number
  is_processing: boolean
  n_ctx: number
}

export default function PerformancePage() {
  const { t, lang } = useI18n()
  const zh = lang === 'zh-CN'
  const { instances } = useAppStore()
  const running = instances.filter(instance => instance.status === 'running')
  const [selectedId, setSelectedId] = useState('')
  const [interval, setIntervalState] = useState(5)
  const [sys, setSys] = useState<SystemMetrics | null>(null)
  const [slots, setSlots] = useState<SlotInfo[]>([])
  const [viewMode, setViewMode] = useState<'overview' | 'gauges'>('overview')

  const inst = instances.find(instance => instance.id === selectedId)
  const host = inst?.config.host || '127.0.0.1'
  const port = inst?.config.port || 8080

  useEffect(() => {
    if (running.length > 0 && (!selectedId || !running.find(item => item.id === selectedId))) {
      setSelectedId(running[0].id)
    }
  }, [running, selectedId])

  useEffect(() => {
    if (!selectedId || !inst) {
      return
    }

    const fetchInitial = async () => {
      try {
        setSys(await invoke<SystemMetrics>('get_system_metrics', { instanceId: selectedId }))
      } catch {
        // ignore
      }

      try {
        setSlots(await invoke<SlotInfo[]>('get_slots', { host, port, apiKey: inst?.config.api_key || null }))
      } catch {
        setSlots([])
      }
    }

    void fetchInitial()

    const unlisten = listen<{ instanceId: string; system: SystemMetrics; ts: number }>('metrics-update', event => {
      const { instanceId, system } = event.payload
      if (instanceId !== selectedId) {
        return
      }
      setSys(system)
    })

    const slotsInterval = window.setInterval(async () => {
      try {
        setSlots(await invoke<SlotInfo[]>('get_slots', { host, port, apiKey: inst?.config.api_key || null }))
      } catch {
        setSlots([])
      }
    }, interval * 1000)

    return () => {
      unlisten.then(fn => fn())
      clearInterval(slotsInterval)
    }
  }, [selectedId, interval, host, port, inst?.config.api_key])

  const slotSummary = useMemo(() => {
    const busy = slots.filter(slot => slot.is_processing).length
    const cached = slots.filter(slot => !slot.is_processing && slot.n_ctx > 0).length
    const idle = slots.length - busy - cached
    return { busy, cached, idle }
  }, [slots])
  const labels = {
    subtitle: zh
      ? '\u5728\u5e94\u7528\u58f3\u5185\u76d1\u63a7\u7cfb\u7edf\u538b\u529b\u3001\u5b9e\u65f6 slots \u548c\u8fd1\u671f\u4efb\u52a1\u541e\u5410\u3002'
      : 'Watch system pressure, live slots, and recent task throughput without leaving the app shell.',
    realtimeMetrics: zh ? '\u5b9e\u65f6\u6307\u6807' : 'Realtime Metrics',
    realtimeMetricsDesc: zh ? '\u5728\u5bc6\u96c6\u6982\u89c8\u548c\u4eea\u8868\u68c0\u67e5\u4e4b\u95f4\u5207\u6362\u3002' : 'Switch between dense overview cards and gauge-first inspection.',
    overview: zh ? '\u6982\u89c8' : 'Overview',
    gauges: zh ? '\u4eea\u8868' : 'Gauges',
    runContext: zh ? '\u8fd0\u884c\u4e0a\u4e0b\u6587' : 'Run Context',
    runContextDesc: zh ? '\u5feb\u901f\u67e5\u770b\u5f53\u524d\u76d1\u63a7\u7684\u5b9e\u4f8b\u3002' : 'A quick read on the instance currently being monitored.',
    interval: zh ? '\u5237\u65b0\u95f4\u9694' : 'Interval',
    busySlots: zh ? '\u5fd9\u788c slots' : 'Busy slots',
    cachedSlots: zh ? '\u5df2\u7f13\u5b58 slots' : 'Cached slots',
    idleSlots: zh ? '\u7a7a\u95f2 slots' : 'Idle slots',
    cached: zh ? '\u5df2\u7f13\u5b58' : 'Cached',
    total: zh ? '\u603b\u91cf' : 'total',
    online: zh ? '\u5728\u7ebf' : 'online',
    devices: zh ? '\u8bbe\u5907' : 'devices',
  }

  if (!running.length) {
    return (
      <div className="flex-1 overflow-y-auto p-6">
        <EmptyState icon={<BarChart3 className="h-10 w-10" />} title={t.nav.perf} description={t.perfBlock.noRunning} />
      </div>
    )
  }

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="mb-6 flex flex-col gap-5 xl:flex-row xl:items-end xl:justify-between">
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <div className="rounded-2xl border border-violet-500/20 bg-violet-500/10 p-3 text-violet-300">
              <Gauge className="h-5 w-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-3xl font-semibold tracking-tight text-slate-50">{t.nav.perf}</h1>
                <Badge tone="slate">
                  {running.length} {t.nav.up}
                </Badge>
              </div>
              <p className="text-sm text-slate-400">{labels.subtitle}</p>
            </div>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <SelectInput
            value={selectedId}
            onChange={event => setSelectedId(event.target.value)}
            className="min-w-[180px]"
            data-guide="perf-select"
          >
            {running.map(instance => (
              <option key={instance.id} value={instance.id}>
                {instance.name}
              </option>
            ))}
          </SelectInput>
          <SelectInput
            value={interval}
            onChange={event => setIntervalState(Number(event.target.value))}
            className="min-w-[96px]"
          >
            <option value={2}>2s</option>
            <option value={5}>5s</option>
            <option value={10}>10s</option>
            <option value={30}>30s</option>
          </SelectInput>
        </div>
      </div>

      <div className="mb-6 grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          { label: t.perfBlock.cpu, value: sys ? `${Math.round(sys.cpu_percent ?? 0)}%` : '--', icon: Cpu, tone: 'text-sky-300 bg-sky-500/10 border-sky-500/20' },
          { label: t.perfBlock.memory, value: sys ? `${((sys.memory_mb ?? 0) / 1024).toFixed(1)} GB` : '--', icon: Activity, tone: 'text-purple-300 bg-purple-500/10 border-purple-500/20' },
          { label: t.perfBlock.gpu, value: sys ? `${Math.round(sys.gpu_percent ?? 0)}%` : '--', icon: Gauge, tone: 'text-emerald-300 bg-emerald-500/10 border-emerald-500/20' },
          { label: t.perfBlock.slotStatus, value: `${slotSummary.busy}/${slots.length}`, icon: Layers3, tone: 'text-amber-300 bg-amber-500/10 border-amber-500/20' },
        ].map(card => (
          <MetricCard key={card.label} label={card.label} value={card.value} icon={<card.icon className="h-5 w-5" />} tone={card.tone} />
        ))}
      </div>

      <div className="mb-6 grid gap-6 xl:grid-cols-[minmax(0,1fr),320px]">
        <Surface as="section" className="p-5">
          <div className="mb-5 flex items-center justify-between">
            <SectionHeader title={labels.realtimeMetrics} description={labels.realtimeMetricsDesc} />
            <div className="inline-flex rounded-xl border border-slate-700 bg-slate-950 p-1">
              <Button
                onClick={() => setViewMode('overview')}
                variant={viewMode === 'overview' ? 'primary' : 'subtle'}
                size="sm"
              >
                {labels.overview}
              </Button>
              <Button
                onClick={() => setViewMode('gauges')}
                variant={viewMode === 'gauges' ? 'primary' : 'subtle'}
                size="sm"
              >
                {labels.gauges}
              </Button>
            </div>
          </div>

          {viewMode === 'overview' ? (
            <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
              {[
                { label: t.perfBlock.cpu, value: sys ? `${Math.round(sys.cpu_percent ?? 0)}%` : t.perfBlock.waiting, detail: sys ? `${t.perfBlock.sysLabel} ${Math.round(sys.system_cpu_percent ?? 0)}%` : '' },
                { label: t.perfBlock.memory, value: sys ? `${((sys.memory_mb ?? 0) / 1024).toFixed(1)} GB` : t.perfBlock.waiting, detail: sys ? `${((sys.system_memory_used_mb ?? 0) / 1024).toFixed(1)} / ${((sys.system_memory_total_mb ?? 0) / 1024).toFixed(1)} GB` : '' },
                { label: t.perfBlock.gpu, value: sys ? `${Math.round(sys.gpu_percent ?? 0)}%` : t.perfBlock.waiting, detail: sys?.gpu_vendor || 'N/A' },
                { label: t.perfBlock.vram, value: sys ? `${((sys.vram_used_mb ?? 0) / 1024).toFixed(1)} GB` : t.perfBlock.waiting, detail: sys ? `${((sys.vram_total_mb ?? 0) / 1024).toFixed(1)} GB ${labels.total}` : '' },
              ].map(card => (
                <div key={card.label} className="rounded-2xl border border-slate-800 bg-slate-950/60 p-4">
                  <p className="text-sm text-slate-400">{card.label}</p>
                  <p className="mt-3 text-2xl font-semibold text-slate-50">{card.value}</p>
                  <p className="mt-2 text-xs text-slate-500">{card.detail}</p>
                </div>
              ))}
            </div>
          ) : (
            <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
              {sys ? (
                <>
                  <GaugeMeter label={t.perfBlock.cpu} value={sys.cpu_percent ?? 0} max={100} unit="%" color="blue" detail={`${t.perfBlock.sysLabel} ${(sys.system_cpu_percent ?? 0).toFixed(0)}%`} />
                  <GaugeMeter label={t.perfBlock.memory} value={(sys.memory_mb ?? 0) / 1024} max={(sys.system_memory_total_mb ?? 32000) / 1024} unit="GB" color="purple" detail={`${((sys.memory_mb ?? 0) / 1024).toFixed(1)} / ${((sys.system_memory_total_mb ?? 0) / 1024).toFixed(1)} GB`} />
                  <GaugeMeter label={t.perfBlock.gpu} value={sys.gpu_percent ?? 0} max={100} unit="%" color="emerald" detail={sys.gpu_vendor || 'N/A'} />
                  <GaugeMeter label={t.perfBlock.vram} value={(sys.vram_used_mb ?? 0) / 1024} max={(sys.vram_total_mb ?? 8192) / 1024} unit="GB" color="amber" detail={`${((sys.vram_used_mb ?? 0) / 1024).toFixed(1)} / ${((sys.vram_total_mb ?? 0) / 1024).toFixed(1)} GB`} />
                </>
              ) : (
                <div className="col-span-4 py-6 text-center text-sm text-slate-400">{t.perfBlock.waitingMetrics}</div>
              )}
            </div>
          )}
        </Surface>

        <Surface as="aside" className="p-5">
          <div className="mb-5">
            <SectionHeader title={labels.runContext} description={labels.runContextDesc} />
          </div>

          <div className="space-y-4">
            <InsetSurface className="p-4">
              <div className="flex items-start gap-3">
                <div className="rounded-2xl border border-slate-700 bg-slate-900 p-3 text-slate-300">
                  <Server className="h-5 w-5" />
                </div>
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium text-slate-100">{inst?.name}</p>
                  <p className="mt-1 text-xs text-slate-500">{host}:{port}</p>
                </div>
              </div>
            </InsetSurface>

            <InsetSurface className="space-y-3 p-4">
              {[
                [labels.interval, `${interval}s`],
                [labels.busySlots, String(slotSummary.busy)],
                [labels.cachedSlots, String(slotSummary.cached)],
                [labels.idleSlots, String(slotSummary.idle)],
                [t.perfBlock.uptime, sys ? formatSeconds(sys.uptime_secs) : '--'],
              ].map(([label, value]) => (
                <div key={label} className="flex items-center justify-between gap-3">
                  <span className="text-sm text-slate-500">{label}</span>
                  <span className="text-sm text-slate-200">{value}</span>
                </div>
              ))}
            </InsetSurface>

            <InsetSurface className="p-4">
              <p className="text-sm font-medium text-slate-100">{t.perfBlock.slotStatus}</p>
              <div className="mt-3 space-y-3">
                {slots.length === 0 ? (
                  <div className="py-4 text-center text-sm text-slate-500">{t.perfBlock.noSlots}</div>
                ) : (
                  slots.map((slot, index) => {
                    const usage = slot.is_processing ? 95 : slot.n_ctx > 0 ? 12 : 2
                    const label = slot.is_processing ? t.perfBlock.busy : slot.n_ctx > 0 ? labels.cached : t.perfBlock.idle
                    return (
                      <div key={index}>
                        <div className="mb-1 flex items-center justify-between text-xs text-slate-500">
                          <span>Slot {slot.id ?? index}</span>
                          <span>{label}</span>
                        </div>
                        <div className="h-2 rounded-full bg-slate-800">
                          <div
                            className={`h-2 rounded-full ${slot.is_processing ? 'bg-blue-500' : slot.n_ctx > 0 ? 'bg-emerald-500' : 'bg-slate-600'}`}
                            style={{ width: `${usage}%` }}
                          />
                        </div>
                      </div>
                    )
                  })
                )}
              </div>
            </InsetSurface>
          </div>
        </Surface>
      </div>

      <PerfAnalysis instanceId={selectedId} />

      <div className="mt-6">
        <ClusterThroughput t={t} labels={labels} />
      </div>
    </div>
  )
}

function ClusterThroughput({ t, labels }: { t: any; labels: { online: string; devices: string } }) {
  const [metrics, setMetrics] = useState<any>(null)

  useEffect(() => {
    const fetch = async () => {
      try {
        const result: any = await invoke('get_cluster_metrics')
        setMetrics(result)
      } catch {
        // ignore
      }
    }

    void fetch()
    const timer = setInterval(fetch, 5000)
    return () => clearInterval(timer)
  }, [])

  if (!metrics || metrics.total_workers === 0) {
    return null
  }

  return (
    <Surface className="overflow-hidden">
      <div className="flex items-center gap-2 border-b border-slate-800 bg-slate-950/90 px-5 py-3">
        <Activity className="h-4 w-4 text-sky-300" />
        <h2 className="text-sm font-medium text-slate-100">{t.clusterPage?.clusterThroughput || 'Cluster Throughput'}</h2>
        <span className="text-xs text-slate-500">{metrics.online_workers}/{metrics.total_workers} {labels.online}</span>
      </div>
      <div className="space-y-3 p-5">
        {metrics.worker_metrics?.map((worker: any, index: number) => (
          <div key={index} className="rounded-2xl border border-slate-800 bg-slate-950/50 p-4">
            <div className="flex items-center justify-between gap-4">
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <span className={`inline-block h-2 w-2 rounded-full ${worker.online ? 'bg-emerald-400' : 'bg-red-400'}`} />
                  <p className="truncate text-sm font-medium text-slate-100">{worker.name}</p>
                </div>
                <p className="mt-1 text-xs text-slate-500">{worker.host}:{worker.port}</p>
              </div>
              <div className="text-right text-xs text-slate-500">{(worker.devices || []).length} {labels.devices}</div>
            </div>
            {worker.devices?.length > 0 && (
              <div className="mt-3 flex flex-wrap gap-2">
                {worker.devices.map((device: any, deviceIndex: number) => (
                  <Badge key={deviceIndex} tone="slate">
                    {device.name} · {(device.free_mb / 1024).toFixed(0)} GB free
                  </Badge>
                ))}
              </div>
            )}
          </div>
        ))}
      </div>
    </Surface>
  )
}

function formatSeconds(seconds: number | null | undefined) {
  if (!seconds) {
    return '0s'
  }
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  const secs = Math.floor(seconds % 60)
  if (hours > 0) {
    return `${hours}h ${minutes}m`
  }
  if (minutes > 0) {
    return `${minutes}m ${secs}s`
  }
  return `${secs}s`
}
