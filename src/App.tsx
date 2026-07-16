import { useState, useEffect, useMemo, useRef, lazy, Suspense, Component, ReactNode } from 'react'
import { Activity, BarChart3, BookOpen, Cpu, Database, Download, Monitor, Network, Route, Server, Settings, Terminal, X } from 'lucide-react'
import { version } from '../package.json'
import { invokeApp as invoke } from './lib/ipc'
import { listen } from '@tauri-apps/api/event'
import ModelRepo from './components/ModelRepo'
import EngineManager from './components/EngineManager'
import InstanceManager from './components/InstanceManager'
const LogsViewer = lazy(() => import('./components/LogsViewer'))
import ConfigPage from './components/ConfigPage'
const PerformancePage = lazy(() => import('./components/PerformancePage/PerformancePage'))
const ClusterPage = lazy(() => import('./components/ClusterPage/ClusterPage'))
const DownloadManager = lazy(() => import('./components/DownloadManager'))
const ProxyPage = lazy(() => import('./components/ProxyPage'))
const BigScreenPage = lazy(() => import('./components/BigScreenPage'))
import Dashboard from './components/Dashboard/Dashboard'
const GuidePage = lazy(() => import('./components/GuidePage'))
import { _startupTimings } from './store'
import { useAppStore } from './store'
import { parseHostPort } from './utils/network'
import type { WorkerInfo } from './store'
import { I18nProvider, nextLanguage, useI18n } from './i18n'
import { Button } from './components/ui'
import { AppShell, type ShellStatusChip } from './components/shell/AppShell'
import { CommandCenter } from './components/shell/CommandCenter'
import { useCommandCenterModel } from './components/shell/useCommandCenterModel'

type ErrorBoundaryCopy = { title: string; description: string; unknown: string; reload: string }

class ErrorBoundaryCore extends Component<{ children: ReactNode; copy: ErrorBoundaryCopy }, { hasError: boolean; error: Error | null }> {
  constructor(props: { children: ReactNode; copy: ErrorBoundaryCopy }) {
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
                <h2 className="text-lg font-semibold">{this.props.copy.title}</h2>
                <p className="text-sm text-slate-500 dark:text-slate-400">{this.props.copy.description}</p>
              </div>
            </div>
            <div className="mb-5 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-sm text-slate-600 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-300">
              {this.state.error?.message || this.props.copy.unknown}
            </div>
            <Button
              onClick={() => this.setState({ hasError: false, error: null })}
              variant="primary"
            >
              {this.props.copy.reload}
            </Button>
          </div>
        </div>
      )
    }

    return this.props.children
  }
}

function ErrorBoundary({ children }: { children: ReactNode }) {
  const { t } = useI18n()
  return <ErrorBoundaryCore copy={t.appShell.crash}>{children}</ErrorBoundaryCore>
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
  proxy: () => <ProxyPage />,
  downloads: () => <DownloadManager />,
  bigscreen: () => <BigScreenPage />,
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
  const runtimeWarnings = useAppStore(s => s.runtimeWarnings)
  const clearRuntimeWarnings = useAppStore(s => s.clearRuntimeWarnings)
  const { t, lang, setLang } = useI18n()
  const shellCopy = t.appShell
  const [updateInfo, setUpdateInfo] = useState<{ latest_version: string; url: string } | null>(null)
  const [autoStartEnabled, setAutoStartEnabled] = useState(false)
  const [autoStarted, setAutoStarted] = useState(false)
  const [commandCenterOpen, setCommandCenterOpen] = useState(false)
  const [proxyExitConfirmOpen, setProxyExitConfirmOpen] = useState(false)
  const [proxyExitKeepAliveEnabled, setProxyExitKeepAliveEnabled] = useState(false)

  useEffect(() => {
    invoke<boolean>('is_autostart_enabled').then(setAutoStartEnabled).catch(() => {})
  }, [])

  const upCount = useMemo(() => instances.filter(i => i.status === 'running').length, [instances])
  const downCount = useMemo(() => instances.filter(i => i.status !== 'running').length, [instances])
  const activeDownloadCount = useAppStore(state => Object.values(state.downloadTasks)
    .filter(task => task.status === 'active' || task.status === 'paused' || task.status === 'pausing').length)

  const navigation = useMemo(() => [
    { id: 'dashboard', name: t.nav.dashboard, icon: BarChart3 },
    { id: 'model-repo', name: t.nav.modelRepo, icon: Database },
    { id: 'downloads', name: t.nav.downloads, icon: Download, badge: activeDownloadCount },
    { id: 'engine', name: t.nav.engine, icon: Cpu },
    { id: 'instances', name: t.nav.instances, icon: Server, badge: upCount },
    { id: 'config', name: t.nav.config, icon: Settings, separator: true },
    { id: 'cluster', name: t.nav.cluster, icon: Network },
    { id: 'proxy', name: t.nav.proxy, icon: Route },
    { id: 'perf', name: t.nav.perf, icon: Activity },
    { id: 'bigscreen', name: t.nav.bigScreen, icon: Monitor },
    { id: 'logs', name: t.nav.logs, icon: Terminal },
    { id: 'guide', name: t.nav.guide, icon: BookOpen, separator: true },
  ], [t, upCount, activeDownloadCount])

  const { productIssues, commandActions } = useCommandCenterModel()

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
    let disposed = false
    let unlisten: (() => void) | undefined

    listen('proxy-exit-confirmation-requested', event => {
      const payload = event.payload && typeof event.payload === 'object'
        ? event.payload as Record<string, unknown>
        : {}
      setProxyExitKeepAliveEnabled(payload.backgroundServiceMode === true || payload.background_service_mode === true)
      setProxyExitConfirmOpen(true)
    }).then(cleanup => {
      if (disposed) cleanup()
      else unlisten = cleanup
    }).catch(() => {})

    return () => {
      disposed = true
      if (unlisten) unlisten()
    }
  }, [])

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
              const endpoint = parseHostPort(s, 50052)
              return w.host === endpoint.host && w.port === endpoint.port
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
          if (running) void stopInstance(running.id).catch(() => {})
          else if (stopped) void startInstance(stopped.id).catch(() => {})
        }
      }
      if (e.key === 's' || e.key === 'S') {
        e.preventDefault()
        void useAppStore.getState().saveConfig().catch(() => {})
      }
      if (e.key === 'k' || e.key === 'K') {
        e.preventDefault()
        setCommandCenterOpen(true)
      }
    }

    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])

  const immersiveContent = activeTab === 'bigscreen'
  const layoutWide = activeTab === 'logs' || activeTab === 'guide' || immersiveContent || activeTab === 'proxy'
  const statusChips: ShellStatusChip[] = [
    { label: t.nav.up || 'up', value: upCount, tone: 'emerald' },
    { label: t.nav.down || 'down', value: downCount, tone: 'slate' },
    { label: t.nav.downloads || 'Downloads', value: activeDownloadCount, tone: 'blue' },
  ]
  const pageContext: Record<string, string> = {
    dashboard: shellCopy.pageDescriptions.dashboard,
    'model-repo': shellCopy.pageDescriptions.modelRepo,
    downloads: shellCopy.pageDescriptions.downloads,
    engine: shellCopy.pageDescriptions.engine,
    instances: shellCopy.pageDescriptions.instances,
    config: shellCopy.pageDescriptions.config,
    cluster: shellCopy.pageDescriptions.cluster,
    proxy: shellCopy.pageDescriptions.proxy,
    perf: shellCopy.pageDescriptions.perf,
    logs: shellCopy.pageDescriptions.logs,
    guide: shellCopy.pageDescriptions.guide,
  }
  const handleAutoStartChange = (next: boolean) => {
    setAutoStartEnabled(next)
    ;(async () => {
      try {
        if (next) await invoke('enable_autostart')
        else await invoke('disable_autostart')
      } catch {
        setAutoStartEnabled(!next)
      }
    })()
  }
  const handleStopProxyAndQuit = async () => {
    try {
      await invoke('stop_proxy')
      await invoke('quit_app')
    } catch {
      setProxyExitConfirmOpen(false)
      setProxyExitKeepAliveEnabled(false)
      setActiveTab('proxy')
    }
  }
  const handleKeepProxyInTray = () => {
    setProxyExitConfirmOpen(false)
    setProxyExitKeepAliveEnabled(false)
    invoke('hide_window').catch(() => {})
  }

  return (
    <AppShell
      appTitle={t.common.appTitle}
      version={version}
      navigation={navigation}
      activeId={activeTab}
      onNavigate={setActiveTab}
      pageDescription={pageContext[activeTab] || shellCopy.pageDescriptions.fallback}
      statusChips={statusChips}
      updateInfo={updateInfo}
      autoStartLabel={t.common.autoStart || 'Auto Start'}
      autoStartEnabled={autoStartEnabled}
      onAutoStartChange={handleAutoStartChange}
      darkMode={darkMode}
      onToggleDarkMode={() => setDarkMode(!darkMode)}
      darkModeTitle={darkMode ? shellCopy.switchToLight : shellCopy.switchToDark}
      languageLabel={shellCopy.languageLabel}
      languageTitle={shellCopy.languageTitle}
      onToggleLanguage={() => setLang(nextLanguage(lang))}
      commandLabel={shellCopy.commandCenter}
      attentionLabel={shellCopy.attentionCenter}
      attentionCount={productIssues.length}
      onOpenCommandCenter={() => setCommandCenterOpen(true)}
      wideContent={layoutWide}
      immersiveContent={immersiveContent}
      constrainContent={activeTab === 'guide'}
    >
      {!immersiveContent && runtimeWarnings.length > 0 && (
        <div className="mx-auto mb-4 flex w-full max-w-7xl items-start justify-between gap-4 rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 text-sm text-amber-900 shadow-sm dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-100">
          <div className="min-w-0">
            <div className="font-semibold">{shellCopy.runtimeWarnings}</div>
            <div className="mt-1 truncate text-xs opacity-90" title={runtimeWarnings[0]}>{runtimeWarnings[0]}</div>
          </div>
          <button
            type="button"
            onClick={clearRuntimeWarnings}
            className="shrink-0 rounded-md border border-amber-300 px-2 py-1 text-xs font-medium transition hover:bg-amber-100 dark:border-amber-400/40 dark:hover:bg-amber-500/20"
          >
            {shellCopy.clear}
          </button>
        </div>
      )}
      {!configLoaded ? (
        <div className="flex min-h-[calc(100vh-160px)] items-center justify-center px-6">
          <PageFallback label={t.common?.loading || 'Loading...'} />
        </div>
      ) : (
        <Suspense fallback={<PageFallback label={t.common?.loading || 'Loading...'} />}>
          {renderTabContent(activeTab)}
        </Suspense>
      )}
      <CommandCenter
        open={commandCenterOpen}
        issues={productIssues}
        commands={commandActions}
        onClose={() => setCommandCenterOpen(false)}
      />
      {proxyExitConfirmOpen ? (
        <div className="fixed inset-0 z-[120] flex items-center justify-center bg-slate-950/60 px-4 backdrop-blur-sm">
          <div className="w-full max-w-lg rounded-xl border border-slate-200 bg-white p-6 shadow-2xl dark:border-slate-800 dark:bg-slate-950">
            <div className="flex items-start justify-between gap-4">
              <div>
                <h2 className="text-xl font-semibold text-slate-950 dark:text-slate-50">
                  {shellCopy.proxyRunning}
                </h2>
                <p className="mt-3 text-sm leading-6 text-slate-600 dark:text-slate-300">
                  {proxyExitKeepAliveEnabled ? shellCopy.proxyKeepAliveDescription : shellCopy.proxyExitDescription}
                </p>
              </div>
              <button
                type="button"
                onClick={() => {
                  setProxyExitConfirmOpen(false)
                  setProxyExitKeepAliveEnabled(false)
                }}
                className="rounded-lg border border-slate-200 p-2 text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 dark:border-slate-800 dark:text-slate-400 dark:hover:bg-slate-900 dark:hover:text-slate-100"
                aria-label={shellCopy.close}
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="mt-6 flex flex-col-reverse gap-3 sm:flex-row sm:justify-end">
              <Button onClick={handleKeepProxyInTray}>
                {proxyExitKeepAliveEnabled
                  ? shellCopy.keepRouteRunning
                  : shellCopy.keepInTray}
              </Button>
              <Button
                onClick={() => {
                  setActiveTab('proxy')
                  setProxyExitConfirmOpen(false)
                  setProxyExitKeepAliveEnabled(false)
                }}
              >
                {shellCopy.openProxySettings}
              </Button>
              <Button variant="danger" onClick={() => void handleStopProxyAndQuit()}>
                {shellCopy.stopProxyAndQuit}
              </Button>
            </div>
          </div>
        </div>
      ) : null}
    </AppShell>
  )
}

function App() {
  return <I18nProvider><ErrorBoundary><AppInner /></ErrorBoundary></I18nProvider>
}

export default App
