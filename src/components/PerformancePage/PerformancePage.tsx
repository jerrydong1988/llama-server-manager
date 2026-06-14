import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useAppStore } from '../../store'
import { useI18n } from '../../i18n'
import PerfAnalysis from './PerfAnalysis'

interface SystemMetrics {
  cpu_percent: number
  memory_mb: number
  uptime_secs: number
  gpu_percent: number | null
  vram_used_mb: number | null
  vram_total_mb: number | null
  system_cpu_percent: number | null
  system_memory_total_mb: number | null
  system_memory_used_mb: number | null
  gpu_vendor: string | null
}

interface SlotInfo {
  id: number
  is_processing: boolean
  n_ctx: number
}

export default function PerformancePage() {
  const { t } = useI18n()
  const { instances } = useAppStore()
  const running = instances.filter(i => i.status === 'running')
  const [selectedId, setSelectedId] = useState('')
  const [interval, setIntervalState] = useState(5)
  const [sys, setSys] = useState<SystemMetrics | null>(null)
  const [slots, setSlots] = useState<SlotInfo[]>([])

  const inst = instances.find(i => i.id === selectedId)
  const host = inst?.config.host || '127.0.0.1'
  const port = inst?.config.port || 8080

  useEffect(() => {
    if (running.length > 0 && (!selectedId || !running.find(r => r.id === selectedId))) {
      setSelectedId(running[0].id)
    }
  }, [running, selectedId])

  useEffect(() => {
    if (!selectedId || !inst) return

    // 立即获取初始数据
    const fetchInitial = async () => {
      try { setSys(await invoke<SystemMetrics>('get_system_metrics', { instanceId: selectedId })) } catch { /* stopped */ }
      try { setSlots(await invoke<SlotInfo[]>('get_slots', { host, port, apiKey: inst?.config.api_key || null })) } catch { setSlots([]) }
    }
    fetchInitial()

    // P1: 监听后台推送的指标事件，替代轮询
    const unlisten = listen<{
      instanceId: string
      system: SystemMetrics
      ts: number
    }>('metrics-update', (event) => {
      const { instanceId: evId, system } = event.payload
      if (evId !== selectedId) return // 只处理当前选中实例的事件
      setSys(system)
      // Slots 仍然用轮询（llama-server 不推送 slots 变更）
    })

    // Slots 轮询（轻量，单独处理）
    const slotsInterval = window.setInterval(async () => {
      try { setSlots(await invoke<SlotInfo[]>('get_slots', { host, port, apiKey: inst?.config.api_key || null })) } catch { setSlots([]) }
    }, interval * 1000)

    return () => {
      unlisten.then(fn => fn())
      clearInterval(slotsInterval)
    }
  }, [selectedId, interval, host, port])

  const fmtSecs = (s: number) => {
    if (s < 60) return `${s}s`
    if (s < 3600) return `${Math.floor(s / 60)}m${s % 60}s`
    const h = Math.floor(s / 3600)
    const m = Math.floor((s % 3600) / 60)
    return `${h}h${m}m`
  }

  if (!running.length) {
    return (
      <div className="flex-1 p-6">
        <h2 className="text-2xl font-bold mb-6">{t.nav.perf}</h2>
        <div className="text-center text-gray-500 py-12">{t.perfBlock.noRunning}</div>
      </div>
    )
  }

  return (
    <div className="flex-1 p-6 overflow-y-auto">
      <div className="flex items-center justify-between mb-6">
        <h2 className="text-2xl font-bold">{t.nav.perf}</h2>
        <div className="flex items-center gap-4">
          <select value={selectedId} onChange={e => setSelectedId(e.target.value)}
            className="px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900">
            {running.map(i => <option key={i.id} value={i.id}>{i.name}</option>)}
          </select>
          <select value={interval} onChange={e => setIntervalState(Number(e.target.value))}
            className="px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900">
            <option value={2}>2s</option>
            <option value={5}>5s</option>
            <option value={10}>10s</option>
            <option value={30}>30s</option>
          </select>
        </div>
      </div>

      <div className="grid grid-cols-3 md:grid-cols-5 gap-3 mb-6">
        {sys ? (
          <>
            <Card label={t.perfBlock.cpu} value={`${sys.cpu_percent.toFixed(1)}${sys.system_cpu_percent != null ? ` / ${sys.system_cpu_percent.toFixed(1)}` : ''}`} unit="%" color="text-blue-500" />
            <Card label={t.perfBlock.memory} value={`${(sys.memory_mb / 1024).toFixed(2)}${sys.system_memory_used_mb != null ? ` / ${(sys.system_memory_used_mb / 1024).toFixed(2)}` : ''}`} unit="GB" color="text-purple-500" />
            {sys.vram_used_mb != null ? (
              <Card label={`${t.perfBlock.vram}${sys.gpu_vendor ? ` (${sys.gpu_vendor})` : ''}`} value={`${(sys.vram_used_mb / 1024).toFixed(2)} / ${(sys.vram_total_mb! / 1024).toFixed(2)}`} unit="GB" color="text-green-500" />
            ) : (
              <Card label={t.perfBlock.vram} value="—" color="text-gray-400" />
            )}
            {sys.gpu_percent != null ? (
              <Card label={`GPU${sys.gpu_vendor ? ` (${sys.gpu_vendor})` : ''}`} value={sys.gpu_percent.toFixed(1)} unit="%" color="text-emerald-500" />
            ) : (
              <Card label="GPU" value="—" color="text-gray-400" />
            )}
            <Card label={t.perfBlock.uptime} value={fmtSecs(sys.uptime_secs)} color="text-gray-700 dark:text-gray-200" />
          </>
        ) : (
          <div className="col-span-5 text-center text-sm text-gray-400 py-4">{t.perfBlock.waiting}</div>
        )}
      </div>

      <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-4">
        <div className="text-xs text-gray-500 mb-3">{t.perfBlock.slotStatus}</div>
        {slots.length === 0 ? (
          <div className="text-sm text-gray-400 text-center py-2">{t.perfBlock.noSlots}</div>
        ) : (
          <div className="space-y-2">
            {slots.map((s, i) => (
              <div key={i} className="flex items-center gap-3 text-sm">
                <span className="text-gray-500 w-14 shrink-0">Slot {s.id ?? i}</span>
                <div className="flex-1 h-5 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden">
                  <div className={`h-full rounded-full transition-all ${s.is_processing ? 'bg-blue-500 w-[95%]' : (s.n_ctx > 0 ? 'bg-green-500 w-[5%]' : 'bg-gray-400 w-[2%]')}`} />
                </div>
                <span className={`text-xs w-12 text-right shrink-0 ${s.is_processing ? 'text-blue-500 font-medium' : 'text-gray-400'}`}>
                  {s.is_processing ? t.perfBlock.busy : t.perfBlock.idle}
                </span>
                <span className="text-xs text-gray-400 w-20 text-right shrink-0">
                  {s.n_ctx > 0 ? `${(s.n_ctx / 1024).toFixed(0)}K ctx` : ''}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>

      <PerfAnalysis instanceId={selectedId} />

      <div className="mt-4">
        <ClusterThroughput t={t} />
      </div>
    </div>
  )
}

function Card({ label, value, unit, color }: { label: string; value: string; unit?: string; color?: string }) {
  return (
    <div className="bg-gray-50 dark:bg-gray-800 rounded-lg p-3 text-center">
      <div className="text-xs text-gray-500 mb-1">{label}</div>
      <div className={`text-lg font-bold ${color || ''}`}>
        {value}{unit ? <span className="text-xs font-normal ml-0.5 text-gray-500">{unit}</span> : null}
      </div>
    </div>
  )
}

// ── Cluster Throughput Panel ──
function ClusterThroughput({ t }: { t: any }) {
  const [metrics, setMetrics] = useState<any>(null)

  useEffect(() => {
    const fetch = async () => {
      try {
        const m: any = await invoke('get_cluster_metrics')
        setMetrics(m)
      } catch {}
    }
    fetch()
    const interval = setInterval(fetch, 5000)
    return () => clearInterval(interval)
  }, [])

  if (!metrics || metrics.total_workers === 0) return null

  return (
    <div className="border dark:border-gray-700 rounded-lg overflow-hidden">
      <div className="px-4 py-2.5 bg-gray-100 dark:bg-gray-800 text-sm font-medium flex items-center gap-2">
        <span>🔗</span> {t.clusterPage?.clusterThroughput || 'Cluster Throughput'}
        <span className="text-xs text-gray-400 ml-2">{metrics.online_workers}/{metrics.total_workers} online</span>
      </div>
      <div className="p-4 space-y-2">
        {metrics.worker_metrics?.map((w: any, i: number) => (
          <div key={i} className="flex items-center gap-3 text-sm">
            <span className={`inline-block w-2 h-2 rounded-full ${w.online ? 'bg-green-500' : 'bg-red-500'}`} />
            <span className="flex-1">{w.name}</span>
            <span className="text-gray-400">{w.host}:{w.port}</span>
            {w.devices?.map((d: any, j: number) => (
              <span key={j} className="text-xs text-gray-400">{d.name} ({(d.free_mb/1024).toFixed(0)} GB free)</span>
            ))}
          </div>
        ))}
      </div>
    </div>
  )
}
