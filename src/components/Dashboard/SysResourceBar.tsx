interface SystemMetrics {
  cpu_percent: number | null
  memory_mb: number | null
  gpu_percent: number | null
  vram_used_mb: number | null
  system_cpu_percent: number | null
  system_memory_total_mb: number | null
  gpu_vendor: string | null
  vram_total_mb: number | null
}

export default function SysResourceBar({ metrics }: { metrics: SystemMetrics | null }) {
  if (!metrics) {
    return (
      <div className="bg-white dark:bg-slate-800 rounded-xl border border-slate-200 dark:border-slate-700 p-5">
        <div className="text-sm text-slate-400 text-center py-4">等待系统指标...</div>
      </div>
    )
  }

  const items = [
    {
      label: 'CPU', pct: (metrics.cpu_percent ?? 0) / 100,
      detail: `进程 ${(metrics.cpu_percent ?? 0).toFixed(0)}% / 系统 ${(metrics.system_cpu_percent ?? 0).toFixed(0)}%`,
      color: 'bg-blue-500',
    },
    {
      label: '内存', pct: (metrics.memory_mb ?? 0) / ((metrics.system_memory_total_mb ?? 1) || 1),
      detail: `${((metrics.memory_mb ?? 0) / 1024).toFixed(1)} / ${((metrics.system_memory_total_mb ?? 0) / 1024).toFixed(1)} GB`,
      color: 'bg-purple-500',
    },
    {
      label: 'GPU', pct: (metrics.gpu_percent ?? 0) / 100,
      detail: metrics.gpu_vendor || 'N/A',
      color: 'bg-emerald-500',
    },
    {
      label: '显存', pct: (metrics.vram_used_mb ?? 0) / ((metrics.vram_total_mb ?? 1) || 1),
      detail: `${((metrics.vram_used_mb ?? 0) / 1024).toFixed(1)} / ${((metrics.vram_total_mb ?? 0) / 1024).toFixed(1)} GB`,
      color: 'bg-amber-500',
    },
  ]

  return (
    <div className="bg-white dark:bg-slate-800 rounded-xl border border-slate-200 dark:border-slate-700 p-5 space-y-3">
      {items.map(item => (
        <div key={item.label} className="flex items-center gap-3">
          <span className="w-10 text-xs font-medium text-slate-500 dark:text-slate-400 shrink-0">{item.label}</span>
          <div className="flex-1 h-2 bg-slate-200 dark:bg-slate-700 rounded-full overflow-hidden">
            <div className={`h-full rounded-full ${item.color} transition-all duration-700`}
              style={{ width: `${Math.min(100, item.pct * 100).toFixed(0)}%` }} />
          </div>
          <span className="text-xs text-slate-400 w-64 text-right shrink-0">{item.detail}</span>
        </div>
      ))}
    </div>
  )
}
