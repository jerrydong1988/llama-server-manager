import type { ButtonHTMLAttributes, CSSProperties, HTMLAttributes, InputHTMLAttributes, ReactNode, SelectHTMLAttributes } from 'react'
import { formatPathForDisplay } from '../utils/path'

// Opaque panels avoid extra compositor layers during long WebView2 scrolls.
export const surfaceClassName = 'console-surface'

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
  return <Component {...elementProps} className={joinClassNames(surfaceClassName, className)}>{children}</Component>
}

export function InsetSurface({
  className = '',
  children,
}: {
  className?: string
  children: ReactNode
}) {
  return <div className={joinClassNames('console-inset', className)}>{children}</div>
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
          <p className="text-sm text-ui-muted">{label}</p>
          <p className={`mt-3 truncate font-semibold text-ui-ink ${valueClassName}`} title={typeof value === 'string' ? value : undefined}>
            {value}
          </p>
        </div>
        {icon ? (
          <div className={`rounded-lg border p-3 ${tone || 'border-ui-line bg-ui-inset text-ui-body'}`}>
            {icon}
          </div>
        ) : (
          <div className={`rounded-lg border px-3 py-2 text-xs ${tone || 'border-ui-line bg-ui-inset text-ui-body'}`}>
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
        <h2 className="text-lg font-semibold text-ui-ink">{title}</h2>
        {description ? <p className="mt-1 text-sm leading-6 text-ui-muted">{description}</p> : null}
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
      <div className="mb-4 rounded-lg border p-4 border-ui-line bg-ui-inset text-ui-muted">{icon}</div>
      <h2 className="text-2xl font-semibold text-ui-ink">{title}</h2>
      {description ? <p className="mt-3 max-w-2xl text-sm leading-6 text-ui-muted">{description}</p> : null}
    </Surface>
  )
}

type ButtonVariant = 'primary' | 'secondary' | 'subtle' | 'danger' | 'success' | 'cyan' | 'violet'

const buttonSizes = {
  toolbar: 'console-toolbar-button',
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
  variant?: ButtonVariant
  size?: keyof typeof buttonSizes
}) {
  return (
    <button
      type={type}
      data-variant={variant}
      className={joinClassNames(
        'inline-flex items-center justify-center font-medium transition disabled:cursor-not-allowed disabled:opacity-50',
        'console-btn',
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
  return <span data-tone={tone} className={joinClassNames('inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs', 'console-badge', className)}>{children}</span>
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
    return <input className={joinClassNames('w-full min-w-0 max-w-full px-3 console-input h-9', className)} {...props} />
  }

  return (
    <label className={joinClassNames('relative block min-w-0', className)}>
      <span className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-ui-muted">{leadingIcon}</span>
      <input className={joinClassNames('w-full min-w-0 max-w-full pl-10 pr-3 console-input h-9', inputClassName)} {...props} />
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
    <select className={joinClassNames('select-custom min-w-0 max-w-full pl-3 pr-8', 'console-input h-9', className)} {...props}>
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
        {eyebrow ? <div className="mb-1 text-xs font-semibold uppercase text-ui-muted">{eyebrow}</div> : null}
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <h2 className="truncate text-xl font-semibold leading-7 text-ui-ink">{title}</h2>
          {meta}
        </div>
        {description ? <p className="mt-1 max-w-3xl text-sm leading-6 text-ui-muted">{description}</p> : null}
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
    <div className={joinClassNames('flex min-h-12 min-w-0 flex-col gap-3 rounded-lg border border-ui-line bg-ui-surface px-3 py-2 sm:flex-row sm:items-center sm:justify-between', className)}>
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
        'inline-flex h-10 w-10 shrink-0 items-center justify-center rounded-lg transition disabled:cursor-not-allowed disabled:opacity-50',
        'console-btn',
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
      {destructive ? <div className="ml-1 flex shrink-0 items-center gap-1.5 border-l border-ui-line pl-2">{destructive}</div> : null}
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
  // Fixed table layout ignores min-width on individual cells. Keep the table
  // wide enough for its declared columns and let its wrapper scroll instead.
  const minTableWidth = columns.reduce((total, column) => total + Math.max(
    typeof column.width === 'number' ? column.width : 0,
    typeof column.minWidth === 'number' ? column.minWidth : 0,
  ), 0)

  return (
    <div className={joinClassNames('min-w-0 overflow-hidden rounded-lg border border-ui-line bg-ui-surface', className)}>
      <div className="overflow-x-auto">
        <table className="w-full min-w-full table-fixed border-collapse text-left text-sm" style={{ minWidth: minTableWidth || undefined }}>
          <thead className="border-b border-ui-line bg-ui-inset text-xs uppercase text-ui-muted">
            <tr>
              {columns.map(column => (
                <th
                  key={column.key}
                  scope="col"
                  className={joinClassNames('h-10 whitespace-nowrap px-3 font-semibold', column.align === 'right' && 'text-right', column.align === 'center' && 'text-center', column.headerClassName)}
                  style={{ width: column.width ?? column.minWidth, minWidth: column.minWidth }}
                >
                  {column.header}
                </th>
              ))}
            </tr>
          </thead>
          <tbody className="divide-y divide-ui-line">
            {rows.length === 0 ? (
              <tr>
                <td colSpan={columns.length} className="px-4 py-10 text-center text-sm text-ui-muted">
                  {empty || 'No data'}
                </td>
              </tr>
            ) : (
              rows.map((row, index) => {
                const rowKey = getRowKey(row, index)
                return (
                  <tr
                    key={rowKey}
                    data-selected={selectedKey === rowKey}
                    onClick={onRowClick ? () => onRowClick(row, index) : undefined}
                    className={joinClassNames('console-list-row h-12', onRowClick && 'cursor-pointer')}
                  >
                    {columns.map(column => {
                      const cell = column.render ? column.render(row, index) : column.accessor ? row[column.accessor] as ReactNode : null
                      return (
                        <td
                          key={column.key}
                          className={joinClassNames('min-w-0 align-middle text-ui-body', rowPadding, column.align === 'right' && 'text-right', column.align === 'center' && 'text-center', column.className)}
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
    <aside className={joinClassNames('sticky top-4 min-w-0 rounded-lg border border-ui-line bg-ui-surface', className)}>
      <div className="border-b border-ui-line px-4 py-3">
        <div className="min-w-0">
          <h3 className="truncate text-sm font-semibold text-ui-ink">{title}</h3>
          {subtitle ? <div className="mt-1 truncate text-xs text-ui-muted">{subtitle}</div> : null}
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
      <dt className="truncate text-xs font-medium uppercase text-ui-muted">{label}</dt>
      <dd className="min-w-0 text-ui-body">
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
    <div className="min-w-0 rounded-lg border border-ui-line bg-ui-surface p-3">
      <div className="mb-2 flex min-w-0 items-center justify-between gap-3 text-sm">
        <span className="truncate font-medium text-ui-body">{label}</span>
        <span className="shrink-0 text-xs font-semibold text-ui-muted">{Math.round(value)}{unit}</span>
      </div>
      <div className="h-2 overflow-hidden rounded-full bg-ui-inset">
        <div className={joinClassNames('h-full rounded-full transition-[width]', meterColors[tone])} style={{ width: `${percent}%` }} />
      </div>
      {description ? <div className="mt-2 truncate text-xs text-ui-muted">{description}</div> : null}
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
        <div key={String(item.label)} className="min-w-0 rounded-lg border border-ui-line bg-ui-surface p-3">
          <div className="truncate text-xs font-medium uppercase text-ui-muted">{item.label}</div>
          <div className="mt-2 truncate text-xl font-semibold text-ui-ink">{item.value}</div>
          {item.detail ? <div className="mt-1 truncate text-xs text-ui-muted">{item.detail}</div> : null}
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
    <div className={joinClassNames('flex min-h-[220px] flex-col items-center justify-center rounded-lg border border-dashed border-ui-line bg-ui-surface px-6 py-8 text-center', className)}>
      <h3 className="text-base font-semibold text-ui-ink">{title}</h3>
      {description ? <p className="mt-2 max-w-lg text-sm leading-6 text-ui-muted">{description}</p> : null}
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
    <div className={joinClassNames('inline-flex rounded-lg border border-ui-line bg-ui-inset p-1', className)}>
      {options.map(option => {
        const selected = option.value === value
        return (
          <button
            key={option.value}
            type="button"
            aria-pressed={selected}
            onClick={() => onChange(option.value)}
            className={joinClassNames(
              'h-8 rounded-md px-3 text-sm font-medium transition',
              selected ? 'bg-ui-action text-ui-action-ink' : 'text-ui-muted hover:text-ui-ink',
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
    slate: 'bg-ui-control',
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
      className={joinClassNames('flex min-w-0 items-center gap-2 rounded-lg border border-ui-line bg-ui-surface px-3 py-2', className)}
      style={style}
    >
      {children}
    </div>
  )
}
