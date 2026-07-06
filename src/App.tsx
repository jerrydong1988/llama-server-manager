import { useState, useEffect, useMemo, useRef, lazy, Suspense, Component, ReactNode } from 'react'
import { Server, Database, Cpu, Terminal, Sun, Moon, Zap, Settings, Activity, Network, Download, BarChart3, BookOpen, ArrowUpRight, Languages } from 'lucide-react'
import { version } from '../package.json'
import { invoke } from '@tauri-apps/api/core'
import ModelRepo from './components/ModelRepo'
import EngineManager from './components/EngineManager'
import InstanceManager from './components/InstanceManager'
const LogsViewer = lazy(() => import('./components/LogsViewer'))
import ConfigPage from './components/ConfigPage'
const PerformancePage = lazy(() => import('./components/PerformancePage/PerformancePage'))
const ClusterPage = lazy(() => import('./components/ClusterPage/ClusterPage'))
const DownloadManager = lazy(() => import('./components/DownloadManager'))
import Dashboard from './components/Dashboard/Dashboard'
const GuidePage = lazy(() => import('./components/GuidePage'))
import { _startupTimings } from './store'
import { useAppStore } from './store'
import type { WorkerInfo } from './store'
import { I18nProvider, useI18n } from './i18n'
import { Badge, Button } from './components/ui'

class ErrorBoundary extends Component<{ children: ReactNode }, { hasError: boolean; error: Error | null }> {
  constructor(props: { children: ReactNode }) {
    super(props)
    this.state = { hasError: false, error: null }
  }

  static getDerivedStateFromError(error: Error) {
    return { hasError: true, error }
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="flex h-screen items-center justify-center bg-slate-100 px-6 text-slate-900 dark:bg-slate-950 dark:text-slate-100">
          <div className="w-full max-w-md rounded-lg border border-slate-200 bg-white p-6 shadow-sm dark:border-slate-800 dark:bg-slate-900">
            <div className="mb-4 flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-rose-100 text-rose-600 dark:bg-rose-950/50 dark:text-rose-300">
                <Terminal className="h-5 w-5" />
              </div>
              <div>
                <h2 className="text-lg font-semibold">Something went wrong</h2>
                <p className="text-sm text-slate-500 dark:text-slate-400">The current view crashed and can be reloaded.</p>
              </div>
            </div>
            <div className="mb-5 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-sm text-slate-600 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-300">
              {this.state.error?.message || 'Unknown error'}
            </div>
            <Button
              onClick={() => this.setState({ hasError: false, error: null })}
              variant="primary"
            >
              Reload view
            </Button>
          </div>
        </div>
      )
    }

    return this.props.children
  }
}

const TAB_CONTENT: Record<string, () => React.ReactElement> = {
  instances: () => <InstanceManager />,
  'model-repo': () => <ModelRepo />,
  dashboard: () => <Dashboard />,
  engine: () => <EngineManager />,
  config: () => <ConfigPage />,
  perf: () => <PerformancePage />,
  logs: () => <LogsViewer />,
  cluster: () => <ClusterPage />,
  downloads: () => <DownloadManager />,
  guide: () => <GuidePage />,
}

const renderTabContent = (tabId: string) => {
  const Renderer = TAB_CONTENT[tabId]
  return Renderer ? <Renderer /> : <div className="p-6 text-center text-slate-500">...</div>
}

function PageFallback({ label }: { label: string }) {
  return (
    <div className="flex min-h-[280px] items-center justify-center">
      <div className="inline-flex items-center gap-3 rounded-lg border border-slate-200 bg-white px-4 py-3 text-sm text-slate-500 shadow-sm dark:border-slate-800 dark:bg-slate-900 dark:text-slate-300">
        <div className="h-4 w-4 animate-spin rounded-full border-2 border-blue-500 border-t-transparent" />
        <span>{label}</span>
      </div>
    </div>
  )
}

function AppInner() {
  const activeTab = useAppStore(s => s.activeTab)
  const setActiveTab = useAppStore(s => s.setActiveTab)
  const loadConfig = useAppStore(s => s.loadConfig)
  const instances = useAppStore(s => s.instances)
  const engines = useAppStore(s => s.engines)
  const startInstance = useAppStore(s => s.startInstance)
  const darkMode = useAppStore(s => s.darkMode)
  const setDarkMode = useAppStore(s => s.setDarkMode)
  const { t, lang, setLang } = useI18n()
  const [updateInfo, setUpdateInfo] = useState<{ latest_version: string; url: string } | null>(null)
  const [autoStartEnabled, setAutoStartEnabled] = useState(false)
  const [autoStarted, setAutoStarted] = useState(false)

  useEffect(() => {
    invoke<boolean>('is_autostart_enabled').then(setAutoStartEnabled).catch(() => {})
  }, [])

  const upCount = useMemo(() => instances.filter(i => i.status === 'running').length, [instances])
  const downCount = useMemo(() => instances.filter(i => i.status !== 'running').length, [instances])
  const downloadTasks = useAppStore(s => s.downloadTasks)
  const activeDownloadCount = useMemo(
    () => Object.values(downloadTasks).filter(t => t.status === 'active' || t.status === 'paused' || t.status === 'pausing').length,
    [downloadTasks],
  )

  const navigation = useMemo(() => [
    { id: 'dashboard', name: t.nav.dashboard || 'Dashboard', icon: BarChart3 },
    { id: 'model-repo', name: t.nav.modelRepo, icon: Database },
    { id: 'downloads', name: t.nav.downloads || 'Downloads', icon: Download, badge: activeDownloadCount },
    { id: 'engine', name: t.nav.engine, icon: Cpu },
    { id: 'instances', name: t.nav.instances, icon: Server, badge: upCount },
    { id: 'config', name: t.nav.config, icon: Settings, separator: true },
    { id: 'cluster', name: t.nav.cluster, icon: Network },
    { id: 'perf', name: t.nav.perf, icon: Activity },
    { id: 'logs', name: t.nav.logs, icon: Terminal },
    { id: 'guide', name: lang === 'zh-CN' ? '\u4F7F\u7528\u8BF4\u660E' : 'Guide', icon: BookOpen, separator: true },
  ], [t, lang, upCount, activeDownloadCount])

  const mountTimeRef = useRef(performance.now())
  const [configLoaded, setConfigLoaded] = useState(false)
  const windowShownRef = useRef(false)

  useEffect(() => {
    _startupTimings.push({ name: 'app-mount', ms: Math.round(performance.now() - mountTimeRef.current) })
    if (!windowShownRef.current) {
      windowShownRef.current = true
      invoke('show_window').catch(() => {})
    }
    loadConfig().then(() => setConfigLoaded(true)).catch(() => setConfigLoaded(true))
  }, [loadConfig])

  useEffect(() => {
    if (autoStarted || instances.length === 0) return
    const toBoot = instances.filter(i => i.config.auto_start && i.status !== 'running')
    if (toBoot.length === 0) {
      setAutoStarted(true)
      return
    }
    if (engines.length === 0) return

    let cancelled = false
    const boot = async () => {
      const currentWorkers = useAppStore.getState().workers.length > 0
        ? useAppStore.getState().workers
        : await invoke<WorkerInfo[]>('get_workers').catch(() => [])

      for (const inst of toBoot) {
        if (cancelled) return
        if (inst.config.rpc_servers) {
          const configuredServers = inst.config.rpc_servers.split(/[, ]+/).filter(Boolean)
          const hasMatchingWorker = currentWorkers.some(w =>
            configuredServers.some(s => {
              const [h, p] = s.includes(':') ? s.split(':') : [s, '50052']
              return w.host === h && (w.port === parseInt(p) || w.port === 50052)
            }),
          )
          if (!hasMatchingWorker) {
            console.warn(`Instance "${inst.name}" requires a cluster worker but no matching worker is available; skipping auto-start`)
            continue
          }
        }

        try {
          await startInstance(inst.id)
        } catch {
          // ignore failed auto-start
        }
        await new Promise(resolve => setTimeout(resolve, 3000))
      }
      setAutoStarted(true)
    }

    boot()
    return () => { cancelled = true }
  }, [instances, engines, autoStarted, startInstance])

  useEffect(() => {
    const controller = new AbortController()
    fetch('https://api.github.com/repos/jerrydong1988/llama-server-manager/releases/latest', {
      headers: { Accept: 'application/vnd.github+json' },
      signal: controller.signal,
    })
      .then(r => r.json())
      .then(json => {
        const latest = (json.tag_name || '').replace(/^v/, '')
        const current = version
        const l = latest.split('.').map(Number)
        const c = current.split('.').map(Number)
        const maxLen = Math.max(l.length, c.length)
        let has = false
        for (let i = 0; i < maxLen; i++) {
          const lv = l[i] || 0
          const cv = c[i] || 0
          if (lv > cv) {
            has = true
            break
          }
          if (lv < cv) {
            has = false
            break
          }
        }
        if (has) setUpdateInfo({ latest_version: latest, url: json.html_url || '' })
      })
      .catch(() => {})
    return () => controller.abort()
  }, [])

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!e.ctrlKey) return
      if (e.key === 'Enter') {
        e.preventDefault()
        const { instances, startInstance, stopInstance } = useAppStore.getState()
        if (instances.length > 0) {
          const running = instances.find(i => i.status === 'running')
          const stopped = instances.find(i => i.status !== 'running')
          if (running) stopInstance(running.id)
          else if (stopped) startInstance(stopped.id)
        }
      }
      if (e.key === 's' || e.key === 'S') {
        e.preventDefault()
        useAppStore.getState().saveConfig()
      }
    }

    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])

  const activeNavItem = navigation.find(item => item.id === activeTab) || navigation[0]
  const ActivePageIcon = activeNavItem?.icon || BarChart3
  const layoutWide = activeTab === 'logs' || activeTab === 'guide'
  const statusChips = [
    { label: t.nav.up || 'up', value: upCount, tone: 'emerald' },
    { label: t.nav.down || 'down', value: downCount, tone: 'slate' },
    { label: t.nav.downloads || 'Downloads', value: activeDownloadCount, tone: 'blue' },
  ] as const
  const pageContext: Record<string, string> = {
    dashboard: 'System overview and current llama-server activity',
    'model-repo': 'Browse local and remote model assets',
    downloads: 'Model transfer queue and active tasks',
    engine: 'Runtime binaries and execution backends',
    instances: 'Server instances, ports, and process state',
    config: 'Application defaults and runtime preferences',
    cluster: 'Distributed workers and network availability',
    perf: 'Performance telemetry and resource signals',
    logs: 'Runtime logs and diagnostic output',
    guide: 'Reference material and operating notes',
  }

  return (
    <div className={darkMode ? 'dark' : ''}>
      <div className="flex h-screen flex-col overflow-hidden bg-slate-100 text-slate-900 dark:bg-slate-950 dark:text-slate-100 lg:flex-row">
        <aside className="flex shrink-0 flex-col border-b border-slate-200 bg-white/90 px-4 py-4 backdrop-blur dark:border-slate-800 dark:bg-slate-950/90 lg:h-screen lg:w-[272px] lg:border-b-0 lg:border-r">
          <div className="mb-4 flex items-center gap-3 px-2 lg:mb-5">
            <div className="flex h-11 w-11 items-center justify-center rounded-lg bg-blue-600 text-white shadow-sm shadow-blue-600/25">
              <Zap className="h-5 w-5" />
            </div>
            <div className="min-w-0">
              <div className="truncate text-sm font-semibold">{t.common.appTitle}</div>
              <div className="text-xs text-slate-500 dark:text-slate-400">v{version}</div>
            </div>
          </div>

          <nav className="overflow-x-auto overflow-y-hidden pb-1 lg:flex-1 lg:overflow-y-auto lg:pr-1">
            <div className="flex min-w-max gap-1 lg:block lg:min-w-0 lg:space-y-1">
              {navigation.map(item => {
                const Icon = item.icon
                const active = activeTab === item.id
                return (
                  <div key={item.id}>
                    {item.separator && <div className="mx-2 h-9 border-l border-slate-200 dark:border-slate-800 lg:my-3 lg:h-auto lg:border-l-0 lg:border-t" />}
                    <button
                      onClick={() => setActiveTab(item.id)}
                      className={`flex w-full items-center gap-3 whitespace-nowrap rounded-lg px-3 py-2.5 text-sm transition ${
                        active
                          ? 'bg-slate-900 text-white shadow-sm dark:bg-slate-100 dark:text-slate-900'
                          : 'text-slate-600 hover:bg-slate-100 hover:text-slate-900 dark:text-slate-300 dark:hover:bg-slate-900 dark:hover:text-white'
                      }`}
                    >
                      <Icon className="h-4 w-4 shrink-0" />
                      <span className="min-w-0 flex-1 truncate text-left">{item.name}</span>
                      {item.badge != null && item.badge > 0 && (
                        <span
                          className={`rounded-md px-1.5 py-0.5 text-[11px] font-medium ${
                            active
                              ? 'bg-white/15 text-white dark:bg-slate-900/10 dark:text-slate-900'
                              : 'bg-blue-100 text-blue-700 dark:bg-blue-950/50 dark:text-blue-300'
                          }`}
                        >
                          {item.badge}
                        </span>
                      )}
                    </button>
                  </div>
                )
              })}
            </div>
          </nav>

          <div className="mt-3 hidden rounded-lg border border-slate-200 bg-slate-50 p-3 dark:border-slate-800 dark:bg-slate-900 sm:block lg:mt-4">
            <div className="mb-3 flex items-center justify-between text-xs text-slate-500 dark:text-slate-400">
              <span>{t.common.autoStart || 'Auto Start'}</span>
              <button
                type="button"
                role="switch"
                aria-checked={autoStartEnabled}
                onClick={() => {
                  const next = !autoStartEnabled
                  setAutoStartEnabled(next)
                  ;(async () => {
                    try {
                      if (next) await invoke('enable_autostart')
                      else await invoke('disable_autostart')
                    } catch {
                      setAutoStartEnabled(!next)
                    }
                  })()
                }}
                className={`relative inline-flex h-5 w-9 shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ${autoStartEnabled ? 'bg-blue-600' : 'bg-slate-300 dark:bg-slate-700'}`}
                title={t.common.autoStart || 'Auto Start'}
              >
                <span className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200 ${autoStartEnabled ? 'translate-x-4' : 'translate-x-0'}`} />
              </button>
            </div>
            <div className="grid grid-cols-3 gap-2 text-center text-xs">
              <div className="rounded-lg bg-white px-2 py-2 dark:bg-slate-950">
                <div className="font-semibold text-emerald-600 dark:text-emerald-400">{upCount}</div>
                <div className="mt-1 text-slate-500 dark:text-slate-400">{t.nav.up || 'up'}</div>
              </div>
              <div className="rounded-lg bg-white px-2 py-2 dark:bg-slate-950">
                <div className="font-semibold text-slate-700 dark:text-slate-200">{downCount}</div>
                <div className="mt-1 text-slate-500 dark:text-slate-400">{t.nav.down || 'down'}</div>
              </div>
              <div className="rounded-lg bg-white px-2 py-2 dark:bg-slate-950">
                <div className="font-semibold text-blue-600 dark:text-blue-400">{activeDownloadCount}</div>
                <div className="mt-1 text-slate-500 dark:text-slate-400">{t.nav.downloads || 'Downloads'}</div>
              </div>
            </div>
          </div>
        </aside>

        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          <header className="sticky top-0 z-20 border-b border-slate-200 bg-slate-100/90 backdrop-blur dark:border-slate-800 dark:bg-slate-950/90">
            <div className="flex flex-col gap-3 px-4 py-3 sm:px-6 lg:flex-row lg:items-center lg:justify-between">
              <div className="flex min-w-0 items-start gap-3">
                <div className="mt-0.5 hidden h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-slate-200 bg-white text-slate-600 shadow-sm dark:border-slate-800 dark:bg-slate-900 dark:text-slate-300 sm:flex">
                  <ActivePageIcon className="h-5 w-5" />
                </div>
                <div className="min-w-0">
                  <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
                    <h1 className="truncate text-lg font-semibold leading-7">{activeNavItem?.name || t.common.appTitle}</h1>
                    <span className="rounded-md border border-slate-200 bg-white px-2 py-0.5 text-[11px] font-medium text-slate-500 dark:border-slate-800 dark:bg-slate-900 dark:text-slate-400">
                      {t.common.appTitle} v{version}
                    </span>
                  </div>
                  <p className="mt-0.5 truncate text-sm text-slate-500 dark:text-slate-400">
                    {pageContext[activeTab] || 'Manage local llama-server runtime state'}
                  </p>
                </div>
              </div>

              <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between lg:justify-end">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-xs font-medium uppercase tracking-wide text-slate-500 dark:text-slate-500">
                    Status
                  </span>
                  {statusChips.map(chip => (
                    <Badge key={chip.label} tone={chip.tone}>
                      <span className="font-semibold">{chip.value}</span>
                      <span>{chip.label}</span>
                    </Badge>
                  ))}
                </div>

                <div className="flex items-center gap-2">
                  {updateInfo && (
                    <a
                      href={updateInfo.url}
                      target="_blank"
                      rel="noreferrer"
                      className="inline-flex items-center gap-2 rounded-lg border border-emerald-500/20 bg-emerald-500/10 px-3 py-2 text-sm font-medium text-emerald-700 transition hover:bg-emerald-500/15 dark:text-emerald-200"
                    >
                      <span>{t.common.updateAvailable || 'New Version'} v{updateInfo.latest_version}</span>
                      <ArrowUpRight className="h-4 w-4" />
                    </a>
                  )}
                  <Button
                    onClick={() => setLang(lang === 'zh-CN' ? 'en-US' : 'zh-CN')}
                    size="md"
                    title={lang === 'zh-CN' ? 'Switch to English' : '\u5207\u6362\u4E3A\u4E2D\u6587'}
                    icon={<Languages className="h-4 w-4" />}
                  >
                    {lang === 'zh-CN' ? 'EN' : '\u4E2D\u6587'}
                  </Button>
                  <Button
                    onClick={() => setDarkMode(!darkMode)}
                    size="icon"
                    title={darkMode ? 'Switch to light mode' : 'Switch to dark mode'}
                  >
                    {darkMode ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
                  </Button>
                </div>
              </div>
            </div>
          </header>

          <div className="min-h-0 flex-1 overflow-y-auto">
            {!configLoaded ? (
              <div className="flex h-full items-center justify-center px-6">
                <PageFallback label={t.common?.loading || 'Loading...'} />
              </div>
            ) : (
              <div className={layoutWide ? 'px-4 py-4' : 'px-6 py-6'}>
                <div className={layoutWide ? '' : 'mx-auto max-w-[1440px]'}>
                  <Suspense fallback={<PageFallback label={t.common?.loading || 'Loading...'} />}>
                    {renderTabContent(activeTab)}
                  </Suspense>
                </div>
              </div>
            )}
          </div>
        </main>
      </div>
    </div>
  )
}

function App() {
  return <I18nProvider><ErrorBoundary><AppInner /></ErrorBoundary></I18nProvider>
}

export default App
