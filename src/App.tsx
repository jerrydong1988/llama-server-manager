import { useState, useEffect, useMemo, useRef, lazy, Suspense, Component, ReactNode } from 'react'
import { Server, Database, Cpu, Terminal, Sun, Moon, Zap, Settings, Activity, Network, Download, BarChart3, BookOpen } from 'lucide-react'
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
        <div className="h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900 text-gray-900 dark:text-gray-100">
          <div className="text-center max-w-md p-6">
            <h2 className="text-xl font-bold mb-2">页面渲染出错</h2>
            <p className="text-sm text-gray-500 mb-4">{this.state.error?.message || '未知错误'}</p>
            <button onClick={() => this.setState({ hasError: false, error: null })}
              className="px-4 py-2 bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors">
              重试
            </button>
          </div>
        </div>
      )
    }
    return this.props.children
  }
}

// Moved outside AppInner to avoid re-creation on each render
const TAB_CONTENT: Record<string, () => React.ReactElement> = {
  instances: () => <InstanceManager />,
  'model-repo': () => <ModelRepo />,
  dashboard: () => <Dashboard />,
  engine: () => <EngineManager />,
  config: () => <ConfigPage />,
  perf: () => <Suspense fallback={null}><PerformancePage /></Suspense>,
  logs: () => <Suspense fallback={null}><LogsViewer /></Suspense>,
  cluster: () => <Suspense fallback={null}><ClusterPage /></Suspense>,
  downloads: () => <Suspense fallback={null}><DownloadManager /></Suspense>,
  guide: () => <Suspense fallback={null}><GuidePage /></Suspense>,
}
const renderTabContent = (tabId: string) => {
  const Renderer = TAB_CONTENT[tabId]
  return Renderer ? <Renderer /> : <div className="p-6 text-center text-gray-500">...</div>
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
  const [updateInfo, setUpdateInfo] = useState<{latest_version: string; url: string} | null>(null)
  const [autoStartEnabled, setAutoStartEnabled] = useState(false)
  const [autoStarted, setAutoStarted] = useState(false)

  // 读取主程序自启动状态
  useEffect(() => { invoke<boolean>('is_autostart_enabled').then(setAutoStartEnabled).catch(() => {}) }, [])

  const upCount = useMemo(() => instances.filter(i => i.status === 'running').length, [instances])
  const downCount = useMemo(() => instances.filter(i => i.status !== 'running').length, [instances])
  const downloadTasks = useAppStore(s => s.downloadTasks)
  const activeDownloadCount = useMemo(() =>
    Object.values(downloadTasks).filter(t => t.status === 'active' || t.status === 'paused' || t.status === 'pausing').length,
  [downloadTasks])
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
    { id: 'guide', name: '\u4F7F\u7528\u8BF4\u660E', icon: BookOpen, separator: true },
  ], [t, upCount, activeDownloadCount])
  // dark mode handled by store setDarkMode
  const mountTimeRef = useRef(performance.now())
  const [configLoaded, setConfigLoaded] = useState(false)

  // 窗口立即显示 — React 首次渲染骨架 UI 后就调用 show_window
  // 不等 loadConfig，用户立即看到可拖动窗口 + 加载动画
  const windowShownRef = useRef(false)
  useEffect(() => {
    _startupTimings.push({ name: 'app-mount', ms: Math.round(performance.now() - mountTimeRef.current) })
    if (!windowShownRef.current) {
      windowShownRef.current = true
      invoke('show_window').catch(() => {})
    }
    loadConfig().then(() => setConfigLoaded(true)).catch(() => setConfigLoaded(true))
  }, [loadConfig])

  // 自动启动标记了 auto_start 的实例 — 等待 engines 加载完毕再启动
  useEffect(() => {
    if (autoStarted || instances.length === 0) return
    const toBoot = instances.filter(i => i.config.auto_start && i.status !== 'running')
    if (toBoot.length === 0) { setAutoStarted(true); return }
    if (engines.length === 0) return // engines 尚未加载，等待 load_app_data 完成

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
            })
          )
          if (!hasMatchingWorker) {
            console.warn(`实例 "${inst.name}" 配置了集群 worker 但无匹配记录，跳过自启动`)
            continue
          }
        }

        try { await startInstance(inst.id) } catch { /* skip failed start */ }
        await new Promise(r => setTimeout(r, 3000))
      }
      setAutoStarted(true)
    }
    boot()
    return () => { cancelled = true }
  }, [instances, engines, autoStarted, startInstance])

  // 自动更新检查 (前端 fetch 走 Chromium 网络栈, 兼容TUN/Clash)
  useEffect(() => {
    const controller = new AbortController()
    fetch('https://api.github.com/repos/jerrydong1988/llama-server-manager/releases/latest', {
      headers: { 'Accept': 'application/vnd.github+json' },
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
        if (lv > cv) { has = true; break }
        if (lv < cv) { has = false; break }
      }
      if (has) setUpdateInfo({ latest_version: latest, url: json.html_url || '' })
    })
    .catch(() => {})
    return () => controller.abort()
  }, [])

  // 键盘快捷键 — 使用 getState() 避免依赖频繁变化
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

  return (
    <div className={`h-screen flex overflow-hidden ${darkMode ? 'dark bg-gray-900 text-gray-100' : 'bg-gray-50 text-gray-900'}`}>
      <div className="w-64 h-screen flex flex-col border-r dark:border-gray-700 border-gray-200 p-4">
        <div className="flex items-center gap-2 mb-6 shrink-0">
          <Zap className="w-8 h-8 text-blue-500" />
          <h1 className="text-xl font-bold">{t.common.appTitle}</h1>
          <span className="text-xs text-gray-500 ml-1">v{version}</span>
        </div>
        <nav className="space-y-1 overflow-y-auto flex-1">
          {navigation.map((item) => {
            const Icon = item.icon
            return (
              <div key={item.id}>
                {item.separator && <div className="my-2 border-t border-gray-200 dark:border-gray-700" />}
                <button onClick={() => setActiveTab(item.id)}
                  className={`w-full flex items-center gap-3 px-3 py-2 rounded-lg transition-colors ${activeTab === item.id ? 'bg-indigo-600 text-white shadow-lg shadow-indigo-600/20' : 'hover:bg-gray-100 dark:hover:bg-gray-800'}`}>
                  <Icon className="w-5 h-5" /><span>{item.name}</span>
                  {item.badge != null && item.badge > 0 && (
                    <span className="ml-auto bg-emerald-500 text-white text-xs px-1.5 py-0.5 rounded-full font-medium">{item.badge}</span>
                  )}
                </button>
              </div>
            )
          })}
        </nav>
        <div className="flex items-center gap-1 shrink-0 pt-3 border-t border-gray-200 dark:border-gray-700">
          <div className="flex-1 flex items-center justify-between text-xs text-gray-500">
            <span className="flex items-center gap-1"><span className="w-2 h-2 rounded-full bg-emerald-500" /> {upCount} {t.nav.up || 'up'}</span>
            <span className="flex items-center gap-1"><span className="w-2 h-2 rounded-full bg-slate-400" /> {downCount} {t.nav.down || 'down'}</span>
          </div>
        </div>
        <div className="flex items-center gap-1 shrink-0">
          <button
            type="button"
            role="switch"
            aria-checked={autoStartEnabled}
            onClick={() => {
              const next = !autoStartEnabled
              setAutoStartEnabled(next)
              ;(async () => {
                try { if (next) await invoke('enable_autostart'); else await invoke('disable_autostart') }
                catch { setAutoStartEnabled(!next) }
              })()
            }}
            className={`relative inline-flex h-5 w-9 shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ${autoStartEnabled ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'}`}
            title={t.common.autoStart || '开机自启动'}
          >
            <span className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow transition duration-200 ${autoStartEnabled ? 'translate-x-4' : 'translate-x-0'}`} />
          </button>
          <button onClick={() => setLang(lang === 'zh-CN' ? 'en-US' : 'zh-CN')}
            className="p-2 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors text-xs">
            {lang === 'zh-CN' ? 'EN' : '中'}
          </button>
          <button onClick={() => setDarkMode(!darkMode)}
            className="p-2 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors">
            {darkMode ? <Sun className="w-5 h-5" /> : <Moon className="w-5 h-5" />}
          </button>
        </div>
        {/* 更新提示 */}
        {updateInfo && (
          <a href={updateInfo.url} target="_blank" rel="noreferrer"
            className="block mt-2 px-2 py-1.5 bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-400 rounded text-xs text-center hover:bg-green-200 dark:hover:bg-green-900/50 transition-colors">
            {'\uD83D\uDD14'} {t.common.updateAvailable || '新版本'} v{updateInfo.latest_version} {t.common.clickToDownload || '点击下载'}
          </a>
        )}
      </div>
      <div className="flex-1 overflow-y-auto">
        {!configLoaded ? (
          <div className="h-full flex items-center justify-center">
            <div className="text-center">
              <div className="w-10 h-10 border-3 border-blue-500 border-t-transparent rounded-full animate-spin mx-auto mb-3" style={{ borderWidth: '3px' }} />
              <p className="text-sm text-gray-400">{t.common?.loading || '加载中...'}</p>
            </div>
          </div>
        ) : activeTab === 'logs' ? (
          <Suspense fallback={null}><LogsViewer /></Suspense>
        ) : activeTab === 'guide' ? (
          <Suspense fallback={null}><GuidePage /></Suspense>
        ) : (
          <div className="max-w-7xl mx-auto p-6">
            {renderTabContent(activeTab)}
          </div>
        )}
      </div>
    </div>
  )
}

function App() {
  return <I18nProvider><ErrorBoundary><AppInner /></ErrorBoundary></I18nProvider>
}

export default App
