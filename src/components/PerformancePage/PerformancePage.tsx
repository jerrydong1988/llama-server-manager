import { useState, useEffect, useRef } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useAppStore } from '../../store'
import { useI18n } from '../../i18n'

interface SystemMetrics {
  cpu_percent: number
  memory_mb: number
  uptime_secs: number
  gpu_percent: number | null
  vram_used_mb: number | null
  vram_total_mb: number | null
}

interface SlotInfo {
  id: number
  is_processing: boolean
  n_ctx: number
}

interface MetricsData {
  tokens_per_sec: number
  prompt_tokens: number
  gen_tokens: number
  requests: number
}

export default function PerformancePage() {
  const { t } = useI18n()
  const { instances } = useAppStore()
  const running = instances.filter(i => i.status === 'running')
  const [selectedId, setSelectedId] = useState('')
  const [interval, setIntervalState] = useState(5)
  const [sys, setSys] = useState<SystemMetrics | null>(null)
  const [slots, setSlots] = useState<SlotInfo[]>([])
  const [metrics, setMetrics] = useState<MetricsData | null>(null)
  const intervalRef = useRef<number | null>(null)

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

    const poll = async () => {
      try { setSys(await invoke<SystemMetrics>('get_system_metrics', { instanceId: selectedId })) } catch { /* stopped */ }
      try { setSlots(await invoke<SlotInfo[]>('get_slots', { host, port })) } catch { setSlots([]) }
      try {
        const m = await invoke<MetricsData | null>('get_metrics', { host, port })
        setMetrics(m)
      } catch { setMetrics(null) }
    }

    poll()
    intervalRef.current = window.setInterval(poll, interval * 1000)
    return () => { if (intervalRef.current) clearInterval(intervalRef.current) }
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
            <Card label={t.perfBlock.cpu} value={sys.cpu_percent.toFixed(1)} unit="%" color="text-blue-500" />
            <Card label={t.perfBlock.memory} value={sys.memory_mb.toFixed(0)} unit="MB" color="text-purple-500" />
            {sys.vram_used_mb != null ? (
              <Card label={t.perfBlock.vram} value={`${sys.vram_used_mb.toFixed(0)} / ${sys.vram_total_mb?.toFixed(0)}`} unit="MB" color="text-green-500" />
            ) : (
              <Card label={t.perfBlock.vram} value="—" color="text-gray-400" />
            )}
            {sys.gpu_percent != null ? (
              <Card label="GPU" value={sys.gpu_percent.toFixed(1)} unit="%" color="text-emerald-500" />
            ) : (
              <Card label="GPU" value="—" color="text-gray-400" />
            )}
            <Card label={t.perfBlock.uptime} value={fmtSecs(sys.uptime_secs)} color="text-gray-700 dark:text-gray-200" />
            <Card label={t.perfBlock.slots} value={`${slots.filter(s => s.is_processing).length}`} unit={`/ ${slots.length}`} color="text-indigo-500" />
          </>
        ) : (
          <div className="col-span-5 text-center text-sm text-gray-400 py-4">{t.perfBlock.waiting}</div>
        )}
      </div>

      {metrics ? (
        <div className="grid grid-cols-3 md:grid-cols-4 gap-3 mb-6">
          <Card label={t.perfBlock.tokensPerSec} value={metrics.tokens_per_sec.toFixed(1)} color="text-orange-500" />
          <Card label={t.perfBlock.requests} value={metrics.requests.toFixed(0)} color="text-teal-500" />
          <Card label="Prompt Tokens" value={metrics.prompt_tokens.toFixed(0)} color="text-cyan-500" />
          <Card label="Gen Tokens" value={metrics.gen_tokens.toFixed(0)} color="text-rose-500" />
        </div>
      ) : null}

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
