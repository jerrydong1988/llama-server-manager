import { type DataPoint } from './types'

// Simple SVG line chart for time-series data
export default function MiniChart({
  data,
  width = 300,
  height = 80,
  field,
  color = '#3b82f6',
  label = '',
  unit = '',
}: {
  data: DataPoint[]
  width?: number
  height?: number
  field: keyof DataPoint
  color?: string
  label?: string
  unit?: string
}) {
  const values = data.map(d => d[field] as number | null).filter((v): v is number => v != null && v > 0)
  if (values.length < 2) {
    return (
      <div className="text-xs text-gray-400 text-center py-2">
        {label ? `${label}: ` : ''}Not enough data
      </div>
    )
  }

  const min = Math.min(...values)
  const max = Math.max(...values)
  const range = max - min || 1
  const padding = { top: 10, right: 4, bottom: 16, left: 4 }
  const plotW = width - padding.left - padding.right
  const plotH = height - padding.top - padding.bottom

  const points = values.map((v, i) => {
    const x = padding.left + (i / (values.length - 1)) * plotW
    const y = padding.top + plotH - ((v - min) / range) * plotH
    return `${x},${y}`
  }).join(' ')

  return (
    <div className="w-full">
      {label && <div className="text-xs text-gray-500 mb-1">{label}</div>}
      <svg viewBox={`0 0 ${width} ${height}`} className="w-full" style={{ maxWidth: width }}>
        {/* Grid lines */}
        {[0, 0.5, 1].map(frac => {
          const y = padding.top + plotH * (1 - frac)
          return <line key={frac} x1={padding.left} y1={y} x2={width - padding.right} y2={y} stroke="#e5e7eb" strokeWidth="0.5" className="dark:stroke-gray-700" />
        })}
        {/* Line */}
        <polyline points={points} fill="none" stroke={color} strokeWidth="1.5" strokeLinejoin="round" />
        {/* Labels */}
        <text x={padding.left} y={padding.top + 4} className="fill-gray-500 dark:fill-gray-400" fontSize="8">{max.toFixed(1)}{unit}</text>
        <text x={padding.left} y={height - 2} className="fill-gray-400 dark:fill-gray-500" fontSize="8">{min.toFixed(1)}{unit}</text>
      </svg>
    </div>
  )
}
