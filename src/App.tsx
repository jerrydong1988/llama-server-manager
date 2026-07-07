import { useState, useEffect, useMemo, useRef, lazy, Suspense, Component, ReactNode } from 'react'
import { Server, Database, Cpu, Terminal, Settings, Activity, Network, Download, BarChart3, BookOpen } from 'lucide-react'
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
import { Button } from './components/ui'
import { AppShell, type ShellStatusChip } from './components/shell/AppShell'

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

  const layoutWide = activeTab === 'logs' || activeTab === 'guide'
  const statusChips: ShellStatusChip[] = [
    { label: t.nav.up || 'up', value: upCount, tone: 'emerald' },
    { label: t.nav.down || 'down', value: downCount, tone: 'slate' },
    { label: t.nav.downloads || 'Downloads', value: activeDownloadCount, tone: 'blue' },
  ]
  const pageContext: Record<string, string> = {
    dashboard: lang === 'zh-CN' ? '系统总览与 llama-server 实例活动' : 'System overview and current llama-server activity',
    'model-repo': lang === 'zh-CN' ? '浏览本地与远程模型资源' : 'Browse local and remote model assets',
    downloads: lang === 'zh-CN' ? '模型传输队列与活动任务' : 'Model transfer queue and active tasks',
    engine: lang === 'zh-CN' ? '运行时二进制与执行后端' : 'Runtime binaries and execution backends',
    instances: lang === 'zh-CN' ? '服务实例、端口与进程状态' : 'Server instances, ports, and process state',
    config: lang === 'zh-CN' ? '应用默认值与运行时偏好' : 'Application defaults and runtime preferences',
    cluster: lang === 'zh-CN' ? '分布式 worker 与网络可用性' : 'Distributed workers and network availability',
    perf: lang === 'zh-CN' ? '性能遥测与资源信号' : 'Performance telemetry and resource signals',
    logs: lang === 'zh-CN' ? '运行日志与诊断输出' : 'Runtime logs and diagnostic output',
    guide: lang === 'zh-CN' ? '参考资料与操作说明' : 'Reference material and operating notes',
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

  return (
    <AppShell
      appTitle={t.common.appTitle}
      version={version}
      navigation={navigation}
      activeId={activeTab}
      onNavigate={setActiveTab}
      pageDescription={pageContext[activeTab] || (lang === 'zh-CN' ? '管理本地 llama-server 运行状态' : 'Manage local llama-server runtime state')}
      statusChips={statusChips}
      updateInfo={updateInfo}
      autoStartLabel={t.common.autoStart || 'Auto Start'}
      autoStartEnabled={autoStartEnabled}
      onAutoStartChange={handleAutoStartChange}
      darkMode={darkMode}
      onToggleDarkMode={() => setDarkMode(!darkMode)}
      darkModeTitle={darkMode ? 'Switch to light mode' : 'Switch to dark mode'}
      languageLabel={lang === 'zh-CN' ? 'EN' : '\u4E2D\u6587'}
      languageTitle={lang === 'zh-CN' ? 'Switch to English' : '\u5207\u6362\u4E3A\u4E2D\u6587'}
      onToggleLanguage={() => setLang(lang === 'zh-CN' ? 'en-US' : 'zh-CN')}
      wideContent={layoutWide}
    >
      {!configLoaded ? (
        <div className="flex min-h-[calc(100vh-160px)] items-center justify-center px-6">
          <PageFallback label={t.common?.loading || 'Loading...'} />
        </div>
      ) : (
        <Suspense fallback={<PageFallback label={t.common?.loading || 'Loading...'} />}>
          {renderTabContent(activeTab)}
        </Suspense>
      )}
    </AppShell>
  )
}

function App() {
  return <I18nProvider><ErrorBoundary><AppInner /></ErrorBoundary></I18nProvider>
}

export default App
