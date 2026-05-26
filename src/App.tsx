import { useState, useEffect } from 'react'
import { Server, Database, Cpu, Terminal, Sun, Moon, Zap, Settings } from 'lucide-react'
import ModelRepo from './components/ModelRepo'
import EngineManager from './components/EngineManager'
import InstanceManager from './components/InstanceManager'
import LogsViewer from './components/LogsViewer'
import ConfigPage from './components/ConfigPage'
import { useAppStore } from './store'
import { I18nProvider, useI18n } from './i18n'

const renderTabContent = (tabId: string) => {
  switch (tabId) {
    case 'instances': return <InstanceManager />
    case 'model-repo': return <ModelRepo />
    case 'engine': return <EngineManager />
    case 'config': return <ConfigPage />
    case 'logs': return <LogsViewer />
    default: return <div className="p-6 text-center text-gray-500">...</div>
  }
}

function AppInner() {
  const { activeTab, setActiveTab, loadConfig, instances, startInstance, stopInstance, saveConfig, darkMode, setDarkMode } = useAppStore()
  const { t, lang, setLang } = useI18n()

  const navigation = [
    { id: 'model-repo', name: t.nav.modelRepo, icon: Database },
    { id: 'engine', name: t.nav.engine, icon: Cpu },
    { id: 'instances', name: t.nav.instances, icon: Server },
    { id: 'config', name: t.nav.config, icon: Settings },
    { id: 'logs', name: t.nav.logs, icon: Terminal },
  ]
  // dark mode handled by store setDarkMode
  useEffect(() => { loadConfig() }, [loadConfig])

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
          <span className="text-xs text-gray-500 ml-1">v2.0.5</span>
        </div>
        <nav className="space-y-1 overflow-y-auto flex-1">
          {navigation.map((item) => {
            const Icon = item.icon
            return (
              <button key={item.id} onClick={() => setActiveTab(item.id)}
                className={`w-full flex items-center gap-3 px-3 py-2 rounded-lg transition-colors ${activeTab === item.id ? 'bg-blue-600 text-white' : 'hover:bg-gray-100 dark:hover:bg-gray-800'}`}>
                <Icon className="w-5 h-5" /><span>{item.name}</span>
              </button>
            )
          })}
        </nav>
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
      </div>
      <div className="flex-1 overflow-y-auto">
        <div className="max-w-7xl mx-auto p-6">
          <h2 className="text-2xl font-bold mb-6">{navigation.find(n => n.id === activeTab)?.name}</h2>
          {renderTabContent(activeTab)}
        </div>
      </div>
    </div>
  )
}

function App() {
  return <I18nProvider><AppInner /></I18nProvider>
}

export default App
