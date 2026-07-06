import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode, SelectHTMLAttributes } from 'react'

export const surfaceClassName = 'rounded-lg border border-slate-800/80 bg-slate-900/70 shadow-[0_16px_48px_rgba(15,23,42,0.28)] backdrop-blur'
export const insetSurfaceClassName = 'rounded-lg border border-slate-800 bg-slate-950/70'
export const controlClassName = 'rounded-lg border border-slate-700 bg-slate-950 text-sm text-slate-100 outline-none transition placeholder:text-slate-500 focus:border-blue-500 disabled:cursor-not-allowed disabled:opacity-50'

function joinClassNames(...items: Array<string | false | null | undefined>) {
  return items.filter(Boolean).join(' ')
}

export function Surface({
  as = 'div',
  className = '',
  children,
}: {
  as?: 'div' | 'section' | 'aside'
  className?: string
  children: ReactNode
}) {
  const Component = as
  return <Component className={`${surfaceClassName} ${className}`}>{children}</Component>
}

export function InsetSurface({
  className = '',
  children,
}: {
  className?: string
  children: ReactNode
}) {
  return <div className={`${insetSurfaceClassName} ${className}`}>{children}</div>
}

export function MetricCard({
  label,
  value,
  icon,
  tone,
  valueClassName = 'text-3xl',
}: {
  label: string
  value: ReactNode
  icon?: ReactNode
  tone?: string
  valueClassName?: string
}) {
  return (
    <Surface className="p-5">
      <div className="flex items-start justify-between">
        <div className="min-w-0">
          <p className="text-sm text-slate-400">{label}</p>
          <p className={`mt-3 truncate font-semibold text-slate-50 ${valueClassName}`} title={typeof value === 'string' ? value : undefined}>
            {value}
          </p>
        </div>
        {icon ? (
          <div className={`rounded-lg border p-3 ${tone || 'border-slate-700 bg-slate-800 text-slate-300'}`}>
            {icon}
          </div>
        ) : (
          <div className={`rounded-lg border px-3 py-2 text-xs ${tone || 'border-slate-700 bg-slate-800 text-slate-300'}`}>
            {label}
          </div>
        )}
      </div>
    </Surface>
  )
}

export function SectionHeader({
  title,
  description,
  action,
}: {
  title: ReactNode
  description?: ReactNode
  action?: ReactNode
}) {
  return (
    <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
      <div className="min-w-0">
        <h2 className="text-lg font-semibold text-slate-50">{title}</h2>
        {description ? <p className="mt-1 text-sm leading-6 text-slate-400">{description}</p> : null}
      </div>
      {action ? <div className="shrink-0">{action}</div> : null}
    </div>
  )
}

export function EmptyState({
  icon,
  title,
  description,
  className = '',
}: {
  icon: ReactNode
  title: ReactNode
  description?: ReactNode
  className?: string
}) {
  return (
    <Surface className={`flex min-h-[360px] flex-col items-center justify-center p-10 text-center ${className}`}>
      <div className="mb-4 rounded-lg border border-slate-700 bg-slate-950/70 p-4 text-slate-300">{icon}</div>
      <h2 className="text-2xl font-semibold text-slate-50">{title}</h2>
      {description ? <p className="mt-3 max-w-2xl text-sm leading-6 text-slate-400">{description}</p> : null}
    </Surface>
  )
}

const buttonVariants = {
  primary: 'border border-blue-500/20 bg-blue-600 text-white hover:bg-blue-500',
  secondary: 'border border-slate-700 bg-slate-900 text-slate-200 hover:border-slate-600 hover:bg-slate-800',
  subtle: 'border border-transparent text-slate-400 hover:bg-slate-800 hover:text-white',
  danger: 'border border-red-500/20 bg-red-500/10 text-red-200 hover:bg-red-500/15',
  success: 'border border-emerald-500/20 bg-emerald-500/10 text-emerald-200 hover:bg-emerald-500/15',
  cyan: 'border border-cyan-500/20 bg-cyan-600 text-white hover:bg-cyan-500',
  violet: 'border border-violet-500/20 bg-violet-600 text-white hover:bg-violet-500',
} as const

const buttonSizes = {
  sm: 'gap-1 rounded-lg px-2.5 py-1.5 text-xs',
  md: 'gap-2 rounded-lg px-4 py-2.5 text-sm',
  lg: 'gap-2 rounded-lg px-5 py-3 text-sm',
  icon: 'h-10 w-10 justify-center rounded-lg p-0',
} as const

export function Button({
  children,
  icon,
  variant = 'secondary',
  size = 'md',
  className = '',
  type = 'button',
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  icon?: ReactNode
  variant?: keyof typeof buttonVariants
  size?: keyof typeof buttonSizes
}) {
  return (
    <button
      type={type}
      className={joinClassNames(
        'inline-flex items-center justify-center font-medium transition disabled:cursor-not-allowed disabled:opacity-50',
        buttonVariants[variant],
        buttonSizes[size],
        className,
      )}
      {...props}
    >
      {icon}
      {children}
    </button>
  )
}

export function Badge({
  children,
  tone = 'slate',
  className = '',
}: {
  children: ReactNode
  tone?: 'slate' | 'blue' | 'emerald' | 'amber' | 'red' | 'violet'
  className?: string
}) {
  const tones = {
    slate: 'border-slate-700 bg-slate-800 text-slate-300',
    blue: 'border-blue-500/20 bg-blue-500/10 text-blue-300',
    emerald: 'border-emerald-500/20 bg-emerald-500/10 text-emerald-300',
    amber: 'border-amber-500/20 bg-amber-500/10 text-amber-300',
    red: 'border-red-500/20 bg-red-500/10 text-red-300',
    violet: 'border-violet-500/20 bg-violet-500/10 text-violet-300',
  }

  return <span className={joinClassNames('inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs', tones[tone], className)}>{children}</span>
}

export function TextInput({
  leadingIcon,
  className = '',
  inputClassName = '',
  ...props
}: InputHTMLAttributes<HTMLInputElement> & {
  leadingIcon?: ReactNode
  inputClassName?: string
}) {
  if (!leadingIcon) {
    return <input className={joinClassNames('h-11 w-full px-3', controlClassName, className)} {...props} />
  }

  return (
    <label className={joinClassNames('relative block', className)}>
      <span className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-slate-500">{leadingIcon}</span>
      <input className={joinClassNames('h-11 w-full pl-10 pr-3', controlClassName, inputClassName)} {...props} />
    </label>
  )
}

export function SelectInput({
  children,
  className = '',
  ...props
}: SelectHTMLAttributes<HTMLSelectElement> & {
  children: ReactNode
}) {
  return (
    <select className={joinClassNames('select-custom h-11 pl-3 pr-8', controlClassName, className)} {...props}>
      {children}
    </select>
  )
}
