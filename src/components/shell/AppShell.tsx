import { useEffect, useRef, type ComponentType, type ReactNode } from 'react'
import { ArrowUpRight, Languages, Moon, Sun, Zap } from 'lucide-react'
import { Badge, Button, IconButton, joinClassNames } from '../ui'

type NavIcon = ComponentType<{ className?: string }>

export type ShellNavigationItem = {
  id: string
  name: string
  icon: NavIcon
  badge?: number
  separator?: boolean
}

export type ShellStatusChip = {
  label: ReactNode
  value: ReactNode
  tone?: 'slate' | 'blue' | 'emerald' | 'amber' | 'red' | 'violet'
}

export function AppShell({
  appTitle,
  version,
  navigation,
  activeId,
  onNavigate,
  pageDescription,
  statusChips,
  updateInfo,
  autoStartLabel,
  autoStartEnabled,
  onAutoStartChange,
  darkMode,
  onToggleDarkMode,
  darkModeTitle,
  languageLabel,
  languageTitle,
  onToggleLanguage,
  wideContent = false,
  children,
}: {
  appTitle: string
  version: string
  navigation: ShellNavigationItem[]
  activeId: string
  onNavigate: (id: string) => void
  pageDescription: string
  statusChips: ShellStatusChip[]
  updateInfo?: { latest_version: string; url: string } | null
  autoStartLabel: string
  autoStartEnabled: boolean
  onAutoStartChange: (enabled: boolean) => void
  darkMode: boolean
  onToggleDarkMode: () => void
  darkModeTitle: string
  languageLabel: string
  languageTitle: string
  onToggleLanguage: () => void
  wideContent?: boolean
  children: ReactNode
}) {
  const activeItem = navigation.find(item => item.id === activeId) || navigation[0]
  const ActiveIcon = activeItem?.icon || Zap
  const runningChip = statusChips[0]
  const secondaryChips = statusChips.slice(1)
  const activeNavRef = useRef<HTMLButtonElement | null>(null)

  useEffect(() => {
    activeNavRef.current?.scrollIntoView({ block: 'nearest', inline: 'center' })
  }, [activeId])

  return (
    <div className={darkMode ? 'dark' : ''}>
      <div className="app-shell flex h-screen flex-col overflow-hidden bg-[var(--color-app-bg)] text-slate-900 dark:bg-[var(--color-app-bg-dark)] dark:text-slate-100 lg:flex-row">
        <aside className="app-sidebar flex shrink-0 flex-col border-b border-slate-200 bg-white/95 px-3 py-3 shadow-sm shadow-slate-950/5 backdrop-blur dark:border-slate-800 dark:bg-slate-950/95 lg:h-screen lg:w-[264px] lg:border-b-0 lg:border-r">
          <div className="mb-3 flex items-center gap-3 px-2 lg:mb-5">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-slate-950 text-white shadow-sm dark:bg-slate-100 dark:text-slate-950">
              <Zap className="h-5 w-5" />
            </div>
            <div className="min-w-0">
              <div className="truncate text-sm font-semibold leading-5">{appTitle}</div>
              <div className="text-[11px] text-slate-500 dark:text-slate-400">Desktop Console v{version}</div>
            </div>
          </div>

          <nav className="min-h-0 snap-x overflow-x-auto overflow-y-hidden pb-1 lg:flex-1 lg:overflow-y-auto lg:pr-1">
            <div className="flex min-w-max gap-1 lg:block lg:min-w-0 lg:space-y-1">
              {navigation.map(item => {
                const Icon = item.icon
                const active = item.id === activeId
                return (
                  <div key={item.id} className="flex items-center lg:block">
                    {item.separator ? <div className="mx-2 h-8 border-l border-slate-200 dark:border-slate-800 lg:my-3 lg:h-auto lg:border-l-0 lg:border-t" /> : null}
                    <button
                      ref={active ? activeNavRef : undefined}
                      type="button"
                      onClick={() => onNavigate(item.id)}
                      className={joinClassNames(
                        'group flex h-9 w-full snap-start items-center gap-2.5 whitespace-nowrap rounded-lg px-2.5 text-sm transition lg:h-10',
                        active
                          ? 'bg-slate-950 text-white shadow-sm dark:bg-slate-100 dark:text-slate-950'
                          : 'text-slate-600 hover:bg-slate-100 hover:text-slate-950 dark:text-slate-300 dark:hover:bg-slate-900 dark:hover:text-white',
                      )}
                    >
                      <Icon className="h-4 w-4 shrink-0" />
                      <span className="min-w-0 flex-1 truncate text-left">{item.name}</span>
                      {item.badge != null && item.badge > 0 ? (
                        <span
                          className={joinClassNames(
                            'min-w-5 rounded-md px-1.5 py-0.5 text-center text-[11px] font-semibold',
                            active
                              ? 'bg-white/15 text-white dark:bg-slate-900/10 dark:text-slate-950'
                              : 'bg-blue-100 text-blue-700 dark:bg-blue-950/60 dark:text-blue-200',
                          )}
                        >
                          {item.badge}
                        </span>
                      ) : null}
                    </button>
                  </div>
                )
              })}
            </div>
          </nav>

          <div className="mt-3 hidden rounded-lg border border-slate-200 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-900/80 sm:block lg:mt-4">
            <div className="mb-3 flex items-center justify-between gap-3 text-xs text-slate-500 dark:text-slate-400">
              <span className="truncate">{autoStartLabel}</span>
              <button
                type="button"
                role="switch"
                aria-checked={autoStartEnabled}
                onClick={() => onAutoStartChange(!autoStartEnabled)}
                className={joinClassNames(
                  'relative inline-flex h-5 w-9 shrink-0 rounded-full border-2 border-transparent transition-colors',
                  autoStartEnabled ? 'bg-blue-600' : 'bg-slate-300 dark:bg-slate-700',
                )}
                title={autoStartLabel}
              >
                <span className={joinClassNames('pointer-events-none inline-block h-4 w-4 rounded-full bg-white shadow transition', autoStartEnabled ? 'translate-x-4' : 'translate-x-0')} />
              </button>
            </div>
            <div className="grid grid-cols-3 gap-2 text-center text-xs">
              {statusChips.map(chip => (
                <div key={String(chip.label)} className="min-w-0 rounded-lg border border-slate-200 bg-white px-2 py-2 dark:border-slate-800 dark:bg-slate-950">
                  <div className="truncate font-semibold text-slate-950 dark:text-slate-50">{chip.value}</div>
                  <div className="mt-1 truncate text-slate-500 dark:text-slate-400">{chip.label}</div>
                </div>
              ))}
            </div>
          </div>
        </aside>

        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          <header className="z-20 border-b border-slate-200 bg-[var(--color-app-panel)]/95 backdrop-blur dark:border-slate-800 dark:bg-[var(--color-app-panel-dark)]/95">
            <div className="flex min-h-[72px] flex-col gap-3 px-4 py-3 sm:px-5 lg:flex-row lg:items-center lg:justify-between">
              <div className="flex min-w-0 items-center gap-3">
                <div className="hidden h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-slate-200 bg-white text-slate-600 shadow-sm dark:border-slate-800 dark:bg-slate-900 dark:text-slate-300 sm:flex">
                  <ActiveIcon className="h-5 w-5" />
                </div>
                <div className="min-w-0">
                  <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
                    <h1 className="truncate text-lg font-semibold leading-7">{activeItem?.name || appTitle}</h1>
                    {runningChip ? (
                      <Badge tone={runningChip.tone || 'emerald'} className="hidden sm:inline-flex">
                        <span className="font-semibold">{runningChip.value}</span>
                        <span>{runningChip.label}</span>
                      </Badge>
                    ) : null}
                  </div>
                  <p className="mt-0.5 truncate text-sm text-slate-500 dark:text-slate-400">{pageDescription}</p>
                </div>
              </div>

              <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between lg:justify-end">
                <div className="flex flex-wrap items-center gap-2">
                  {secondaryChips.map(chip => (
                    <Badge key={String(chip.label)} tone={chip.tone || 'slate'}>
                      <span className="font-semibold">{chip.value}</span>
                      <span>{chip.label}</span>
                    </Badge>
                  ))}
                </div>

                <div className="flex items-center gap-2">
                  {updateInfo ? (
                    <a
                      href={updateInfo.url}
                      target="_blank"
                      rel="noreferrer"
                      className="inline-flex h-10 items-center gap-2 rounded-lg border border-emerald-500/20 bg-emerald-500/10 px-3 text-sm font-medium text-emerald-700 transition hover:bg-emerald-500/15 dark:text-emerald-200"
                    >
                      <span className="max-w-[150px] truncate">v{updateInfo.latest_version}</span>
                      <ArrowUpRight className="h-4 w-4 shrink-0" />
                    </a>
                  ) : null}
                  <Button
                    onClick={onToggleLanguage}
                    size="md"
                    title={languageTitle}
                    icon={<Languages className="h-4 w-4" />}
                    className="h-10 px-3"
                  >
                    {languageLabel}
                  </Button>
                  <IconButton
                    label={darkModeTitle}
                    title={darkModeTitle}
                    onClick={onToggleDarkMode}
                    icon={darkMode ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
                  />
                </div>
              </div>
            </div>
          </header>

          <div className="min-h-0 flex-1 overflow-y-auto">
            <div className={joinClassNames('min-h-full px-4 py-4 sm:px-5', wideContent ? '' : 'mx-auto w-full max-w-[1480px]')}>
              {children}
            </div>
          </div>

          <footer className="hidden h-9 shrink-0 items-center justify-between border-t border-slate-200 bg-white/85 px-4 text-xs text-slate-500 dark:border-slate-800 dark:bg-slate-950/85 dark:text-slate-400 sm:flex">
            <div className="flex min-w-0 items-center gap-3">
              <span className="truncate">{appTitle}</span>
              <span className="h-3 border-l border-slate-300 dark:border-slate-700" />
              <span className="truncate">{activeItem?.name}</span>
            </div>
            <div className="flex shrink-0 items-center gap-3">
              {statusChips.map(chip => (
                <span key={String(chip.label)} className="inline-flex items-center gap-1">
                  <span>{chip.label}</span>
                  <strong className="font-semibold text-slate-700 dark:text-slate-200">{chip.value}</strong>
                </span>
              ))}
            </div>
          </footer>
        </main>
      </div>
    </div>
  )
}
