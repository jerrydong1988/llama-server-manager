import { useState, useEffect } from 'react'
import { Server, Database, Cpu, Terminal, Sun, Moon, Zap, Settings, Activity, Network, Download, BarChart3 } from 'lucide-react'
import { version } from '../package.json'
import ModelRepo from './components/ModelRepo'
import EngineManager from './components/EngineManager'
import InstanceManager from './components/InstanceManager'
import LogsViewer from './components/LogsViewer'
import ConfigPage from './components/ConfigPage'
import PerformancePage from './components/PerformancePage/PerformancePage'
import ClusterPage from './components/ClusterPage/ClusterPage'
import DownloadManager from './components/DownloadManager'
import Dashboard from './components/Dashboard/Dashboard'
import { useAppStore } from './store'
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
}
const renderTabContent = (tabId: string) => {
  const Renderer = TAB_CONTENT[tabId]
  return Renderer ? <Renderer /> : <div className="p-6 text-center text-gray-500">...</div>
}

function AppInner() {
  const { activeTab, setActiveTab, loadConfig, instances, startInstance, stopInstance, saveConfig, darkMode, setDarkMode } = useAppStore()
  const { t, lang, setLang } = useI18n()
  const [updateInfo, setUpdateInfo] = useState<{latest_version: string; url: string} | null>(null)

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
  ]
  // dark mode handled by store setDarkMode
  useEffect(() => { loadConfig() }, [loadConfig])

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
      const has = l.some((v: number, i: number) => v > (c[i] || 0)) && !c.some((v: number, i: number) => v > (l[i] || 0))
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
            _ 新版本 v{updateInfo.latest_version} 可用 | 点击下载
          </a>
        )}
      </div>
      <div className="flex-1 overflow-y-auto">
        {activeTab === 'logs' ? (
          <LogsViewer />
        ) : (
          <div className="max-w-7xl mx-auto p-6">
            <h2 className="text-2xl font-bold mb-6">{navigation.find(n => n.id === activeTab)?.name}</h2>
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
