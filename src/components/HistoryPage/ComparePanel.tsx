import { type SessionMeta, type DataPoint } from './types'

export default function ComparePanel({
  sessionA,
  sessionB,
  onClear,
}: {
  sessionA: SessionMeta
  dataA: DataPoint[] | null
  sessionB: SessionMeta
  dataB: DataPoint[] | null
  onClear: () => void
}) {
  const sa = sessionA.summary
  const sb = sessionB.summary
  const ca = sessionA.config_snapshot
  const cb = sessionB.config_snapshot

  const diff = (a: number | null | undefined, b: number | null | undefined): string => {
    if (a == null || b == null || b === 0) return ''
    const d = ((a - b) / b * 100)
    if (Math.abs(d) < 1) return '≈'
    return d > 0 ? `+${d.toFixed(1)}%` : `${d.toFixed(1)}%`
  }

  const diffColor = (a: number | null | undefined, b: number | null | undefined, higherIsBetter = true): string => {
    if (a == null || b == null) return 'text-gray-400'
    const d = (a - b) / (b || 1)
    if (Math.abs(d) < 0.01) return 'text-gray-400'
    const better = higherIsBetter ? d > 0 : d < 0
    return better ? 'text-green-500' : 'text-red-500'
  }

  const modelNameA = sessionA.model_path.split(/[/\\]/).pop() || ''
  const modelNameB = sessionB.model_path.split(/[/\\]/).pop() || ''

  return (
    <div className="border-2 border-blue-300 dark:border-blue-700 rounded-lg overflow-hidden">
      <div className="px-4 py-2.5 bg-blue-50 dark:bg-blue-900/30 flex items-center justify-between">
        <span className="text-sm font-medium text-blue-700 dark:text-blue-300">Session Comparison</span>
        <button onClick={onClear} className="text-xs text-blue-500 hover:text-blue-700">Clear</button>
      </div>

      <div className="p-4">
        {/* Side-by-side headers */}
        <div className="grid grid-cols-2 gap-4 mb-4">
          <div>
            <div className="font-medium text-sm">{sessionA.instance_name}</div>
            <div className="text-xs text-gray-400">{modelNameA}</div>
            <div className="text-xs text-gray-400">{sessionA.engine_backend} · ctx={ca.ctx_size} · ngl={ca.gpu_layers}</div>
          </div>
          <div>
            <div className="font-medium text-sm">{sessionB.instance_name}</div>
            <div className="text-xs text-gray-400">{modelNameB}</div>
            <div className="text-xs text-gray-400">{sessionB.engine_backend} · ctx={cb.ctx_size} · ngl={cb.gpu_layers}</div>
          </div>
        </div>

        {/* Metric comparison table */}
        <table className="w-full text-xs">
          <thead>
            <tr className="text-gray-400 border-b dark:border-gray-700">
              <th className="text-left py-1.5 pr-2">Metric</th>
              <th className="text-right py-1.5 px-2 w-[30%]">Session A</th>
              <th className="text-right py-1.5 px-2 w-[30%]">Session B</th>
              <th className="text-right py-1.5 pl-2 w-[12%]">Δ</th>
            </tr>
          </thead>
          <tbody>
            <Row label="Avg tok/s" a={sa?.avg_tps} b={sb?.avg_tps} fmt={v => v.toFixed(1)} diff={diff} dc={diffColor} higherIsBetter />
            <Row label="Peak tok/s" a={sa?.peak_tps} b={sb?.peak_tps} fmt={v => v.toFixed(1)} diff={diff} dc={diffColor} higherIsBetter />
            <Row label="Avg GPU %" a={sa?.avg_gpu_pct} b={sb?.avg_gpu_pct} fmt={v => `${v.toFixed(1)}%`} diff={diff} dc={diffColor} higherIsBetter />
            <Row label="Max VRAM" a={sa?.max_vram_mb} b={sb?.max_vram_mb} fmt={v => `${(v / 1024).toFixed(1)} GB`} diff={diff} dc={() => 'text-gray-400'} />
            <Row label="Avg CPU %" a={sa?.avg_cpu_pct} b={sb?.avg_cpu_pct} fmt={v => `${v.toFixed(1)}%`} diff={diff} dc={diffColor} higherIsBetter={false} />
            <Row label="Gen tokens" a={sa?.total_gen_tok} b={sb?.total_gen_tok} fmt={v => fmtNum(v)} diff={diff} dc={() => 'text-gray-400'} />
            <Row label="Data pts" a={sa?.data_points} b={sb?.data_points} fmt={v => v.toString()} diff={diff} dc={() => 'text-gray-400'} />
          </tbody>
        </table>

        {/* Config diff */}
        <div className="mt-4 text-xs">
          <div className="text-gray-500 mb-1.5 font-medium">Config Diff</div>
          <div className="space-y-1">
            <ConfigDiff label="Context size" a={ca.ctx_size} b={cb.ctx_size} />
            <ConfigDiff label="GPU layers" a={ca.gpu_layers} b={cb.gpu_layers} />
            <ConfigDiff label="Batch size" a={ca.batch_size} b={cb.batch_size} />
            <ConfigDiff label="Threads" a={ca.threads} b={cb.threads} />
            <ConfigDiff label="Flash Attn" a={ca.flash_attn} b={cb.flash_attn} />
            <ConfigDiff label="Cont Batching" a={ca.cont_batching ? 'on' : 'off'} b={cb.cont_batching ? 'on' : 'off'} />
          </div>
        </div>
      </div>
    </div>
  )
}

function Row({
  label, a, b, fmt, diff, dc, higherIsBetter = true,
}: {
  label: string
  a: number | null | undefined
  b: number | null | undefined
  fmt: (v: number) => string
  diff: (a: number | null | undefined, b: number | null | undefined) => string
  dc: (a: number | null | undefined, b: number | null | undefined, h?: boolean) => string
  higherIsBetter?: boolean
}) {
  return (
    <tr className="border-b dark:border-gray-700/50">
      <td className="py-1.5 pr-2 text-gray-500">{label}</td>
      <td className="py-1.5 px-2 text-right font-mono font-medium">{a != null ? fmt(a) : '—'}</td>
      <td className="py-1.5 px-2 text-right font-mono">{b != null ? fmt(b) : '—'}</td>
      <td className={`py-1.5 pl-2 text-right font-mono ${dc(a, b, higherIsBetter)}`}>{diff(a, b)}</td>
    </tr>
  )
}

function ConfigDiff({ label, a, b }: { label: string; a: string | number; b: string | number }) {
  const same = String(a) === String(b)
  return (
    <div className="flex items-center gap-2">
      <span className="text-gray-400 w-24 shrink-0">{label}:</span>
      <span className={same ? 'text-gray-500' : 'text-amber-500 font-medium'}>{String(a)}</span>
      {!same && (
        <>
          <span className="text-gray-300">→</span>
          <span className="text-amber-600 font-medium">{String(b)}</span>
        </>
      )}
    </div>
  )
}

function fmtNum(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`
  return n.toFixed(0)
}
