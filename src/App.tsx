import { useState, useEffect } from 'react'
import { Server, Database, Cpu, Terminal, Sun, Moon, Zap, Settings } from 'lucide-react'
import ModelRepo from './components/ModelRepo'
import EngineManager from './components/EngineManager'
import InstanceManager from './components/InstanceManager'
import LogsViewer from './components/LogsViewer'
import ConfigPage from './components/ConfigPage'
import { useAppStore } from './store'

const navigation = [
  { id: 'model-repo', name: '模型仓库', icon: Database },
  { id: 'engine', name: '引擎管理', icon: Cpu },
  { id: 'instances', name: '实例管理', icon: Server },
  { id: 'config', name: '参数配置', icon: Settings },
  { id: 'logs', name: '服务器日志', icon: Terminal },
]

const renderTabContent = (tabId: string) => {
  switch (tabId) {
    case 'instances': return <InstanceManager />
    case 'model-repo': return <ModelRepo />
    case 'engine': return <EngineManager />
    case 'config': return <ConfigPage />
    case 'logs': return <LogsViewer />
    default: return <div className="p-6 text-center text-gray-500">模块开发中...</div>
  }
}

function App() {
  const { activeTab, setActiveTab, loadConfig } = useAppStore()
  const [darkMode, setDarkMode] = useState(true)

  useEffect(() => {
    document.documentElement.classList.toggle('dark', darkMode)
  }, [darkMode])

  useEffect(() => {
    loadConfig()
  }, [loadConfig])

  return (
    <div className={`h-screen flex overflow-hidden ${darkMode ? 'dark bg-gray-900 text-gray-100' : 'bg-gray-50 text-gray-900'}`}>
      <div className="w-64 h-screen flex flex-col border-r dark:border-gray-700 border-gray-200 p-4">
        <div className="flex items-center gap-2 mb-6 shrink-0">
          <Zap className="w-8 h-8 text-blue-500" />
          <h1 className="text-xl font-bold">Llama 管理器</h1>
        </div>

        <nav className="space-y-1 overflow-y-auto flex-1">
          {navigation.map((item) => {
            const Icon = item.icon
            return (
              <button
                key={item.id}
                onClick={() => setActiveTab(item.id)}
                className={`w-full flex items-center gap-3 px-3 py-2 rounded-lg transition-colors ${
                  activeTab === item.id
                    ? 'bg-blue-600 text-white'
                    : 'hover:bg-gray-100 dark:hover:bg-gray-800'
                }`}
              >
                <Icon className="w-5 h-5" />
                <span>{item.name}</span>
              </button>
            )
          })}
        </nav>

        <button onClick={() => setDarkMode(!darkMode)}
          className="mt-auto p-2 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors shrink-0">
          {darkMode ? <Sun className="w-5 h-5" /> : <Moon className="w-5 h-5" />}
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        <div className="max-w-7xl mx-auto p-6">
          <h2 className="text-2xl font-bold mb-6">
            {navigation.find(n => n.id === activeTab)?.name}
          </h2>
          {renderTabContent(activeTab)}
        </div>
      </div>
    </div>
  )
}

export default App
