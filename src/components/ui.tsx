import type { ButtonHTMLAttributes, CSSProperties, InputHTMLAttributes, ReactNode, SelectHTMLAttributes } from 'react'

export const surfaceClassName = 'rounded-lg border border-slate-800/80 bg-slate-900/70 shadow-[0_16px_48px_rgba(15,23,42,0.28)] backdrop-blur'
export const insetSurfaceClassName = 'rounded-lg border border-slate-800 bg-slate-950/70'
export const controlClassName = 'rounded-lg border border-slate-700 bg-slate-950 text-sm text-slate-100 outline-none transition placeholder:text-slate-500 focus:border-blue-500 disabled:cursor-not-allowed disabled:opacity-50'

export function joinClassNames(...items: Array<string | false | null | undefined>) {
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

export function PageFrame({
  header,
  toolbar,
  inspector,
  children,
  className = '',
  contentClassName = '',
}: {
  header?: ReactNode
  toolbar?: ReactNode
  inspector?: ReactNode
  children: ReactNode
  className?: string
  contentClassName?: string
}) {
  return (
    <section className={joinClassNames('flex min-h-full min-w-0 flex-col gap-4', className)}>
      {header}
      {toolbar}
      <div className={joinClassNames('grid min-h-0 min-w-0 flex-1 gap-4 xl:grid-cols-[minmax(0,1fr)_320px]', !inspector && 'xl:block')}>
        <div className={joinClassNames('min-w-0', contentClassName)}>{children}</div>
        {inspector ? <div className="min-w-0">{inspector}</div> : null}
      </div>
    </section>
  )
}

export function PageHeader({
  eyebrow,
  title,
  description,
  meta,
  actions,
  className = '',
}: {
  eyebrow?: ReactNode
  title: ReactNode
  description?: ReactNode
  meta?: ReactNode
  actions?: ReactNode
  className?: string
}) {
  return (
    <div className={joinClassNames('flex min-w-0 flex-col gap-3 sm:flex-row sm:items-start sm:justify-between', className)}>
      <div className="min-w-0">
        {eyebrow ? <div className="mb-1 text-xs font-semibold uppercase text-slate-500 dark:text-slate-400">{eyebrow}</div> : null}
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <h2 className="truncate text-xl font-semibold leading-7 text-slate-950 dark:text-slate-50">{title}</h2>
          {meta}
        </div>
        {description ? <p className="mt-1 max-w-3xl text-sm leading-6 text-slate-500 dark:text-slate-400">{description}</p> : null}
      </div>
      {actions ? <div className="flex shrink-0 items-center gap-2">{actions}</div> : null}
    </div>
  )
}

export function PageToolbar({
  children,
  className = '',
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <div className={joinClassNames('flex min-h-12 min-w-0 flex-col gap-3 rounded-lg border border-slate-200 bg-white px-3 py-2 dark:border-slate-800 dark:bg-slate-900 sm:flex-row sm:items-center sm:justify-between', className)}>
      {children}
    </div>
  )
}

export const Toolbar = PageToolbar

function middleTruncate(value: string, maxLength: number) {
  if (value.length <= maxLength) return value
  const head = Math.max(12, Math.floor(maxLength * 0.42))
  const tail = Math.max(12, maxLength - head - 3)
  return `${value.slice(0, head)}...${value.slice(-tail)}`
}

export function PathText({
  value,
  multiline = false,
  maxLength = 72,
  actions,
  className = '',
}: {
  value: string
  multiline?: boolean
  maxLength?: number
  actions?: ReactNode
  className?: string
}) {
  const displayValue = multiline ? value : middleTruncate(value, maxLength)
  return (
    <span className={joinClassNames('flex min-w-0 items-center gap-2 font-mono text-[12px] leading-5', className)} title={value}>
      <span className={joinClassNames('min-w-0 flex-1', multiline ? 'whitespace-pre-wrap break-all' : 'truncate')}>{displayValue}</span>
      {actions ? <span className="shrink-0">{actions}</span> : null}
    </span>
  )
}

export function IconButton({
  icon,
  label,
  className = '',
  title,
  ...props
}: Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'children'> & {
  icon: ReactNode
  label: string
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={title || label}
      className={joinClassNames(
        'inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-slate-700 bg-slate-900 text-slate-200 transition hover:border-slate-600 hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    >
      {icon}
    </button>
  )
}

export function ActionGroup({
  primary,
  children,
  destructive,
  className = '',
}: {
  primary?: ReactNode
  children?: ReactNode
  destructive?: ReactNode
  className?: string
}) {
  return (
    <div className={joinClassNames('flex min-w-0 items-center justify-end gap-1.5', className)}>
      {primary ? <div className="shrink-0">{primary}</div> : null}
      {children ? <div className="flex shrink-0 items-center gap-1.5">{children}</div> : null}
      {destructive ? <div className="ml-1 flex shrink-0 items-center gap-1.5 border-l border-slate-700 pl-2">{destructive}</div> : null}
    </div>
  )
}

export function DataTableActionCell({ children }: { children: ReactNode }) {
  return <td className="w-0 whitespace-nowrap px-3 py-2 align-middle">{children}</td>
}

export type DataTableColumn<T> = {
  key: string
  header: ReactNode
  accessor?: keyof T
  render?: (row: T, index: number) => ReactNode
  className?: string
  headerClassName?: string
  width?: number | string
  minWidth?: number | string
  align?: 'left' | 'center' | 'right'
}

export function DataTable<T>({
  columns,
  rows,
  getRowKey,
  empty,
  selectedKey,
  onRowClick,
  className = '',
  density = 'default',
}: {
  columns: DataTableColumn<T>[]
  rows: T[]
  getRowKey: (row: T, index: number) => string
  empty?: ReactNode
  selectedKey?: string
  onRowClick?: (row: T, index: number) => void
  className?: string
  density?: 'compact' | 'default'
}) {
  const rowPadding = density === 'compact' ? 'px-3 py-2' : 'px-3 py-2.5'

  return (
    <div className={joinClassNames('min-w-0 overflow-hidden rounded-lg border border-slate-200 bg-white dark:border-slate-800 dark:bg-slate-900', className)}>
      <div className="overflow-x-auto">
        <table className="w-full min-w-full table-fixed border-collapse text-left text-sm">
          <thead className="border-b border-slate-200 bg-slate-50 text-xs uppercase text-slate-500 dark:border-slate-800 dark:bg-slate-950/70 dark:text-slate-400">
            <tr>
              {columns.map(column => (
                <th
                  key={column.key}
                  scope="col"
                  className={joinClassNames('h-10 whitespace-nowrap px-3 font-semibold', column.align === 'right' && 'text-right', column.align === 'center' && 'text-center', column.headerClassName)}
                  style={{ width: column.width, minWidth: column.minWidth }}
                >
                  {column.header}
                </th>
              ))}
            </tr>
          </thead>
          <tbody className="divide-y divide-slate-200 dark:divide-slate-800">
            {rows.length === 0 ? (
              <tr>
                <td colSpan={columns.length} className="px-4 py-10 text-center text-sm text-slate-500 dark:text-slate-400">
                  {empty || 'No data'}
                </td>
              </tr>
            ) : (
              rows.map((row, index) => {
                const rowKey = getRowKey(row, index)
                return (
                  <tr
                    key={rowKey}
                    onClick={onRowClick ? () => onRowClick(row, index) : undefined}
                    className={joinClassNames(
                      'h-12 transition',
                      onRowClick && 'cursor-pointer',
                      selectedKey === rowKey ? 'bg-blue-50/80 dark:bg-blue-950/30' : 'hover:bg-slate-50 dark:hover:bg-slate-800/50',
                    )}
                  >
                    {columns.map(column => {
                      const cell = column.render ? column.render(row, index) : column.accessor ? row[column.accessor] as ReactNode : null
                      return (
                        <td
                          key={column.key}
                          className={joinClassNames('min-w-0 align-middle text-slate-700 dark:text-slate-200', rowPadding, column.align === 'right' && 'text-right', column.align === 'center' && 'text-center', column.className)}
                        >
                          {cell}
                        </td>
                      )
                    })}
                  </tr>
                )
              })
            )}
          </tbody>
        </table>
      </div>
    </div>
  )
}

export function InspectorPanel({
  title,
  subtitle,
  actions,
  children,
  className = '',
}: {
  title: ReactNode
  subtitle?: ReactNode
  actions?: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <aside className={joinClassNames('sticky top-4 min-w-0 rounded-lg border border-slate-200 bg-white dark:border-slate-800 dark:bg-slate-900', className)}>
      <div className="border-b border-slate-200 px-4 py-3 dark:border-slate-800">
        <div className="min-w-0">
          <h3 className="truncate text-sm font-semibold text-slate-950 dark:text-slate-50">{title}</h3>
          {subtitle ? <div className="mt-1 truncate text-xs text-slate-500 dark:text-slate-400">{subtitle}</div> : null}
        </div>
        {actions ? <div className="mt-3">{actions}</div> : null}
      </div>
      <div className="space-y-3 p-4">{children}</div>
    </aside>
  )
}

export function DetailField({
  label,
  value,
  path = false,
}: {
  label: ReactNode
  value: ReactNode
  path?: boolean
}) {
  return (
    <div className="grid min-w-0 grid-cols-[104px_minmax(0,1fr)] gap-3 text-sm">
      <dt className="truncate text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{label}</dt>
      <dd className="min-w-0 text-slate-700 dark:text-slate-200">
        {path && typeof value === 'string' ? <PathText value={value} /> : value}
      </dd>
    </div>
  )
}

export function ResourceMeter({
  label,
  value,
  max = 100,
  unit = '%',
  tone = 'blue',
  description,
}: {
  label: ReactNode
  value: number
  max?: number
  unit?: string
  tone?: 'blue' | 'emerald' | 'amber' | 'red' | 'violet'
  description?: ReactNode
}) {
  const percent = max <= 0 ? 0 : Math.min(100, Math.max(0, (value / max) * 100))
  const meterColors = {
    blue: 'bg-blue-500',
    emerald: 'bg-emerald-500',
    amber: 'bg-amber-500',
    red: 'bg-red-500',
    violet: 'bg-violet-500',
  }

  return (
    <div className="min-w-0 rounded-lg border border-slate-200 bg-white p-3 dark:border-slate-800 dark:bg-slate-900">
      <div className="mb-2 flex min-w-0 items-center justify-between gap-3 text-sm">
        <span className="truncate font-medium text-slate-700 dark:text-slate-200">{label}</span>
        <span className="shrink-0 text-xs font-semibold text-slate-500 dark:text-slate-400">{Math.round(value)}{unit}</span>
      </div>
      <div className="h-2 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800">
        <div className={joinClassNames('h-full rounded-full transition-[width]', meterColors[tone])} style={{ width: `${percent}%` }} />
      </div>
      {description ? <div className="mt-2 truncate text-xs text-slate-500 dark:text-slate-400">{description}</div> : null}
    </div>
  )
}

export function MetricStrip({
  items,
  className = '',
}: {
  items: Array<{ label: ReactNode; value: ReactNode; detail?: ReactNode }>
  className?: string
}) {
  return (
    <div className={joinClassNames('grid min-w-0 gap-3 sm:grid-cols-2 xl:grid-cols-4', className)}>
      {items.map(item => (
        <div key={String(item.label)} className="min-w-0 rounded-lg border border-slate-200 bg-white p-3 dark:border-slate-800 dark:bg-slate-900">
          <div className="truncate text-xs font-medium uppercase text-slate-500 dark:text-slate-400">{item.label}</div>
          <div className="mt-2 truncate text-xl font-semibold text-slate-950 dark:text-slate-50">{item.value}</div>
          {item.detail ? <div className="mt-1 truncate text-xs text-slate-500 dark:text-slate-400">{item.detail}</div> : null}
        </div>
      ))}
    </div>
  )
}

export function EmptyPanel({
  title,
  description,
  action,
  className = '',
}: {
  title: ReactNode
  description?: ReactNode
  action?: ReactNode
  className?: string
}) {
  return (
    <div className={joinClassNames('flex min-h-[220px] flex-col items-center justify-center rounded-lg border border-dashed border-slate-300 bg-white px-6 py-8 text-center dark:border-slate-700 dark:bg-slate-900', className)}>
      <h3 className="text-base font-semibold text-slate-950 dark:text-slate-50">{title}</h3>
      {description ? <p className="mt-2 max-w-lg text-sm leading-6 text-slate-500 dark:text-slate-400">{description}</p> : null}
      {action ? <div className="mt-4">{action}</div> : null}
    </div>
  )
}

export function SegmentedControl<T extends string>({
  value,
  options,
  onChange,
  className = '',
}: {
  value: T
  options: Array<{ value: T; label: ReactNode }>
  onChange: (value: T) => void
  className?: string
}) {
  return (
    <div className={joinClassNames('inline-flex rounded-lg border border-slate-200 bg-slate-100 p-1 dark:border-slate-800 dark:bg-slate-950', className)}>
      {options.map(option => {
        const selected = option.value === value
        return (
          <button
            key={option.value}
            type="button"
            onClick={() => onChange(option.value)}
            className={joinClassNames(
              'h-8 rounded-md px-3 text-sm font-medium transition',
              selected ? 'bg-white text-slate-950 shadow-sm dark:bg-slate-800 dark:text-slate-50' : 'text-slate-500 hover:text-slate-950 dark:text-slate-400 dark:hover:text-slate-100',
            )}
          >
            {option.label}
          </button>
        )
      })}
    </div>
  )
}

export function StatusBadge({
  children,
  tone = 'slate',
  withDot = true,
  className = '',
}: {
  children: ReactNode
  tone?: 'slate' | 'blue' | 'emerald' | 'amber' | 'red' | 'violet'
  withDot?: boolean
  className?: string
}) {
  const dotColors = {
    slate: 'bg-slate-400',
    blue: 'bg-blue-400',
    emerald: 'bg-emerald-400',
    amber: 'bg-amber-400',
    red: 'bg-red-400',
    violet: 'bg-violet-400',
  }

  return (
    <Badge tone={tone} className={className}>
      {withDot ? <span className={joinClassNames('h-1.5 w-1.5 rounded-full', dotColors[tone])} /> : null}
      {children}
    </Badge>
  )
}

export function CommandBar({
  children,
  style,
  className = '',
}: {
  children: ReactNode
  style?: CSSProperties
  className?: string
}) {
  return (
    <div
      className={joinClassNames('flex min-w-0 items-center gap-2 rounded-lg border border-slate-200 bg-white px-3 py-2 dark:border-slate-800 dark:bg-slate-900', className)}
      style={style}
    >
      {children}
    </div>
  )
}
