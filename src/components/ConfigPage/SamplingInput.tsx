import type { CSSProperties } from 'react'
import { TextInput } from '../ui'

/** The slider is an editing aid. Its display range must never clamp a saved value. */
export function SamplingInput({ label, value, onChange, min = 0, max, step = 1, sliderMax, disabled }: {
  label: string
  value: number
  onChange: (value: number) => void
  min?: number
  max?: number
  step?: number
  sliderMax: number
  disabled?: boolean
}) {
  const rangeMin = Math.min(min, value)
  const rangeMax = Math.max(sliderMax, value, rangeMin + step)
  const progress = Math.max(0, Math.min(100, (value - rangeMin) / (rangeMax - rangeMin) * 100))
  // Native range inputs round to their step, so a precise typed value (0.65,
  // for example) needs a matching step even if keyboard increments are 0.1.
  const [mantissa, exponent = '0'] = String(value).toLowerCase().split('e')
  const decimals = Math.max(0, (mantissa.split('.')[1]?.length ?? 0) - Number(exponent))
  const rangeStep = Math.min(step, 10 ** -decimals)

  return (
    <div className="grid grid-cols-[minmax(0,1fr)_88px] items-center gap-4">
      <TextInput
        aria-label={label}
        type="number"
        value={value}
        min={min}
        max={max}
        step={rangeStep}
        onChange={event => onChange(parseFloat(event.target.value) || 0)}
        disabled={disabled}
        className="col-start-2 row-start-1 h-9 font-mono text-right"
      />
      <input
        aria-label={label}
        type="range"
        value={value}
        min={rangeMin}
        max={rangeMax}
        step={rangeStep}
        onChange={event => onChange(Number(event.target.value))}
        disabled={disabled}
        className="ui-range col-start-1 row-start-1"
        style={{ '--range-progress': `${progress}%` } as CSSProperties}
      />
    </div>
  )
}
