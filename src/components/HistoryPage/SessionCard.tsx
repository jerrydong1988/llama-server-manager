import { ChevronDown, ChevronRight } from 'lucide-react'
import { type SessionMeta, type DataPoint } from './types'
import MiniChart from './MiniChart'

export default function SessionCard({
  session,
  data,
  loading,
  onToggleDetail,
  expanded,
  selected,
  onToggleSelect,
}: {
  session: SessionMeta
  data: DataPoint[] | null
  loading: boolean
  onToggleDetail: () => void
  expanded: boolean
  selected: boolean
  onToggleSelect: () => void
}) {
  const s = session.summary
  const fmtTime = (ts: number) => {
    const d = new Date(ts * 1000)
    return d.toLocaleString()
  }

  const fmtDuration = (secs: number | null | undefined) => {
    if (!secs) return ''
    if (secs < 60) return `${secs}s`
    if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`
    const h = Math.floor(secs / 3600)
    const m = Math.floor((secs % 3600) / 60)
    return `${h}h ${m}m`
  }

  const modelName = session.model_path.split(/[/\\]/).pop() || session.model_path

  return (
    <div className={`border rounded-lg transition-colors ${selected ? 'border-blue-500 bg-blue-50 dark:bg-blue-900/20' : 'border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-900'}`}>
      <div className="p-3">
        {/* Header row */}
        <div className="flex items-center gap-3">
          {/* Select checkbox */}
          <input type="checkbox" checked={selected} onChange={onToggleSelect}
            className="w-4 h-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500" />

          {/* Status dot */}
          <span className={`inline-block w-2 h-2 rounded-full shrink-0 ${session.ended_at ? 'bg-gray-400' : 'bg-green-500 animate-pulse'}`} />

          {/* Main info */}
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="font-medium text-sm truncate">{session.instance_name}</span>
              <span className="text-xs px-1.5 py-0.5 rounded bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400">{session.engine_backend}</span>
              {session.unclean && <span className="text-xs px-1.5 py-0.5 rounded bg-amber-100 dark:bg-amber-900/30 text-amber-700 dark:text-amber-400">unclean</span>}
            </div>
            <div className="text-xs text-gray-500 mt-0.5 truncate">{modelName}</div>
          </div>

          {/* Stats */}
          <div className="text-right shrink-0 space-y-0.5">
            {s?.avg_tps != null && <div className="text-sm font-bold text-orange-500">{s.avg_tps.toFixed(1)} <span className="text-xs font-normal text-gray-400">tok/s</span></div>}
            <div className="text-xs text-gray-400">{fmtDuration(session.duration_secs)}</div>
          </div>

          {/* Expand button */}
          <button onClick={onToggleDetail} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-800 rounded">
            {expanded ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
          </button>
        </div>

        {/* Config tags */}
        <div className="flex gap-1.5 mt-2 flex-wrap">
          <Tag label={`ctx=${session.config_snapshot.ctx_size}`} />
          <Tag label={`ngl=${session.config_snapshot.gpu_layers}`} />
          <Tag label={`b=${session.config_snapshot.batch_size}`} />
          <Tag label={`t=${session.config_snapshot.threads}`} />
          {session.config_snapshot.flash_attn && session.config_snapshot.flash_attn !== 'auto' && <Tag label={`fa=${session.config_snapshot.flash_attn}`} />}
          {session.config_snapshot.cont_batching && <Tag label="cb" />}
        </div>

        {/* Time row */}
        <div className="text-xs text-gray-400 mt-1.5">
          {fmtTime(session.started_at)}{session.ended_at ? ` → ${fmtTime(session.ended_at)}` : ' → running...'}
        </div>
      </div>

      {/* Expanded detail */}
      {expanded && (
        <div className="border-t border-gray-200 dark:border-gray-700 p-3 space-y-4">
          {/* Summary stats grid */}
          {s && (
            <div className="grid grid-cols-3 md:grid-cols-5 gap-2 text-xs">
              {s.avg_req_gen_tps != null && <Stat label="Gen t/s" value={s.avg_req_gen_tps.toFixed(1)} color="text-orange-500" />}
              {s.avg_req_prompt_tps != null && <Stat label="Prompt t/s" value={s.avg_req_prompt_tps.toFixed(1)} color="text-amber-500" />}
              {s.avg_tps != null && s.avg_req_gen_tps == null && <Stat label="Avg tok/s" value={s.avg_tps.toFixed(1)} color="text-orange-500" />}
              {s.peak_req_gen_tps != null && <Stat label="Peak Gen" value={s.peak_req_gen_tps.toFixed(1)} color="text-yellow-500" />}
              {s.peak_tps != null && s.peak_req_gen_tps == null && <Stat label="Peak tok/s" value={s.peak_tps.toFixed(1)} color="text-amber-500" />}
              {s.avg_spec_accept != null && <Stat label="Spec Accept" value={`${(s.avg_spec_accept * 100).toFixed(1)}%`} color="text-indigo-500" />}
              {s.request_count != null && <Stat label="Requests" value={fmtNum(s.request_count)} color="text-teal-500" />}
              {s.total_req_gen_tok != null && <Stat label="Gen tokens" value={fmtNum(s.total_req_gen_tok)} color="text-rose-500" />}
              {s.total_req_prompt_tok != null && <Stat label="Prompt tokens" value={fmtNum(s.total_req_prompt_tok)} color="text-cyan-500" />}
              {s.load_time_secs != null && <Stat label="Load time" value={`${s.load_time_secs.toFixed(1)}s`} color="text-gray-500" />}
              {s.avg_gpu_pct != null && <Stat label="Avg GPU" value={`${s.avg_gpu_pct.toFixed(1)}%`} color="text-emerald-500" />}
              {s.max_vram_mb != null && <Stat label="Max VRAM" value={`${(s.max_vram_mb / 1024).toFixed(1)} GB`} color="text-green-500" />}
              {s.avg_cpu_pct != null && <Stat label="Avg CPU" value={`${s.avg_cpu_pct.toFixed(1)}%`} color="text-blue-500" />}
              <Stat label="Data pts" value={`${s.data_points}`} color="text-gray-500" />
            </div>
          )}

          {/* Mini charts */}
          {loading && <div className="text-xs text-gray-400 text-center py-2">Loading chart data...</div>}
          {data && data.length >= 2 && (
            <div className="space-y-3">
              <MiniChart data={data} field="tps" label="Tokens/sec" color="#f97316" unit="t/s" />
              <MiniChart data={data} field="gpu" label="GPU %" color="#10b981" unit="%" />
              <MiniChart data={data} field="vram_u" label="VRAM" color="#22c55e" unit=" MB" />
            </div>
          )}
          {data && data.length < 2 && (
            <div className="text-xs text-gray-400 text-center py-2">Not enough data points for charts</div>
          )}
        </div>
      )}
    </div>
  )
}

function Tag({ label }: { label: string }) {
  return <span className="text-[10px] px-1.5 py-0.5 rounded bg-gray-100 dark:bg-gray-800 text-gray-500 dark:text-gray-400 font-mono">{label}</span>
}

function Stat({ label, value, color }: { label: string; value: string; color: string }) {
  return (
    <div className="text-center">
      <div className="text-gray-400">{label}</div>
      <div className={`font-bold ${color}`}>{value}</div>
    </div>
  )
}

function fmtNum(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`
  return n.toFixed(0)
}
