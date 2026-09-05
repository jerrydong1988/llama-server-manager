import { useEffect, useRef, type ComponentType, type ReactNode } from 'react'
import { AlertTriangle, Download, Languages, LoaderCircle, Moon, RefreshCw, Search, Sun, Zap } from 'lucide-react'
import appIconUrl from '../../../src-tauri/icons/128x128.png'
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
  updateButtonTitle,
  onInstallUpdate,
  updateChecking,
  updateCheckError,
  updateCheckButtonTitle,
  onCheckUpdate,
  autoStartLabel,
  autoStartEnabled,
  onAutoStartChange,
  darkMode,
  onToggleDarkMode,
  darkModeTitle,
  languageLabel,
  languageTitle,
  onToggleLanguage,
  commandLabel,
  attentionLabel,
  attentionCount = 0,
  onOpenCommandCenter,
  wideContent = false,
  immersiveContent = false,
  constrainContent = false,
  children,
}: {
  appTitle: string
  version: string
  navigation: ShellNavigationItem[]
  activeId: string
  onNavigate: (id: string) => void
  pageDescription: string
  statusChips: ShellStatusChip[]
  updateInfo?: { latest_version: string; progress: number | null; busy: boolean } | null
  updateButtonTitle: string
  onInstallUpdate: () => void
  updateChecking: boolean
  updateCheckError?: string | null
  updateCheckButtonTitle: string
  onCheckUpdate: () => void
  autoStartLabel: string
  autoStartEnabled: boolean
  onAutoStartChange: (enabled: boolean) => void
  darkMode: boolean
  onToggleDarkMode: () => void
  darkModeTitle: string
  languageLabel: string
  languageTitle: string
  onToggleLanguage: () => void
  commandLabel: string
  attentionLabel: string
  attentionCount?: number
  onOpenCommandCenter: () => void
  wideContent?: boolean
  immersiveContent?: boolean
  constrainContent?: boolean
  children: ReactNode
}) {
  const activeItem = navigation.find(item => item.id === activeId) || navigation[0]
  const ActiveIcon = activeItem?.icon || Zap
  const runningChip = statusChips[0]
  const secondaryChips = statusChips.slice(1)
  const activeNavRef = useRef<HTMLButtonElement | null>(null)
  const topControlClassName = 'h-9 shrink-0 rounded-lg'
  const topStatusChipClassName = 'h-9 min-w-[76px] shrink-0 justify-center whitespace-nowrap rounded-lg px-3 text-[12px]'

  useEffect(() => {
    activeNavRef.current?.scrollIntoView({ block: 'nearest', inline: 'center' })
  }, [activeId])

  return (
    <div className={darkMode ? 'dark' : ''}>
      <div className="app-shell flex h-screen flex-col overflow-hidden bg-[var(--ui-canvas)] text-[var(--ui-text)] lg:flex-row">
        <aside className="app-sidebar flex shrink-0 flex-col border-b border-[var(--ui-line)] bg-[var(--ui-sidebar)] px-3 py-3 lg:h-screen lg:w-[216px] lg:border-b-0 lg:border-r">
          <div className="mb-3 flex items-center gap-2.5 px-1 lg:mb-6 lg:mt-2">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center overflow-hidden rounded-lg shadow-sm">
              <img aria-hidden="true" src={appIconUrl} alt="" className="h-10 w-10 max-w-none scale-[1.28]" />
            </div>
            <div className="min-w-0">
              <div className="truncate text-[12px] font-semibold leading-5" title={appTitle}>LlamaServerManager</div>
              <div className="font-mono text-[10px] text-[var(--ui-muted)]">Desktop Console v{version}</div>
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
                      aria-current={active ? 'page' : undefined}
                      data-nav-id={item.id}
                      onClick={() => onNavigate(item.id)}
                      className={joinClassNames(
                        'app-nav-item group flex h-9 w-full snap-start items-center gap-2.5 whitespace-nowrap rounded-lg px-2.5 text-[13px] transition',
                        active
                          ? 'bg-[var(--ui-control)] font-semibold text-[var(--ui-text)]'
                          : 'text-[var(--ui-secondary)] hover:bg-[var(--ui-soft)] hover:text-[var(--ui-text)]',
                      )}
                    >
                      <Icon className={joinClassNames("h-[18px] w-[18px] shrink-0", active && "text-[var(--ui-success)]")} />
                      <span className="min-w-0 flex-1 truncate text-left">{item.name}</span>
                      {item.badge != null && item.badge > 0 ? (
                        <span
                          className={joinClassNames(
                            'ui-chip min-w-5 justify-center px-1.5 py-0.5',
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

          <div className="ui-inset mt-3 hidden p-3 lg:mt-4 lg:block">
            <div className="mb-3 flex items-center justify-between gap-3 text-xs text-slate-500 dark:text-slate-400">
              <span className="truncate">{autoStartLabel}</span>
              <button
                type="button"
                role="switch"
                aria-checked={autoStartEnabled}
                aria-label={autoStartLabel}
                onClick={() => onAutoStartChange(!autoStartEnabled)}
                className="ui-switch"
                title={autoStartLabel}
              >
                <span className="ui-switch-thumb" />
              </button>
            </div>
            <div className="grid grid-cols-3 gap-2 text-center text-xs">
              {statusChips.map(chip => (
                <div key={String(chip.label)} className="min-w-0 rounded-lg bg-[var(--ui-soft)] px-1 py-2">
                  <div className="truncate font-semibold text-slate-950 dark:text-slate-50">{chip.value}</div>
                  <div className="mt-1 truncate text-slate-500 dark:text-slate-400">{chip.label}</div>
                </div>
              ))}
            </div>
          </div>
        </aside>

        <main data-page-id={activeId} className="app-main flex min-h-0 min-w-0 flex-1 flex-col">
          <header className="app-topbar z-20 shrink-0 bg-[var(--ui-canvas)]">
            <div className="flex min-h-16 flex-col gap-2 px-4 py-3 sm:px-6 xl:flex-row xl:items-center xl:justify-between">
              <div className="flex min-w-0 items-center gap-3">
                <div className="hidden h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-[var(--ui-soft)] text-[var(--ui-muted)] sm:flex">
                  <ActiveIcon className="h-5 w-5" />
                </div>
                <div className="min-w-0">
                  <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
                    <h1 className="truncate text-sm font-semibold leading-6">{activeItem?.name || appTitle}</h1>
                    {runningChip ? (
                      <Badge tone={runningChip.tone || 'emerald'} className="hidden sm:inline-flex">
                        <span className="font-semibold">{runningChip.value}</span>
                        <span>{runningChip.label}</span>
                      </Badge>
                    ) : null}
                  </div>
                  <p className="sr-only">{pageDescription}</p>
                </div>
              </div>

              <div className="flex min-w-0 flex-wrap items-center justify-between gap-2 xl:justify-end">
                <div className="hidden items-center gap-2 2xl:flex">
                  {secondaryChips.map(chip => (
                    <Badge key={String(chip.label)} tone={chip.tone || 'slate'} className={topStatusChipClassName}>
                      <span className="font-semibold">{chip.value}</span>
                      <span>{chip.label}</span>
                    </Badge>
                  ))}
                </div>

                <div className="flex min-w-0 flex-wrap items-center gap-2">
                  <IconButton
                    label={updateCheckButtonTitle}
                    title={updateCheckError ? `${updateCheckButtonTitle}: ${updateCheckError}` : updateCheckButtonTitle}
                    onClick={onCheckUpdate}
                    disabled={updateChecking || updateInfo?.busy}
                    icon={<RefreshCw className={joinClassNames('h-4 w-4', updateChecking ? 'animate-spin' : '')} />}
                    className={joinClassNames(
                      'w-9',
                      topControlClassName,
                      updateCheckError ? 'border-amber-400 text-amber-700 dark:border-amber-500/50 dark:text-amber-200' : '',
                    )}
                  />
                  {updateInfo ? (
                    <button
                      type="button"
                      onClick={onInstallUpdate}
                      disabled={updateInfo.busy || updateChecking}
                      title={updateButtonTitle}
                      aria-label={updateButtonTitle}
                      className={joinClassNames(
                        'inline-flex items-center gap-2 border border-emerald-500/20 bg-emerald-500/10 px-3 text-sm font-medium text-emerald-700 transition hover:bg-emerald-500/15 dark:text-emerald-200',
                        updateInfo.busy || updateChecking ? 'cursor-wait opacity-80' : '',
                        topControlClassName,
                      )}
                    >
                      <span className="max-w-[150px] truncate">
                        {updateInfo.busy && updateInfo.progress != null
                          ? `${updateInfo.progress}%`
                          : `v${updateInfo.latest_version}`}
                      </span>
                      {updateInfo.busy
                        ? <LoaderCircle className="h-4 w-4 shrink-0 animate-spin" />
                        : <Download className="h-4 w-4 shrink-0" />}
                    </button>
                  ) : null}
                  <IconButton
                    label={commandLabel}
                    title={commandLabel}
                    onClick={onOpenCommandCenter}
                    icon={<Search className="h-4 w-4" />}
                    className={joinClassNames('w-9', topControlClassName)}
                  />
                  <button
                    type="button"
                    onClick={onOpenCommandCenter}
                    title={attentionLabel}
                    aria-label={attentionLabel}
                    className={joinClassNames(
                      'inline-flex items-center justify-center gap-1.5 border px-2.5 text-xs font-semibold transition',
                      attentionCount > 0
                        ? 'border-amber-300 bg-amber-50 text-amber-700 hover:bg-amber-100 dark:border-amber-500/20 dark:bg-amber-500/10 dark:text-amber-200 dark:hover:bg-amber-500/15'
                        : 'border-slate-300 bg-white text-slate-600 hover:bg-slate-50 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-300 dark:hover:bg-slate-800',
                      topControlClassName,
                    )}
                  >
                    <AlertTriangle className="h-4 w-4 shrink-0" />
                    <span className="tabular-nums">{attentionCount}</span>
                  </button>
                  <Button
                    onClick={onToggleLanguage}
                    size="md"
                    title={languageTitle}
                    icon={<Languages className="h-4 w-4" />}
                    className={joinClassNames('min-w-[82px] px-3 text-[12px]', topControlClassName)}
                  >
                    {languageLabel}
                  </Button>
                  <IconButton
                    label={darkModeTitle}
                    title={darkModeTitle}
                    onClick={onToggleDarkMode}
                    icon={darkMode ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
                    className={joinClassNames('w-9', topControlClassName)}
                  />
                </div>
              </div>
            </div>
          </header>

          <div className={joinClassNames('app-content min-h-0 flex-1', constrainContent ? 'overflow-hidden' : 'overflow-y-auto')}>
            <div
              className={joinClassNames(
                constrainContent ? 'flex h-full min-h-0 flex-col' : 'min-h-full',
                immersiveContent ? '' : 'px-3 py-4 sm:px-6',
                !immersiveContent && !wideContent ? 'mx-auto w-full max-w-[1480px]' : '',
              )}
            >
              {children}
            </div>
          </div>

          {!immersiveContent && (
            <footer className="hidden h-8 shrink-0 items-center justify-between px-6 font-mono text-[10px] text-[var(--ui-muted)] sm:flex">
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
          )}
        </main>
      </div>
    </div>
  )
}
