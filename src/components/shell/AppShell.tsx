import { useEffect, useRef, type ComponentType, type ReactNode } from 'react'
import { AlertTriangle, Download, Languages, LoaderCircle, Moon, RefreshCw, Search, Sun, Menu } from 'lucide-react'
import appIconUrl from '../../../src-tauri/icons/128x128.png'
import { Button, IconButton, joinClassNames } from '../ui'
import { WorkspaceActionsProvider, WorkspaceFloatingActionsHost } from './WorkspaceActions'

type NavIcon = ComponentType<{ className?: string }>

export type ShellNavigationItem = {
  id: string
  name: string
  icon: NavIcon
  separator?: boolean
}

export type ShellStatusChip = {
  label: string
  value: number
}

export function AppShell({
  appTitle,
  version,
  navigation,
  navigationLabel,
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
  guidePanel,
  children,
}: {
  appTitle: string
  version: string
  navigationLabel: string
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
  guidePanel?: ReactNode
  children: ReactNode
}) {
  const activeItem = navigation.find(item => item.id === activeId) || navigation[0]
  const activeNavRef = useRef<HTMLButtonElement | null>(null)

  useEffect(() => {
    activeNavRef.current?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [activeId])

  return (
    <div className="app-shell shell-layout">
      <aside className="app-sidebar console-chrome shell-sidebar" aria-label={navigationLabel}>
        <div className="shell-brand">
          <img src={appIconUrl} alt="" />
          <div className="shell-brand-copy"><strong>{appTitle}</strong><small>v{version}</small></div>
        </div>
        <label className="shell-page-picker" title={navigationLabel}>
          <Menu size={17} aria-hidden="true" />
          <select aria-label={navigationLabel} value={activeId} onChange={event => onNavigate(event.target.value)}>
            {navigation.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
          </select>
        </label>
        <nav className="shell-navigation" aria-label={navigationLabel}>
          {navigation.map(item => {
            const Icon = item.icon
            const active = item.id === activeId
            return <div key={item.id}>
              {item.separator && <div className="shell-divider" aria-hidden="true" />}
              <button ref={active ? activeNavRef : undefined} type="button" className="shell-nav-item" data-nav-id={item.id}
                aria-current={active ? 'page' : undefined} aria-label={item.name} title={item.name} onClick={() => onNavigate(item.id)}>
                <Icon aria-hidden="true" /><span className="shell-nav-name">{item.name}</span>
              </button>
            </div>
          })}
        </nav>
        <div className="shell-autostart">
          <span>{autoStartLabel}</span>
          <button type="button" role="switch" className="console-switch" aria-label={autoStartLabel}
            aria-checked={autoStartEnabled} title={autoStartLabel} onClick={() => onAutoStartChange(!autoStartEnabled)} />
        </div>
      </aside>

      <main className="flex min-h-0 min-w-0 flex-1 flex-col">
        {!immersiveContent && <header className="shell-header console-chrome">
          <div className="shell-heading">
            <h1>{activeItem?.name || appTitle}</h1>
            {pageDescription && <p>{pageDescription}</p>}
          </div>
          <div className="shell-summary">
            {statusChips.map(chip => <span key={chip.label}><b>{chip.value}</b> {chip.label}</span>)}
          </div>
          <div className="shell-tools">
            <IconButton label={updateCheckButtonTitle}
              title={updateCheckError ? `${updateCheckButtonTitle}: ${updateCheckError}` : updateCheckButtonTitle}
              disabled={updateChecking || updateInfo?.busy} onClick={onCheckUpdate}
              icon={<RefreshCw size={15} className={updateChecking ? 'animate-spin' : undefined} />}
              className={joinClassNames('shell-icon', !!updateCheckError && 'has-alert')} />
            {updateInfo && <Button size="sm" variant="primary" onClick={onInstallUpdate} disabled={updateInfo.busy}
              title={updateButtonTitle} aria-label={updateButtonTitle}
              icon={updateInfo.busy ? <LoaderCircle size={15} className="animate-spin" /> : <Download size={15} />}>
              {updateInfo.busy && updateInfo.progress != null ? `${updateInfo.progress}%` : `v${updateInfo.latest_version}`}
            </Button>}
            <IconButton label={commandLabel} icon={<Search size={16} />} onClick={onOpenCommandCenter} className="shell-icon" />
            <button type="button" title={attentionLabel} aria-label={attentionLabel} onClick={onOpenCommandCenter}
              className={joinClassNames('console-btn shell-attention', attentionCount > 0 && 'has-alert')}>
              <AlertTriangle size={15} /><span>{attentionCount}</span>
            </button>
            <Button size="sm" onClick={onToggleLanguage} title={languageTitle} icon={<Languages size={15} />}>{languageLabel}</Button>
            <IconButton label={darkModeTitle} icon={darkMode ? <Sun size={16} /> : <Moon size={16} />} onClick={onToggleDarkMode} className="shell-icon" />
          </div>
        </header>}
        <WorkspaceActionsProvider>
          <div data-workspace-content className="relative flex min-h-0 flex-1 flex-col">
            <div data-workspace-scroll className={joinClassNames('console-page min-h-0 flex-1 bg-ui-canvas text-ui-ink', constrainContent ? 'overflow-hidden' : 'overflow-y-auto')}>
              <div className={joinClassNames(constrainContent ? 'flex h-full min-h-0 flex-col' : 'min-h-full',
                !immersiveContent && 'px-4 py-4 sm:px-5', !immersiveContent && !wideContent && 'mx-auto w-full max-w-[1480px]')}>
                {children}
              </div>
            </div>
            <WorkspaceFloatingActionsHost />
          </div>
          {guidePanel}
        </WorkspaceActionsProvider>
      </main>
    </div>
  )
}
