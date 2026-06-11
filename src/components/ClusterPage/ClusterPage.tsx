import { useState, useEffect, useRef } from 'react'
import { Network, Plus, Trash2, RefreshCw, Copy, Check, X, ChevronDown, ChevronRight, Square, Radio, Zap, StopCircle, Play } from 'lucide-react'
import { useAppStore, type WorkerInfo } from '../../store'
import { useI18n } from '../../i18n'
import { invoke } from '@tauri-apps/api/core'
import { ask, open } from '@tauri-apps/plugin-dialog'

export default function ClusterPage() {
  const { t } = useI18n()
  const { workers, clusterScanning, setWorkers, removeWorker, setClusterScanning, updateWorker, engines, defaultEngineId } = useAppStore()
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [showAddDialog, setShowAddDialog] = useState(false)
  const [formData, setFormData] = useState({ host: '', port: 50052, name: '' })
  const [copiedId, setCopiedId] = useState<string | null>(null)
  const scanCancelled = useRef(false)
  const [mdnsActive, setMdnsActive] = useState(false)
  const [showLaunchWizard, setShowLaunchWizard] = useState(false)
  const [launchStep, setLaunchStep] = useState(0) // 0=host, 1=ssh, 2=confirm
  const [launchForm, setLaunchForm] = useState({ host: '', user: '', keyPath: '', password: '', port: 50052, rpcPath: '', sshPort: 22, remoteOs: 'auto' })
  const [localHosts, setLocalHosts] = useState<Set<string>>(new Set(['127.0.0.1', 'localhost', '::1']))
  const [launching, setLaunching] = useState(false)
  const [launchError, setLaunchError] = useState('')
  const [showLocalLaunch, setShowLocalLaunch] = useState(false)
  const [localPort, setLocalPort] = useState(50052)
  const [localEngine, setLocalEngine] = useState('')

  // Auto-scan on mount
  useEffect(() => {
    loadWorkers()
  }, [])

  const loadWorkers = async () => {
    try {
      const w: WorkerInfo[] = await invoke('get_workers')
      setWorkers(w)
    } catch (e) {
      console.error('Failed to load workers:', e)
    }
  }

  const handleScan = async () => {
    scanCancelled.current = false
    setClusterScanning(true)
    try {
      const discovered: WorkerInfo[] = await invoke('scan_workers_tcp')
      if (!scanCancelled.current) {
        setWorkers(discovered)
      }
    } catch (e) {
      if (!scanCancelled.current) {
        console.error('Scan failed:', e)
      }
    } finally {
      setClusterScanning(false)
    }
  }

  const handleCancelScan = () => {
    scanCancelled.current = true
    setClusterScanning(false)
  }

  const isLocalWorker = (host: string) => {
    return localHosts.has(host)
  }

  // 启动时批量检查所有 worker 是否本机
  useEffect(() => {
    const check = async () => {
      const hosts = new Set(['127.0.0.1', 'localhost', '::1'])
      for (const w of workers) {
        if (hosts.has(w.host)) continue
        try {
          const isLocal: boolean = await invoke('is_local_host', { host: w.host })
          if (isLocal) hosts.add(w.host)
        } catch {}
      }
      setLocalHosts(hosts)
    }
    check()
  }, [workers])

  const handleStopWorker = async (worker: WorkerInfo) => {
    try {
      await invoke('stop_local_worker', { port: worker.port })
      updateWorker(worker.id, { status: 'Offline' })
    } catch (e) {
      console.error('Failed to stop worker:', e)
    }
  }

  const handleLocalLaunch = async () => {
    setLaunching(true)
    setLaunchError('')
    try {
      let engineDir = localEngine === '__custom__' ? '' : (localEngine || engines.find(e => e.id === defaultEngineId)?.dir || '')
      const result: any = await invoke('start_local_rpc', { engineDir: engineDir || null, port: localPort })
      if (result?.ok) {
        const localHost: string = await invoke('get_local_host')
        await invoke('add_worker', { host: localHost, port: localPort, name: 'Local-' + localPort })
        const all: WorkerInfo[] = await invoke('get_workers')
        setWorkers(all)
        setShowLocalLaunch(false)
      }
    } catch (e: any) {
      setLaunchError(typeof e === 'string' ? e : (e?.message || String(e)))
    } finally {
      setLaunching(false)
    }
  }

  const handleMdnsToggle = async () => {
    if (mdnsActive) {
      await invoke('stop_mdns_discovery')
      setMdnsActive(false)
    } else {
      await invoke('start_mdns_discovery')
      setMdnsActive(true)
    }
  }

  const handleSshLaunch = async () => {
    setLaunching(true)
    setLaunchError('')
    try {
      const result: any = await invoke('ssh_launch_rpc', {
        host: launchForm.host,
        sshUser: launchForm.user,
        sshKeyPath: launchForm.keyPath || null,
        sshPassword: launchForm.password || null,
        rpcPort: launchForm.port,
        remoteRpcPath: launchForm.rpcPath || null,
        sshPort: launchForm.sshPort || 22,
        remoteOs: launchForm.remoteOs || null,
      })
      if (result?.ok) {
        await invoke('add_worker', { host: launchForm.host, port: launchForm.port, name: launchForm.host })
        const all: WorkerInfo[] = await invoke('get_workers')
        setWorkers(all)
        setShowLaunchWizard(false)
        setLaunchStep(0)
      } else {
        setLaunchError(result?.error || '启动失败，未返回错误详情')
      }
    } catch (e: any) {
      setLaunchError(typeof e === 'string' ? e : (e?.message || String(e)))
    } finally {
      setLaunching(false)
    }
  }

  const handleTest = async (host: string, port: number) => {
    const w = workers.find(wr => wr.host === host && wr.port === port)
    if (!w) return
    updateWorker(w.id, { status: 'Testing' })
    try {
      const result: any = await invoke('test_worker', { host, port })
      updateWorker(w.id, { status: result.ok ? 'Online' : 'Offline' })
    } catch {
      updateWorker(w.id, { status: 'Offline' })
    }
  }

  const handleAdd = async () => {
    if (!formData.host.trim()) return
    try {
      await invoke('add_worker', { host: formData.host, port: formData.port, name: formData.name })
      const all: WorkerInfo[] = await invoke('get_workers')
      setWorkers(all)
      setShowAddDialog(false)
      handleTest(formData.host, formData.port)
    } catch (e) {
      console.error('Failed to add worker:', e)
    }
  }

  const handleDelete = async (worker: WorkerInfo) => {
    const confirmed = await ask(t.clusterPage.confirmDelete, { kind: 'warning' })
    if (!confirmed) return
    try {
      await invoke('remove_worker', { id: worker.id })
      removeWorker(worker.id)
    } catch (e) {
      console.error('Failed to remove:', e)
    }
  }

  const handleCopyCmd = async () => {
    try {
      const cmd: string = await invoke('generate_rpc_launch_cmd', { port: 50052 })
      await navigator.clipboard.writeText(cmd)
      setCopiedId('cmd')
      setTimeout(() => setCopiedId(null), 2000)
    } catch (e) {
      console.error('Copy failed:', e)
    }
  }

  const toggleExpand = (id: string) => {
    const n = new Set(expanded)
    if (n.has(id)) n.delete(id); else n.add(id)
    setExpanded(n)
  }

  const statusColor = (status: string) => {
    switch (status) {
      case 'Online': return 'text-green-500'
      case 'Offline': return 'text-red-500'
      case 'Testing': return 'text-yellow-500'
      default: return 'text-gray-400'
    }
  }

  const statusLabel = (status: string) => {
    switch (status) {
      case 'Online': return t.clusterPage.online
      case 'Offline': return t.clusterPage.offline
      case 'Testing': return t.clusterPage.testing
      default: return t.clusterPage.unknown
    }
  }

  const totalVRAM = workers.reduce((sum, w) => sum + w.devices.reduce((s, d) => s + d.vram_mb, 0), 0)

  return (
    <div className="space-y-4" data-testid="cluster-page">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Network className="w-5 h-5 text-blue-500" />
          <span className="text-lg font-semibold">{t.clusterPage.title}</span>
          {workers.length > 0 && <span className="text-sm text-gray-400">({workers.length} workers)</span>}
        </div>
        <div className="flex gap-2">
          <button onClick={handleCopyCmd} className="px-3 py-1.5 bg-blue-600 hover:bg-blue-700 text-white rounded-lg text-xs flex items-center gap-1" title={t.clusterPage.copyLaunchCmd}>
            {copiedId === 'cmd' ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
            {t.clusterPage.copyLaunchCmd}
          </button>
          {clusterScanning ? (
            <button onClick={handleCancelScan} className="px-3 py-1.5 bg-red-600 hover:bg-red-700 text-white rounded-lg text-xs flex items-center gap-1">
              <Square className="w-3 h-3" />
              停止扫描
            </button>
          ) : (
            <button onClick={handleScan} className="px-3 py-1.5 bg-green-600 hover:bg-green-700 text-white rounded-lg text-xs flex items-center gap-1">
              <RefreshCw className="w-3 h-3" />
              {t.clusterPage.scanLan}
            </button>
          )}
          <button onClick={() => setShowAddDialog(true)} className="px-3 py-1.5 bg-gray-600 hover:bg-gray-700 text-white rounded-lg text-xs flex items-center gap-1">
            <Plus className="w-3 h-3" />
            {t.clusterPage.addWorker}
          </button>
          <button onClick={handleMdnsToggle} className={`px-3 py-1.5 rounded-lg text-xs flex items-center gap-1 ${mdnsActive ? 'bg-blue-600 hover:bg-blue-700 text-white' : 'bg-gray-200 dark:bg-gray-700 hover:bg-gray-300 dark:hover:bg-gray-600 text-gray-700 dark:text-gray-300'}`}>
            <Radio className="w-3 h-3" />
            {t.clusterPage.discoverMode}
          </button>
          <button onClick={() => { setShowLaunchWizard(true); setLaunchStep(0) }} className="px-3 py-1.5 bg-purple-600 hover:bg-purple-700 text-white rounded-lg text-xs flex items-center gap-1">
            <Zap className="w-3 h-3" />
            {t.clusterPage.oneClickLaunch}
          </button>
          <button onClick={() => { setShowLocalLaunch(true); setLaunchError('') }} className="px-3 py-1.5 bg-teal-600 hover:bg-teal-700 text-white rounded-lg text-xs flex items-center gap-1">
            <Play className="w-3 h-3" />
            本地启动
          </button>
        </div>
      </div>

      {/* Summary */}
      {workers.length > 0 && (
        <div className="flex gap-4 text-xs text-gray-500 bg-gray-50 dark:bg-gray-800/50 rounded-lg px-4 py-2">
          <span>{t.clusterPage.selectedWorkers}: {workers.filter(w => w.status === 'Online').length}</span>
          <span>{t.clusterPage.totalVRAM}: {(totalVRAM / 1024).toFixed(1)} GB</span>
        </div>
      )}

      {/* Worker List */}
      {workers.length === 0 && !clusterScanning ? (
        <div className="text-center py-12 text-gray-400">
          <Network className="w-10 h-10 mx-auto mb-3 opacity-50" />
          <p>{t.clusterPage.noWorkers}</p>
        </div>
      ) : (
        <div className="border dark:border-gray-700 rounded-lg overflow-hidden">
          <table className="w-full text-sm">
            <thead className="bg-gray-100 dark:bg-gray-800 text-left">
              <tr>
                <th className="px-4 py-2 w-8"></th>
                <th className="px-4 py-2">{t.clusterPage.testConnection}</th>
                <th className="px-4 py-2">{t.clusterPage.workerList}</th>
                <th className="px-4 py-2 hidden md:table-cell">{t.clusterPage.deviceType}</th>
                <th className="px-4 py-2 w-20"></th>
              </tr>
            </thead>
            <tbody>
              {workers.map(w => (
                <>
                  <tr key={w.id} data-testid="worker-row" className="border-t dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800/50">
                    <td className="px-4 py-2">
                      <button onClick={() => toggleExpand(w.id)} className="p-0.5 hover:bg-gray-200 dark:hover:bg-gray-600 rounded">
                        {expanded.has(w.id) ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
                      </button>
                    </td>
                    <td className="px-4 py-2">
                      <button onClick={() => handleTest(w.host, w.port)} className="px-2 py-0.5 bg-blue-100 dark:bg-blue-900/30 text-blue-600 dark:text-blue-400 rounded text-xs hover:bg-blue-200">
                        {t.clusterPage.testConnection}
                      </button>
                    </td>
                    <td className="px-4 py-2">
                      <div className="flex items-center gap-2">
                        <span className={`inline-block w-2 h-2 rounded-full ${statusColor(w.status)}`} title={statusLabel(w.status)} />
                        <span className="font-medium">{w.name}</span>
                        <span className="text-gray-400">{w.host}:{w.port}</span>
                        {w.auto_discovered && <span className="text-gray-400 text-xs">[auto]</span>}
                        {isLocalWorker(w.host) && <span className="text-xs bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-300 px-1.5 py-0.5 rounded">本机</span>}
                      </div>
                    </td>
                    <td className="px-4 py-2 text-gray-400 hidden md:table-cell">
                      {w.devices.length > 0 ? `${w.devices.length} devices` : statusLabel(w.status)}
                    </td>
                    <td className="px-4 py-2">
                      {isLocalWorker(w.host) && w.status === 'Online' && (
                        <button onClick={() => handleStopWorker(w)} className="p-1 text-red-400 hover:text-red-600 rounded mr-1" title="停止本机 Worker">
                          <StopCircle className="w-3.5 h-3.5" />
                        </button>
                      )}
                      <button onClick={() => handleDelete(w)} className="p-1 text-gray-400 hover:text-red-500 rounded" title={t.clusterPage.deleteWorker}>
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </td>
                  </tr>
                  {expanded.has(w.id) && (
                    <tr key={`${w.id}-detail`}>
                      <td colSpan={5} className="px-8 py-3 bg-gray-50 dark:bg-gray-800/30">
                        {w.devices.length === 0 ? (
                          <span className="text-xs text-gray-400">{t.clusterPage.noDevices}</span>
                        ) : (
                          <div className="space-y-2">
                            {w.devices.map((d, i) => {
                              const pct = d.vram_mb > 0 ? ((d.vram_mb - d.free_mb) / d.vram_mb * 100) : 0
                              return (
                                <div key={i} className="flex items-center gap-3 text-xs">
                                  <span className="w-16 text-gray-500">{d.device_type}</span>
                                  <span className="font-medium">{d.name}</span>
                                  <div className="flex-1 max-w-xs">
                                    <div className="bg-gray-200 dark:bg-gray-600 rounded-full h-2">
                                      <div className="bg-blue-500 rounded-full h-2" style={{ width: `${pct.toFixed(0)}%` }} />
                                    </div>
                                  </div>
                                  <span className="text-gray-400">{((d.vram_mb - d.free_mb) / 1024).toFixed(1)} / {(d.vram_mb / 1024).toFixed(1)} GB</span>
                                </div>
                              )
                            })}
                          </div>
                        )}
                      </td>
                    </tr>
                  )}
                </>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Add Worker Dialog */}
      {showAddDialog && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-lg w-full max-w-md">
            <div className="flex items-center justify-between px-6 py-4 border-b dark:border-gray-700">
              <h3 className="font-semibold">{t.clusterPage.addWorker}</h3>
              <button onClick={() => setShowAddDialog(false)} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded"><X className="w-4 h-4" /></button>
            </div>
            <div className="p-6 space-y-3">
              <div>
                <label className="block text-xs font-medium mb-1 text-gray-500">Host (IP)</label>
                <input type="text" value={formData.host} onChange={e => setFormData({ ...formData, host: e.target.value })} placeholder="192.168.1.10" className="w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
              </div>
              <div>
                <label className="block text-xs font-medium mb-1 text-gray-500">Port</label>
                <input type="number" value={formData.port} onChange={e => setFormData({ ...formData, port: parseInt(e.target.value) || 50052 })} className="w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
              </div>
              <div>
                <label className="block text-xs font-medium mb-1 text-gray-500">{t.clusterPage.editWorker} ({t.common.default})</label>
                <input type="text" value={formData.name} onChange={e => setFormData({ ...formData, name: e.target.value })} placeholder={formData.host || 'Worker'} className="w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
              </div>
            </div>
            <div className="flex justify-end gap-2 px-6 py-4 border-t dark:border-gray-700">
              <button onClick={() => setShowAddDialog(false)} className="px-4 py-1.5 text-sm text-gray-600 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg">{t.common.cancel}</button>
              <button onClick={handleAdd} className="px-4 py-1.5 text-sm bg-blue-600 hover:bg-blue-700 text-white rounded-lg">{t.common.save}</button>
            </div>
          </div>
        </div>
      )}
      {/* SSH Launch Wizard */}
      {showLaunchWizard && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-lg w-full max-w-lg">
            <div className="flex items-center justify-between px-6 py-4 border-b dark:border-gray-700">
              <h3 className="font-semibold">{t.clusterPage.launchWizard}</h3>
              <button onClick={() => { setShowLaunchWizard(false); setLaunchStep(0) }} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded"><X className="w-4 h-4" /></button>
            </div>
            <div className="p-6 space-y-4">
              {launchStep === 0 && (
                <>
                  <div className="p-2 mb-2 bg-yellow-50 dark:bg-yellow-900/20 rounded text-xs text-yellow-700 dark:text-yellow-400">
                    ⚠ 请确保远程 rpc-server 版本与主引擎一致。推荐从主引擎目录上传 rpc-server 二进制到远程。
                  </div>
                  <div>
                    <label className="block text-xs font-medium mb-1 text-gray-500">Worker IP / Host</label>
                    <input type="text" value={launchForm.host} onChange={e => setLaunchForm({ ...launchForm, host: e.target.value })} placeholder="192.168.1.10" className="w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
                  </div>
                  <div>
                    <label className="block text-xs font-medium mb-1 text-gray-500">{t.clusterPage.rpcPort}</label>
                    <input type="number" value={launchForm.port} onChange={e => setLaunchForm({ ...launchForm, port: parseInt(e.target.value) || 50052 })} className="w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
                  </div>
                  <div>
                    <label className="block text-xs font-medium mb-1 text-gray-500">SSH 端口</label>
                    <input type="number" value={launchForm.sshPort} onChange={e => setLaunchForm({ ...launchForm, sshPort: parseInt(e.target.value) || 22 })} className="w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
                  </div>
                  <div>
                    <label className="block text-xs font-medium mb-1 text-gray-500">远程系统</label>
                    <select value={launchForm.remoteOs} onChange={e => setLaunchForm({ ...launchForm, remoteOs: e.target.value })} className="w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900">
                      <option value="auto">自动检测</option>
                      <option value="linux">Linux</option>
                      <option value="macos">macOS</option>
                      <option value="windows">Windows</option>
                  </select>
                  <button onClick={async () => {
                    try {
                      const selected = await open({ directory: true, multiple: false })
                      if (selected && typeof selected === 'string') setLocalEngine(selected)
                    } catch {}
                  }} className="px-2 py-1.5 bg-blue-600 hover:bg-blue-700 text-white rounded-lg text-xs shrink-0" title="选择引擎目录">📂</button>
                  </div>
                  <div>
                    <label className="block text-xs font-medium mb-1 text-gray-500">远程 rpc-server 路径（留空用 PATH 默认）</label>
                    <input type="text" value={launchForm.rpcPath} onChange={e => setLaunchForm({ ...launchForm, rpcPath: e.target.value })} placeholder="rpc-server（PATH 默认）" className="w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
                  </div>
                </>
              )}
              {launchStep === 1 && (
                <>
                  <div>
                    <label className="block text-xs font-medium mb-1 text-gray-500">{t.clusterPage.sshUser}</label>
                    <input type="text" value={launchForm.user} onChange={e => setLaunchForm({ ...launchForm, user: e.target.value })} placeholder="root" className="w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
                  </div>
                  <div>
                    <label className="block text-xs font-medium mb-1 text-gray-500">{t.clusterPage.sshKeyPath}</label>
                    <input type="text" value={launchForm.keyPath} onChange={e => setLaunchForm({ ...launchForm, keyPath: e.target.value })} placeholder="~/.ssh/id_rsa" className="w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
                  </div>
                  <div className="text-xs text-gray-400">— {t.common.default} —</div>
                  <div>
                    <label className="block text-xs font-medium mb-1 text-gray-500">SSH 密码</label>
                    <input type="password" value={launchForm.password} onChange={e => setLaunchForm({ ...launchForm, password: e.target.value })} className="w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
                  </div>
                </>
              )}
              {launchStep === 2 && (
                <div className="text-sm space-y-2">
                  <div className="flex justify-between"><span className="text-gray-500">Worker:</span><span>{launchForm.host}:{launchForm.port}</span></div>
                  <div className="flex justify-between"><span className="text-gray-500">SSH:</span><span>{launchForm.user}@{launchForm.host}:{launchForm.sshPort}</span></div>
                  <div className="mt-3 p-3 bg-blue-50 dark:bg-blue-900/20 rounded text-xs text-blue-600 dark:text-blue-400">
                    {t.clusterPage.cmdPreview}: {launchForm.rpcPath || 'rpc-server'} --host 0.0.0.0 --port {launchForm.port}
                  </div>
                  {launchError && (
                    <div className="p-3 bg-red-50 dark:bg-red-900/20 rounded text-xs text-red-600 dark:text-red-400 whitespace-pre-wrap">
                      {'\u26A0'} {launchError}
                    </div>
                  )}
                </div>
              )}
            </div>
            <div className="flex justify-between px-6 py-4 border-t dark:border-gray-700">
              <div>
                {launchStep > 0 && (
                  <button onClick={() => setLaunchStep(s => s - 1)} className="px-4 py-1.5 text-sm text-gray-600 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg">← 上一步</button>
                )}
              </div>
              <div className="flex gap-2">
                <button onClick={() => { setShowLaunchWizard(false); setLaunchStep(0) }} className="px-4 py-1.5 text-sm text-gray-600 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg">{t.common.cancel}</button>
                {launchStep < 2 ? (
                  <button onClick={() => setLaunchStep(s => s + 1)} className="px-4 py-1.5 text-sm bg-blue-600 hover:bg-blue-700 text-white rounded-lg">下一步</button>
                ) : (
                  <button onClick={handleSshLaunch} disabled={launching} className="px-4 py-1.5 text-sm bg-purple-600 hover:bg-purple-700 disabled:bg-gray-400 text-white rounded-lg">
                    {launching ? '启动中...' : '启动 Worker'}
                  </button>
                )}
              </div>
            </div>
          </div>
        </div>
      )}
      {/* Local Launch Dialog */}
      {showLocalLaunch && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-lg w-full max-w-sm">
            <div className="flex items-center justify-between px-6 py-4 border-b dark:border-gray-700">
              <h3 className="font-semibold">本地启动 rpc-server</h3>
              <button onClick={() => setShowLocalLaunch(false)} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded"><X className="w-4 h-4" /></button>
            </div>
            <div className="p-6 space-y-3">
              <div>
                <label className="block text-xs font-medium mb-1 text-gray-500">RPC 端口</label>
                <input type="number" value={localPort} onChange={e => setLocalPort(parseInt(e.target.value) || 50052)} className="w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
              </div>
              <div>
                <label className="block text-xs font-medium mb-1 text-gray-500">引擎目录（默认引擎自动填充）</label>
                <div className="flex gap-1">
                  <select value={localEngine} onChange={e => setLocalEngine(e.target.value)} className="flex-1 px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900">
                    <option value="">{engines.find(e => e.id === defaultEngineId)?.name || '默认引擎'}</option>
                    {engines.map(e => (
                      <option key={e.id} value={e.dir}>{e.name}</option>
                    ))}
                    <option value="__custom__">📂 自定义路径...</option>
                  </select>
                </div>
                {localEngine === '__custom__' && (
                  <input type="text" value="" onChange={e => setLocalEngine(e.target.value)} placeholder="输入 rpc-server 所在目录的完整路径..." className="w-full mt-1 px-3 py-1.5 text-xs border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900" />
                )}
              </div>
              {launchError && (
                <div className="p-2 bg-red-50 dark:bg-red-900/20 rounded text-xs text-red-600 dark:text-red-400">{launchError}</div>
              )}
            </div>
            <div className="flex justify-end gap-2 px-6 py-4 border-t dark:border-gray-700">
              <button onClick={() => setShowLocalLaunch(false)} className="px-4 py-1.5 text-sm text-gray-600 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg">取消</button>
              <button onClick={handleLocalLaunch} disabled={launching} className="px-4 py-1.5 text-sm bg-teal-600 hover:bg-teal-700 disabled:bg-gray-400 text-white rounded-lg">
                {launching ? '启动中...' : '启动'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
