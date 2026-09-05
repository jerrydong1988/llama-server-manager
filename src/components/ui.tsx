import type { ButtonHTMLAttributes, CSSProperties, HTMLAttributes, InputHTMLAttributes, ReactNode, SelectHTMLAttributes } from 'react'
import { formatPathForDisplay } from '../utils/path'

// Content panels are intentionally opaque. Applying backdrop-filter to every panel
// creates many compositor layers and can leave off-screen panels temporarily
// unrasterized after a long scroll in WebView2.
export const surfaceClassName = 'ui-panel'
export const insetSurfaceClassName = 'ui-inset'
export const controlClassName = 'ui-control'

export function joinClassNames(...items: Array<string | false | null | undefined>) {
  return items.filter(Boolean).join(' ')
}

export function Surface({
  as = 'div',
  className = '',
  children,
  ...elementProps
}: {
  as?: 'div' | 'section' | 'aside'
  className?: string
  children: ReactNode
} & Omit<HTMLAttributes<HTMLElement>, 'children' | 'className'>) {
  const Component = as
  return <Component {...elementProps} className={`${surfaceClassName} ${className}`}>{children}</Component>
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
  valueClassName = 'ui-metric-value',
}: {
  label: string
  value: ReactNode
  icon?: ReactNode
  tone?: string
  valueClassName?: string
}) {
  return (
    <Surface className="ui-metric min-w-0 p-4">
      <div className="flex items-start justify-between">
        <div className="min-w-0">
          <p className="text-sm text-slate-600 dark:text-slate-400">{label}</p>
          <p className={`mt-2 truncate text-[var(--ui-text)] ${valueClassName}`} title={typeof value === 'string' ? value : undefined}>
            {value}
          </p>
        </div>
        {icon ? (
          <div className={`shrink-0 rounded-lg p-2 ${tone || 'border-slate-200 bg-slate-100 text-slate-600 dark:border-slate-700 dark:bg-slate-800 dark:text-slate-300'}`}>
            {icon}
          </div>
        ) : null}
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
        <h2 className="text-base font-semibold text-[var(--ui-text)]">{title}</h2>
        {description ? <p className="mt-1 text-xs leading-5 text-[var(--ui-muted)]">{description}</p> : null}
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
      <div className="mb-4 rounded-lg border border-slate-200 bg-slate-50 p-4 text-slate-500 dark:border-slate-700 dark:bg-slate-950/70 dark:text-slate-300">{icon}</div>
      <h2 className="text-2xl font-semibold text-slate-950 dark:text-slate-50">{title}</h2>
      {description ? <p className="mt-3 max-w-2xl text-sm leading-6 text-slate-500 dark:text-slate-400">{description}</p> : null}
    </Surface>
  )
}

const buttonVariants = {
  primary: 'ui-button-primary', secondary: '', subtle: 'ui-button-subtle',
  danger: 'ui-button-danger', success: 'ui-button-success',
  cyan: 'ui-button-cyan', violet: 'ui-button-violet',
} as const

const buttonSizes = {
  sm: 'min-h-[30px] gap-1.5 px-2.5 py-1 text-xs',
  md: 'min-h-9 gap-2 px-3.5 py-2 text-xs',
  lg: 'min-h-11 gap-2 px-5 py-3 text-sm',
  icon: 'h-9 w-9 justify-center p-0',
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
        'ui-button inline-flex items-center justify-center font-medium disabled:cursor-not-allowed disabled:opacity-50',
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
    slate: '', blue: 'ui-tone-blue', emerald: 'ui-tone-emerald',
    amber: 'ui-tone-amber', red: 'ui-tone-red', violet: 'ui-tone-violet',
  }

  return <span className={joinClassNames('ui-badge inline-flex items-center gap-1.5 px-2.5 py-1', tones[tone], className)}>{children}</span>
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
    return <input className={joinClassNames('h-9 w-full px-3', controlClassName, className)} {...props} />
  }

  return (
    <label className={joinClassNames('relative block', className)}>
      <span className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-slate-500">{leadingIcon}</span>
      <input className={joinClassNames('h-9 w-full pl-10 pr-3', controlClassName, inputClassName)} {...props} />
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
    <select className={joinClassNames('select-custom h-9 pl-3 pr-8', controlClassName, className)} {...props}>
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
          <h2 className="ui-page-heading break-words text-[var(--ui-text)]">{title}</h2>
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
  const readableValue = formatPathForDisplay(value)
  const displayValue = multiline ? readableValue : middleTruncate(readableValue, maxLength)
  return (
    <span className={joinClassNames('flex min-w-0 items-center gap-2 font-mono text-[12px] leading-5', className)} title={readableValue}>
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
        'ui-button inline-flex h-9 w-9 shrink-0 items-center justify-center disabled:cursor-not-allowed disabled:opacity-50',
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
      <div className="ui-meter-track">
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
    <div className={joinClassNames('ui-segmented', className)}>
      {options.map(option => {
        const selected = option.value === value
        return (
          <button
            key={option.value}
            type="button"
            onClick={() => onChange(option.value)}
            aria-pressed={selected}
            className={joinClassNames(
              'ui-segment',
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
