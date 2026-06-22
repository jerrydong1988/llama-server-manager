import { useState, useEffect } from 'react'
import { Server, Database, Cpu, Terminal, Sun, Moon, Zap, Settings, Activity, Network, Download, BarChart3, BookOpen } from 'lucide-react'
import { version } from '../package.json'
import { invoke } from '@tauri-apps/api/core'
import ModelRepo from './components/ModelRepo'
import EngineManager from './components/EngineManager'
import InstanceManager from './components/InstanceManager'
import LogsViewer from './components/LogsViewer'
import ConfigPage from './components/ConfigPage'
import PerformancePage from './components/PerformancePage/PerformancePage'
import ClusterPage from './components/ClusterPage/ClusterPage'
import DownloadManager from './components/DownloadManager'
import Dashboard from './components/Dashboard/Dashboard'
import GuidePage from './components/GuidePage'
import { useAppStore } from './store'
import type { WorkerInfo } from './store'
import { I18nProvider, useI18n } from './i18n'

// Moved outside AppInner to avoid re-creation on each render
const TAB_CONTENT: Record<string, () => JSX.Element> = {
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
  return Renderer ? <Renderer /> : <div className="p-6 text-center text-gray-500">...</div>
}

function AppInner() {
  const { activeTab, setActiveTab, loadConfig, instances, startInstance, stopInstance, saveConfig, darkMode, setDarkMode } = useAppStore()
  const { t, lang, setLang } = useI18n()
  const [updateInfo, setUpdateInfo] = useState<{latest_version: string; url: string} | null>(null)
  const [autoStartEnabled, setAutoStartEnabled] = useState(false)
  const [autoStarted, setAutoStarted] = useState(false)

  // 读取主程序自启动状态
  useEffect(() => { invoke<boolean>('is_autostart_enabled').then(setAutoStartEnabled).catch(() => {}) }, [])

  const navigation = [
    { id: 'dashboard', name: t.nav.dashboard || 'Dashboard', icon: BarChart3 },
    { id: 'model-repo', name: t.nav.modelRepo, icon: Database },
    { id: 'downloads', name: t.nav.downloads || 'Downloads', icon: Download },
    { id: 'engine', name: t.nav.engine, icon: Cpu },
    { id: 'instances', name: t.nav.instances, icon: Server, badge: instances.filter(i => i.status === 'running').length },
    { id: 'config', name: t.nav.config, icon: Settings, separator: true },
    { id: 'cluster', name: t.nav.cluster, icon: Network },
    { id: 'perf', name: t.nav.perf, icon: Activity },
    { id: 'logs', name: t.nav.logs, icon: Terminal },
    { id: 'guide', name: '\uD83D\uDCD6 \u4F7F\u7528\u8BF4\u660E', icon: BookOpen, separator: true },
  ]
  // dark mode handled by store setDarkMode
  useEffect(() => { loadConfig() }, [loadConfig])

  // 自动启动标记了 auto_start 的实例
  useEffect(() => {
    if (autoStarted || instances.length === 0) return
    setAutoStarted(true)

    const toBoot = instances.filter(i => i.config.auto_start && i.status !== 'running')
    if (toBoot.length === 0) return

    const boot = async () => {
      // 集群实例：检查是否有匹配的 worker
      const currentWorkers = useAppStore.getState().workers.length > 0
        ? useAppStore.getState().workers
        : await invoke<WorkerInfo[]>('get_workers').catch(() => [])

      for (const inst of toBoot) {
        // 集群实例 rpc_servers 检查
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
    }
    boot()
  }, [instances, autoStarted, startInstance])

  // 自动更新检查 (前端 fetch 走 Chromium 网络栈, 兼容TUN/Clash)
  useEffect(() => {
    fetch('https://api.github.com/repos/jerrydong1988/llama-server-manager/releases/latest', {
      headers: { 'Accept': 'application/vnd.github+json' }
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
  }, [])

  // 键盘快捷键
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!e.ctrlKey) return
      if (e.key === 'Enter') {
        e.preventDefault()
        // Ctrl+Enter: 实例页则启动第一个已停止的实例，或停止第一个运行中的实例
        if (instances.length > 0) {
          const running = instances.find(i => i.status === 'running')
          const stopped = instances.find(i => i.status !== 'running')
          if (running) stopInstance(running.id)
          else if (stopped) startInstance(stopped.id)
        }
      }
      if (e.key === 's' || e.key === 'S') {
        e.preventDefault()
        saveConfig()
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [instances, startInstance, stopInstance, saveConfig])

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
            <span className="flex items-center gap-1"><span className="w-2 h-2 rounded-full bg-emerald-500" /> {instances.filter(i => i.status === 'running').length} {t.nav.up || 'up'}</span>
            <span className="flex items-center gap-1"><span className="w-2 h-2 rounded-full bg-slate-400" /> {instances.filter(i => i.status !== 'running').length} {t.nav.down || 'down'}</span>
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
        {activeTab === 'logs' ? (
          <LogsViewer />
        ) : activeTab === 'guide' ? (
          <GuidePage />
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
  return <I18nProvider><AppInner /></I18nProvider>
}

export default App
