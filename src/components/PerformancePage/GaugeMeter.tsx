export default function GaugeMeter({ label, value, max, unit, color, detail }: {
  label: string; value: number | null; max: number | null; unit: string
  color: 'blue' | 'purple' | 'emerald' | 'amber'; detail?: string
}) {
  const pct = value != null && max != null && max > 0 ? Math.min(100, (value / max) * 100) : 0
  const colorMap = { blue: 'stroke-blue-500', purple: 'stroke-purple-500', emerald: 'stroke-emerald-500', amber: 'stroke-amber-500' }
  const radius = 40; const circ = 2 * Math.PI * radius

  return (
    <div className="rounded-2xl border border-slate-800 bg-slate-950/60 p-4">
      <div className="flex flex-col items-center">
      <div className="relative w-24 h-24">
        <svg className="w-24 h-24 transform -rotate-90" viewBox="0 0 96 96">
          <circle cx="48" cy="48" r={radius} fill="none" stroke="currentColor" strokeWidth="8" className="text-slate-800" />
          <circle cx="48" cy="48" r={radius} fill="none" stroke="currentColor" strokeWidth="8" strokeLinecap="round"
            strokeDasharray={circ} strokeDashoffset={circ - (pct / 100) * circ}
            className={`${colorMap[color]} transition-all duration-700`} />
        </svg>
        <div className="absolute inset-0 flex flex-col items-center justify-center">
          <span className="text-xl font-bold text-slate-100">{value != null ? Math.round(value) : 0}</span>
          <span className="text-xs text-slate-500">{unit}</span>
        </div>
      </div>
      <p className="mt-3 text-sm font-medium text-slate-200">{label}</p>
      {detail && <p className="mt-1 text-center text-xs text-slate-500">{detail}</p>}
      </div>
    </div>
  )
}
