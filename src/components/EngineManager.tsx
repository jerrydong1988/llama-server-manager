import { useEffect, useState } from 'react'
import { RefreshCw, FolderOpen, Trash2, Plus, Cpu, Pencil } from 'lucide-react'
import { useAppStore } from '../store'
import { useI18n } from '../i18n'
import { confirm } from '@tauri-apps/plugin-dialog'

const EngineManager = () => {
const { engines, scanEngines, loadInitialData, isLoading, deleteEngine, renameEngine, openEngineFolder, defaultEngineId, setDefaultEngineId, engineDirs, setEngineDirs } = useAppStore()
const { t } = useI18n()

const [editingId, setEditingId] = useState<string | null>(null)
const [editName, setEditName] = useState('')

useEffect(() => { loadInitialData() }, [loadInitialData])

  const handleScan = async () => await scanEngines(engineDirs)

  const handleAddDirectory = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog')
      const dir = await open({ directory: true, title: t.engineMgr.addDirTitle })
      if (dir) {
        const d = dir as string
        const all = [...new Set([...engineDirs, d])]
        setEngineDirs(all)
        await scanEngines(all)
      }
    } catch (_) { await scanEngines(engineDirs) }
  }

  const handleRemoveDir = async (dir: string) => {
    if (!await confirm(t.engineMgr.removeDirConfirm, { title: t.engineMgr.remove, kind: 'warning' })) return
    const next = engineDirs.filter(d => d !== dir)
    setEngineDirs(next)
    scanEngines(next)
  }

  const handleDelete = async (id: string) => {
    const eng = engines.find((e) => e.id === id)
    if (!eng) return
    if (!await confirm(t.engineMgr.removeConfirm, { title: t.engineMgr.remove, kind: 'warning' })) return
    await deleteEngine(id)
    if (defaultEngineId === id) setDefaultEngineId(null)
  }

  const backendColor = (b: string) => {
    switch (b) {
      case 'CUDA': return 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400'
      case 'ROCm': return 'bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-400'
      case 'Vulkan': return 'bg-purple-100 text-purple-800 dark:bg-purple-900/30 dark:text-purple-400'
      default: return 'bg-gray-100 text-gray-800 dark:bg-gray-800 dark:text-gray-300'
    }
  }

  return (
    <div className="space-y-6">
      <h2 className="text-2xl font-bold">{t.nav.engine}</h2>
      <div className="flex items-center gap-4">
          <button onClick={handleScan} disabled={isLoading}
            data-guide="engine-scan"
            className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-400 text-white rounded-lg transition-colors">
          <RefreshCw className={`w-4 h-4 ${isLoading ? 'animate-spin' : ''}`} /> {t.engineMgr.scan}
        </button>
        <button onClick={handleAddDirectory}
          className="flex items-center gap-2 px-4 py-2 bg-green-600 hover:bg-green-700 text-white rounded-lg transition-colors">
          <Plus className="w-4 h-4" /> {t.engineMgr.addDir}
        </button>
      </div>

      {engineDirs.length > 0 && (
        <div className="bg-gray-50 dark:bg-gray-800/50 rounded-lg p-3 text-sm">
          <div className="flex items-center gap-2 mb-2 text-gray-500 font-medium">{t.engineMgr.dirs}</div>
          {engineDirs.map(d => (
            <div key={d} className="flex items-center justify-between py-1 px-2 rounded hover:bg-gray-100 dark:hover:bg-gray-700">
              <span className="text-xs truncate flex-1 mr-2">{d}</span>
              <button onClick={() => handleRemoveDir(d)}
                className="text-xs text-red-500 hover:text-red-700 px-2 py-0.5 rounded hover:bg-red-50 dark:hover:bg-red-900/20">{t.modelRepo.remove}</button>
            </div>
          ))}
        </div>
      )}

      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
        {engines.map((engine) => (
          <div key={engine.id}
            className={`bg-white dark:bg-gray-800 rounded-lg p-5 border ${
              defaultEngineId === engine.id ? 'border-blue-500 ring-2 ring-blue-200 dark:ring-blue-900' : 'border-gray-200 dark:border-gray-700'
            } shadow-sm hover:shadow-md transition-shadow`}>
            <div className="flex items-start justify-between mb-3">
              <div className="flex items-center gap-3">
                <div className="p-2 bg-blue-100 dark:bg-blue-900/30 rounded-lg"><Cpu className="w-5 h-5 text-blue-600 dark:text-blue-400" /></div>
                <div>
                  {editingId === engine.id ? (
                    <input type="text" value={editName}
                      onChange={e => setEditName(e.target.value)}
                      onKeyDown={e => { if (e.key === 'Enter') { renameEngine(engine.id, editName); setEditingId(null) } if (e.key === 'Escape') setEditingId(null) }}
                      onBlur={() => { renameEngine(engine.id, editName); setEditingId(null) }}
                      autoFocus
                      className="font-semibold bg-transparent border-b border-blue-500 outline-none px-1 w-40 dark:text-white" />
                  ) : (
                    <div className="flex items-center gap-1">
                      <h3 className="font-semibold">{engine.name}</h3>
                      <button onClick={() => { setEditingId(engine.id); setEditName(engine.name) }}
                        className="p-0.5 text-gray-400 hover:text-blue-500 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded transition-colors" title="修改名称">
                        <Pencil className="w-3 h-3" />
                      </button>
                    </div>
                  )}
                  <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">{engine.version}</p></div>
              </div>
              <span className={`px-2 py-0.5 rounded-full text-xs font-medium ${backendColor(engine.backend)}`}>{engine.backend}</span>
            </div>
            <p className="text-sm text-gray-600 dark:text-gray-300 mb-4 truncate" title={engine.dir}>{engine.dir}</p>
            <div className="flex items-center justify-between">
              <button onClick={() => setDefaultEngineId(engine.id)}
                className={`px-3 py-1.5 text-sm rounded-lg transition-colors ${defaultEngineId === engine.id ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600'}`}>
                {defaultEngineId === engine.id ? t.engineMgr.defaultEngine : t.engineMgr.setDefault}
              </button>
              <div className="flex items-center gap-2">
                <button onClick={() => openEngineFolder(engine.dir)} className="p-1.5 text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-colors" title={t.engineMgr.openFolder}><FolderOpen className="w-4 h-4" /></button>
                <button onClick={() => handleDelete(engine.id)} className="p-1.5 text-red-600 hover:bg-red-50 dark:hover:bg-red-900/30 rounded-lg transition-colors" title={t.engineMgr.remove}><Trash2 className="w-4 h-4" /></button>
              </div>
            </div>
          </div>
        ))}
        {engines.length === 0 && !isLoading && (
          <div className="col-span-full bg-white dark:bg-gray-800 rounded-lg p-12 text-center border border-gray-200 dark:border-gray-700">
            <Cpu className="w-12 h-12 mx-auto mb-3 text-gray-400" />
            <p className="text-gray-500 dark:text-gray-400">{t.engineMgr.noEngines}</p>
          </div>
        )}
      </div>
    </div>
  )
}

export default EngineManager
