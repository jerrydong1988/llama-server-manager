import { useState } from 'react'
import { ChevronDown, ChevronRight } from 'lucide-react'

export const cacheTypes = ['', 'f32', 'f16', 'bf16', 'q8_0', 'q4_0', 'q4_1', 'iq4_nl', 'q5_0', 'q5_1']
export const specTypes = ['', 'none', 'mtp', 'draft-mtp', 'draft-simple', 'draft-eagle3', 'ngram-cache', 'ngram-simple', 'ngram-map-k', 'ngram-map-k4v', 'ngram-mod']
export const chatTemplates = ['', 'bailing', 'chatglm3', 'chatglm4', 'chatml', 'command-r', 'deepseek', 'deepseek2', 'deepseek3', 'exaone3', 'gemma', 'gpt-oss', 'kimi-k2', 'llama2', 'llama3', 'llama4', 'mistral', 'openchat', 'phi3', 'phi4', 'vicuna', 'zephyr']

export const Section = ({ title, children, disabled, onToggle, toggled }: { title: string; children: React.ReactNode; disabled?: boolean; onToggle?: (v: boolean) => void; toggled?: boolean }) => {
  const [open, setOpen] = useState(false)
  return (
    <div className="border dark:border-gray-700 rounded-lg overflow-hidden">
      <button onClick={() => setOpen(!open)} className="w-full flex items-center gap-2 px-4 py-2.5 bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors text-left dark:text-gray-200">
        {open ? <ChevronDown className="w-4 h-4 text-gray-400 shrink-0" /> : <ChevronRight className="w-4 h-4 text-gray-400 shrink-0" />}
        <span className="text-sm font-medium">{title}</span>
        {disabled && <span className="text-xs text-gray-400 ml-1">{'\uD83D\uDED1'}</span>}
        {onToggle !== undefined && (
          <label className="ml-auto flex items-center gap-1 cursor-pointer shrink-0" onClick={e => e.stopPropagation()}>
            <span className="text-xs text-gray-400">{toggled ? 'On' : 'Off'}</span>
            <input type="checkbox" checked={toggled} onChange={e => { e.stopPropagation(); onToggle(e.target.checked) }} className="w-3.5 h-3.5 rounded" />
          </label>
        )}
      </button>
      {open && <div className={`px-4 py-3 space-y-3 ${disabled ? 'opacity-50 pointer-events-none' : ''}`}>{children}</div>}
    </div>
  )
}

export const Input = ({ label, value, onChange, placeholder, type, title, disabled }: { label: string; value: string; onChange: (v: string) => void; placeholder?: string; type?: string; title?: string; disabled?: boolean }) => (
  <div title={title}>
    <label className="block text-xs font-medium mb-1 text-gray-500">{label}</label>
    <input type={type || 'text'} value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder} disabled={disabled} className={`w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`} />
  </div>
)

export const Num = ({ label, value, onChange, min, max, step, title, disabled }: { label: string; value: number; onChange: (v: number) => void; min?: number; max?: number; step?: number; title?: string; disabled?: boolean }) => (
  <div title={title}>
    <label className="block text-xs font-medium mb-1 text-gray-500">{label}</label>
    <input type="number" value={value} min={min} max={max} step={step || 1} onChange={e => onChange(parseFloat(e.target.value) || 0)} disabled={disabled} className={`w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`} />
  </div>
)

export const Toggle = ({ label, value, onChange, title, disabled }: { label: string; value: boolean; onChange: (v: boolean) => void; title?: string; disabled?: boolean }) => (
  <label className={`flex items-center gap-2 ${disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}`} title={title}>
    <input type="checkbox" checked={value} onChange={e => onChange(e.target.checked)} disabled={disabled} className="w-4 h-4 rounded" />
    <span className="text-sm">{label}</span>
  </label>
)

export const Select = ({ label, value, onChange, options, title, disabled, defaultLabel }: { label: string; value: string; onChange: (v: string) => void; options: string[]; title?: string; disabled?: boolean; defaultLabel?: string }) => (
  <div title={title}>
    <label className="block text-xs font-medium mb-1 text-gray-500">{label}</label>
    <select value={value} onChange={e => onChange(e.target.value)} disabled={disabled} className={`w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}>
      {options.map(o => <option key={o} value={o}>{o || defaultLabel || '\u9ED8\u8BA4'}</option>)}
    </select>
  </div>
)

export const SubGroup = ({ label, toggled, onToggle, children }: { label: string; toggled: boolean; onToggle: (v: boolean) => void; children: React.ReactNode }) => (
  <div>
    <label className="flex items-center gap-2 cursor-pointer mb-2">
      <input type="checkbox" checked={toggled} onChange={e => onToggle(e.target.checked)} className="w-3.5 h-3.5 rounded" />
      <span className="text-xs font-medium text-gray-500">{label}</span>
    </label>
    {toggled && children}
  </div>
)
