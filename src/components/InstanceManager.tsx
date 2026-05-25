import { useState } from 'react'
import { Play, Square, Plus, Trash2, Copy, Globe, CheckCircle2, XCircle, X, Terminal, Settings, File, Image, FolderOpen, ChevronRight, ChevronDown, Wifi } from 'lucide-react'
import { useAppStore, defaultInstanceConfig } from '../store'
import { invoke } from '@tauri-apps/api/core'
import { useI18n } from '../i18n'

const InstanceManager = () => {
  const { instances, addInstance, deleteInstance, startInstance, stopInstance, openBrowser, generateCommand, models, modelDirs, engines, defaultEngineId, saveConfig, setActiveConfigInstanceId, setActiveTab } = useAppStore()
  const { t } = useI18n()
  const [showCreateModal, setShowCreateModal] = useState(false)
  const [showCmdModal, setShowCmdModal] = useState('')
  const [cmdText, setCmdText] = useState('')
  const [showCreatePicker, setShowCreatePicker] = useState(false)
  const [pickerCollapsed, setPickerCollapsed] = useState<Set<string>>(new Set())
  const [newInst, setNewInst] = useState({ name: '', modelId: '', modelPath: '', mmprojPath: '', port: 8080, engineId: '' })
  const [testResult, setTestResult] = useState('')

  const formatUptime = (startTime?: number) => {
    if (!startTime) return '0 分钟'
    const ms = Date.now() - startTime
    const h = Math.floor(ms / 3600000), m = Math.floor((ms % 3600000) / 60000)
    return h > 0 ? `${h} 小时 ${m} 分钟` : `${m} 分钟`
  }

  const handleCreate = () => {
    const model = models.find(m => m.id === newInst.modelId)
    if (!model || !newInst.modelPath) return
    const id = Math.random().toString(36).substring(2, 11)
    const config = defaultInstanceConfig()
    config.id = id; config.name = newInst.name || model.name.replace('.gguf', '')
    config.model_path = newInst.modelPath; config.mmproj_path = newInst.mmprojPath; config.port = newInst.port; config.host = '127.0.0.1'
    addInstance({ id, name: config.name, status: 'stopped', model: model.name, port: newInst.port, healthCheck: 'pending', config })
    if (!defaultEngineId && engines[0]) useAppStore.setState({ defaultEngineId: engines[0].id })
    setShowCreateModal(false)
    setNewInst({ name: '', modelId: '', modelPath: '', mmprojPath: '', port: newInst.port + 1, engineId: newInst.engineId })
    saveConfig()
  }

  const handleDelete = (id: string) => { if (!confirm(t.instance.confirmDelete)) return; deleteInstance(id); saveConfig() }
  const handleCopyCommand = (text: string) => { navigator.clipboard.writeText(text) }

  const handleTestConnection = async (inst: typeof instances[0]) => {
    setTestResult('testing...')
    try {
      const result = await invoke('test_connection', { host: inst.config.host, port: inst.config.port })
      setTestResult(result as string)
      setTimeout(() => setTestResult(''), 3000)
    } catch (e: any) {
      setTestResult(e?.toString() || '连接失败')
      setTimeout(() => setTestResult(''), 3000)
    }
  }

  const handleShowCommand = async (id: string) => {
    const inst = instances.find(i => i.id === id)
    if (!inst) return
    const engine = engines.find(e => e.id === defaultEngineId) || engines[0]
    if (!engine) return
    try { const cmd = await generateCommand(inst.config, engine.exe); setCmdText(cmd.map(a => a.includes(' ') ? `"${a}"` : a).join(' ')); setShowCmdModal(id) }
    catch (e) { console.error(e) }
  }

  const statusBg = (s: string) => {
    switch (s) { case 'running': return 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400'
      case 'stopped': return 'bg-gray-100 text-gray-800 dark:bg-gray-800 dark:text-gray-300'
      default: return 'bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-400' }
  }

  const healthIcon = (inst: typeof instances[0]) => {
    if (inst.status === 'stopped') return <div className="w-4 h-4 rounded-full border-2 border-gray-400" />
    if (inst.status === 'error') return <XCircle className="w-4 h-4 text-red-500" />
    if (inst.healthCheck === 'ok') return <CheckCircle2 className="w-4 h-4 text-green-500" />
    if (inst.healthCheck === 'fail') return <XCircle className="w-4 h-4 text-red-500" />
    return <div className="w-4 h-4 rounded-full border-2 border-blue-400 border-t-transparent animate-spin" />
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-semibold">{t.instance.title} ({instances.length})</h3>
        <div className="flex items-center gap-4">
          {testResult && (
            <span className={`text-sm ${testResult.startsWith('✓') ? 'text-green-500' : 'text-red-500'}`}>{testResult}</span>
          )}
          <button onClick={() => setShowCreateModal(true)} className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors">
          <Plus className="w-4 h-4" /> {t.instance.create}
        </button>
      </div>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {instances.map(inst => (
          <div key={inst.id} className="bg-white dark:bg-gray-800 rounded-lg p-5 border border-gray-200 dark:border-gray-700 shadow-sm hover:shadow-md transition-shadow">
            <div className="flex items-start justify-between mb-4">
              <div>
                <h4 className="font-semibold text-lg">{inst.name}</h4>
                <div className="flex items-center gap-2 mt-1">
                  <span className={`px-2 py-0.5 rounded-full text-xs font-medium ${statusBg(inst.status)}`}>
                    {inst.status === 'running' ? t.instance.running : inst.status === 'stopped' ? t.instance.stopped : t.instance.error}
                  </span>
                  {inst.status === 'running' && <span className="text-xs text-gray-500">{t.instance.uptime} {formatUptime(inst.startTime)}</span>}
                </div>
              </div>
              {healthIcon(inst)}
            </div>
            <div className="space-y-2 mb-4 text-sm">
              <div className="flex justify-between">                <span className="text-gray-500">{t.instance.model}：</span><span className="truncate max-w-[200px]" title={inst.config.model_path}>{inst.model}</span></div>
              <div className="flex justify-between">                <span className="text-gray-500">{t.instance.port}：</span><span>{inst.config.port}</span></div>
              <div className="flex justify-between">                <span className="text-gray-500">{t.instance.engine}：</span><span className="truncate max-w-[180px]">{engines.find(e => e.id === defaultEngineId)?.name || (engines[0]?.name) || '未选择'}</span></div>
            </div>
            <div className="flex items-center gap-2">
              {inst.status !== 'running' ? (
                <button onClick={() => startInstance(inst.id)} className="flex-1 flex items-center justify-center gap-2 py-2 bg-green-600 hover:bg-green-700 text-white rounded-lg transition-colors">                  <Play className="w-4 h-4" /> {t.instance.start}</button>
              ) : (
                <button onClick={() => stopInstance(inst.id)} className="flex-1 flex items-center justify-center gap-2 py-2 bg-red-600 hover:bg-red-700 text-white rounded-lg transition-colors">                  <Square className="w-4 h-4" /> {t.instance.stop}</button>
              )}
              {inst.status === 'running' && (
                <>
                  <button onClick={() => openBrowser(inst.config.host, inst.config.port)}
                    className="p-2 text-blue-600 hover:bg-blue-50 dark:hover:bg-blue-900/30 rounded-lg transition-colors" title="在浏览器中打开">
                    <Globe className="w-4 h-4" />
                  </button>
                  <button onClick={() => handleTestConnection(inst)}
                    className="p-2 text-gray-600 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-colors" title="测试连接">
                    <Wifi className="w-4 h-4" />
                  </button>
                </>
              )}
              <button onClick={() => handleShowCommand(inst.id)} className="p-2 text-gray-600 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-colors" title="生成命令"><Terminal className="w-4 h-4" /></button>
              <button onClick={() => { setActiveConfigInstanceId(inst.id); setActiveTab('config') }} className="p-2 text-blue-600 hover:bg-blue-50 dark:hover:bg-blue-900/30 rounded-lg transition-colors" title="配置参数"><Settings className="w-4 h-4" /></button>
              <button onClick={() => handleDelete(inst.id)} className="p-2 text-red-600 hover:bg-red-50 dark:hover:bg-red-900/30 rounded-lg transition-colors" title="删除实例"><Trash2 className="w-4 h-4" /></button>
            </div>
          </div>
        ))}
        {instances.length === 0 && (
          <div className="col-span-full bg-white dark:bg-gray-800 rounded-lg p-12 text-center border border-gray-200 dark:border-gray-700">
            <Play className="w-12 h-12 mx-auto mb-3 text-gray-400" /><p className="text-gray-500">暂无实例，请点击「创建实例」</p>
          </div>
        )}
      </div>

      {/* 创建实例弹窗 */}
      {showCreateModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-lg p-6 w-full max-w-md">
            <div className="flex items-center justify-between mb-6"><h3 className="text-lg font-semibold">创建新实例</h3><button onClick={() => setShowCreateModal(false)} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded"><X className="w-5 h-5" /></button></div>
            <div className="space-y-4">
              <div><label className="block text-sm font-medium mb-1">实例名称</label><input type="text" value={newInst.name} onChange={e => setNewInst({ ...newInst, name: e.target.value })} placeholder="例如：Llama-3-8B" className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" /></div>
              <div><label className="block text-sm font-medium mb-1">选择模型</label>
                <div className="flex gap-1">
                  <input type="text" value={models.find(m => m.id === newInst.modelId)?.name || ''} readOnly placeholder="点击右侧按钮选择模型" className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900 text-sm cursor-default" />
                  <button onClick={() => setShowCreatePicker(true)} className="px-3 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg text-sm" title="从模型仓库选择">📂</button>
                </div></div>
              <div><label className="block text-sm font-medium mb-1">选择引擎</label>
                <select value={newInst.engineId || defaultEngineId || ''} onChange={e => setNewInst({ ...newInst, engineId: e.target.value })} className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900">
                  <option value="">系统 PATH</option>{engines.map(e => <option key={e.id} value={e.id}>{e.name}</option>)}
                </select></div>
              <div><label className="block text-sm font-medium mb-1">端口</label><input type="number" min={1} max={65535} value={newInst.port} onChange={e => setNewInst({ ...newInst, port: parseInt(e.target.value) || 8080 })} className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" /></div>
            </div>
            <div className="flex gap-3 mt-6">
              <button onClick={() => setShowCreateModal(false)} className="flex-1 px-4 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg">取消</button>
              <button onClick={handleCreate} disabled={!newInst.modelId} className="flex-1 px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-400 text-white rounded-lg">创建</button>
            </div>
          </div>
        </div>
      )}

      {/* 模型树选择器 */}
      {showCreatePicker && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-[60]">
          <div className="bg-white dark:bg-gray-800 rounded-lg w-full max-w-2xl max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between px-6 py-4 border-b dark:border-gray-700"><h3 className="text-lg font-semibold">从模型仓库选择</h3><button onClick={() => setShowCreatePicker(false)} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded"><X className="w-5 h-5" /></button></div>
            <div className="flex-1 overflow-y-auto p-4">
              {(function TreeRenderer() {
                interface TNode { name: string; path: string; isDir: boolean; children?: Map<string, TNode>; model?: typeof models[0] }
                function buildTree(rootDir: string): TNode {
                  const root: TNode = { name: rootDir, path: rootDir, isDir: true, children: new Map() }
                  const normRoot = rootDir.replace(/\\/g, '\\').toLowerCase()
                  for (const m of models) {
                    const p = m.path.replace(/\\/g, '\\').toLowerCase()
                    if (!p.startsWith(normRoot)) continue
                    const rel = m.path.substring(rootDir.length).replace(/^[\\/]+/, '')
                    if (!rel) continue
                    const parts = rel.split(/[\\/]/)
                    let cur = root
                    for (let i = 0; i < parts.length; i++) {
                      if (i === parts.length - 1) { cur.children!.set(parts[i], { name: parts[i], path: m.path, isDir: false, model: m }) }
                      else {
                        if (!cur.children!.has(parts[i])) { cur.children!.set(parts[i], { name: parts[i], path: cur.path + (cur.path.endsWith('\\') ? '' : '\\') + parts[i], isDir: true, children: new Map() }) }
                        cur = cur.children!.get(parts[i])!
                      }
                    }
                  }
                  return root
                }
                const toggleP = (k: string) => { const n = new Set(pickerCollapsed); if (n.has(k)) n.delete(k); else n.add(k); setPickerCollapsed(n) }
                const pickForCreate = (m: typeof models[0]) => {
                  const dir = m.path.replace(/[/\\][^/\\]*$/, '')
                  const mmproj = models.find(x => { const xDir = x.path.replace(/[/\\][^/\\]*$/, ''); return x.file_type === 'mmproj' && xDir === dir })
                  setNewInst({ ...newInst, modelId: m.id, modelPath: m.path, mmprojPath: mmproj?.path || '' })
                  setShowCreatePicker(false)
                }
                function renderNode(node: TNode, depth: number): any {
                  if (node.isDir) {
                    const c = pickerCollapsed.has(node.path)
                    return (<div key={node.path}>
                      <button onClick={() => toggleP(node.path)} style={{ paddingLeft: `${depth * 12 + 4}px` }} className="w-full flex items-center gap-1.5 py-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded text-left text-xs">
                        {c ? <ChevronRight className="w-3 h-3 text-gray-400 shrink-0" /> : <ChevronDown className="w-3 h-3 text-gray-400 shrink-0" />}
                        {depth === 0 ? <FolderOpen className="w-3 h-3 text-yellow-500 shrink-0" /> : <span className="text-xs shrink-0">📁</span>}
                        <span className="truncate font-medium text-xs">{node.name}</span>
                      </button>
                      {!c && node.children && [...node.children.values()].sort((a, b) => { if (a.isDir !== b.isDir) return a.isDir ? -1 : 1; return a.name.localeCompare(b.name) }).map(ch => renderNode(ch, depth + 1))}
                    </div>)
                  }
                  const m = node.model!
                  if (m.file_type === 'mmproj') return (<div key={node.path} style={{ paddingLeft: `${depth * 12 + 20}px` }} className="flex items-center gap-2 py-1 pr-2 text-xs text-gray-500"><Image className="w-3 h-3 text-purple-500 shrink-0" /><span className="truncate flex-1">{m.name}</span><span className="text-purple-400 shrink-0 text-xs">投影器</span></div>)
                  return (<button key={node.path} onClick={() => pickForCreate(m)} style={{ paddingLeft: `${depth * 12 + 20}px` }} className="w-full flex items-center gap-2 py-1 pr-2 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded text-left text-xs">
                    <File className="w-3 h-3 text-blue-500 shrink-0" /><span className="truncate flex-1">{m.name}</span><span className="text-gray-400 shrink-0">{m.quant_type || ''}</span></button>)
                }
                return modelDirs.map(d => buildTree(d)).map(t => renderNode(t, 0))
              })()}
            </div>
          </div>
        </div>
      )}

      {/* 命令预览弹窗 */}
      {showCmdModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-lg p-6 w-full max-w-3xl">
            <div className="flex items-center justify-between mb-4"><h3 className="text-lg font-semibold">生成的命令行</h3><button onClick={() => setShowCmdModal('')} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded"><X className="w-5 h-5" /></button></div>
            <pre className="bg-gray-900 text-gray-200 p-4 rounded-lg overflow-x-auto text-sm font-mono whitespace-pre-wrap break-all max-h-80 overflow-y-auto">{cmdText}</pre>
            <div className="flex gap-2 mt-4">
              <button onClick={() => handleCopyCommand(cmdText)} className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg"><Copy className="w-4 h-4" /> 复制到剪贴板</button>
              <button onClick={() => { const i = instances.find(x => x.id === showCmdModal); if (i) startInstance(i.id); setShowCmdModal('') }} className="px-4 py-2 bg-green-600 hover:bg-green-700 text-white rounded-lg">▶ 直接启动</button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

export default InstanceManager
