import { Fragment, useEffect, useMemo, useRef, useState } from 'react'
import { ask, open } from '@tauri-apps/plugin-dialog'
import { invoke } from '@tauri-apps/api/core'
import { Check, Copy, Network, Play, Plus, Radio, RefreshCw, Server, Square, StopCircle, Trash2, X, Zap } from 'lucide-react'
import { useAppStore, type WorkerInfo } from '../../store'
import { useI18n } from '../../i18n'
import { Badge, Button, InsetSurface, MetricCard, SectionHeader, SelectInput, Surface, TextInput } from '../ui'

export default function ClusterPage() {
  const { t, lang } = useI18n()
  const zh = lang === 'zh-CN'
  const workers = useAppStore(state => state.workers)
  const clusterScanning = useAppStore(state => state.clusterScanning)
  const setWorkers = useAppStore(state => state.setWorkers)
  const removeWorker = useAppStore(state => state.removeWorker)
  const setClusterScanning = useAppStore(state => state.setClusterScanning)
  const updateWorker = useAppStore(state => state.updateWorker)
  const engines = useAppStore(state => state.engines)
  const defaultEngineId = useAppStore(state => state.defaultEngineId)

  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [showAddDialog, setShowAddDialog] = useState(false)
  const [formData, setFormData] = useState({ host: '', port: 50052, name: '' })
  const [copiedId, setCopiedId] = useState<string | null>(null)
  const [mdnsActive, setMdnsActive] = useState(false)
  const [showLaunchWizard, setShowLaunchWizard] = useState(false)
  const [launchStep, setLaunchStep] = useState(0)
  const [launchForm, setLaunchForm] = useState({
    host: '',
    user: '',
    keyPath: '',
    password: '',
    port: 50052,
    rpcPath: '',
    sshPort: 22,
    remoteOs: 'auto',
  })
  const [localHosts, setLocalHosts] = useState<Set<string>>(new Set(['127.0.0.1', 'localhost', '::1']))
  const [launching, setLaunching] = useState(false)
  const [launchError, setLaunchError] = useState('')
  const [showLocalLaunch, setShowLocalLaunch] = useState(false)
  const [localPort, setLocalPort] = useState(50052)
  const [localEngine, setLocalEngine] = useState('')
  const [localMode, setLocalMode] = useState<'engine' | 'custom'>('engine')
  const scanCancelled = useRef(false)

  useEffect(() => {
    void loadWorkers()
  }, [])

  const loadWorkers = async () => {
    try {
      const result: WorkerInfo[] = await invoke('get_workers')
      setWorkers(result)
    } catch (error) {
      console.error('Failed to load workers:', error)
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
    } catch (error) {
      if (!scanCancelled.current) {
        console.error('Scan failed:', error)
      }
    } finally {
      setClusterScanning(false)
    }
  }

  const handleCancelScan = () => {
    scanCancelled.current = true
    setClusterScanning(false)
  }

  const isLocalWorker = (host: string) => localHosts.has(host)

  useEffect(() => {
    const checkLocalHosts = async () => {
      const hosts = new Set(['127.0.0.1', 'localhost', '::1'])
      for (const worker of workers) {
        if (hosts.has(worker.host)) {
          continue
        }
        try {
          const isLocal: boolean = await invoke('is_local_host', { host: worker.host })
          if (isLocal) {
            hosts.add(worker.host)
          }
        } catch {
          // ignore
        }
      }
      setLocalHosts(hosts)
    }

    void checkLocalHosts()
  }, [workers])

  const handleStopWorker = async (worker: WorkerInfo) => {
    try {
      await invoke('stop_local_worker', { port: worker.port })
      updateWorker(worker.id, { status: 'Offline' })
    } catch (error) {
      console.error('Failed to stop worker:', error)
    }
  }

  const handleLocalLaunch = async () => {
    setLaunching(true)
    setLaunchError('')
    try {
      const engineDir = localEngine || engines.find(engine => engine.id === defaultEngineId)?.dir || ''
      const result: any = await invoke('start_local_rpc', { engineDir: engineDir || null, port: localPort })
      if (result?.ok) {
        const localHost: string = await invoke('get_local_host')
        await invoke('add_worker', { host: localHost, port: localPort, name: `Local-${localPort}` })
        const all: WorkerInfo[] = await invoke('get_workers')
        setWorkers(all)
        setShowLocalLaunch(false)
      }
    } catch (error: any) {
      setLaunchError(typeof error === 'string' ? error : (error?.message || String(error)))
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
        setLaunchError(result?.error || t.clusterPage.launchErrorDefault)
      }
    } catch (error: any) {
      setLaunchError(typeof error === 'string' ? error : (error?.message || String(error)))
    } finally {
      setLaunching(false)
    }
  }

  const handleTest = async (host: string, port: number) => {
    const worker = workers.find(item => item.host === host && item.port === port)
    if (!worker) {
      return
    }
    updateWorker(worker.id, { status: 'Testing' })
    try {
      const result: any = await invoke('test_worker', { host, port })
      updateWorker(worker.id, { status: result.ok ? 'Online' : 'Offline' })
    } catch {
      updateWorker(worker.id, { status: 'Offline' })
    }
  }

  const handleAdd = async () => {
    if (!formData.host.trim()) {
      return
    }
    try {
      await invoke('add_worker', { host: formData.host, port: formData.port, name: formData.name })
      const all: WorkerInfo[] = await invoke('get_workers')
      setWorkers(all)
      setShowAddDialog(false)
      await handleTest(formData.host, formData.port)
    } catch (error) {
      console.error('Failed to add worker:', error)
    }
  }

  const handleDelete = async (worker: WorkerInfo) => {
    const confirmed = await ask(t.clusterPage.confirmDelete, { kind: 'warning' })
    if (!confirmed) {
      return
    }
    try {
      await invoke('remove_worker', { id: worker.id })
      removeWorker(worker.id)
    } catch (error) {
      console.error('Failed to remove worker:', error)
    }
  }

  const handleCopyCmd = async () => {
    try {
      const command: string = await invoke('generate_rpc_launch_cmd', { port: 50052 })
      await navigator.clipboard.writeText(command)
      setCopiedId('cmd')
      setTimeout(() => setCopiedId(null), 2000)
    } catch (error) {
      console.error('Copy failed:', error)
    }
  }

  const toggleExpand = (id: string) => {
    const next = new Set(expanded)
    if (next.has(id)) {
      next.delete(id)
    } else {
      next.add(id)
    }
    setExpanded(next)
  }

  const statusTone = (status: string) => {
    switch (status) {
      case 'Online':
        return 'bg-emerald-400'
      case 'Offline':
        return 'bg-red-400'
      case 'Testing':
        return 'bg-amber-400'
      default:
        return 'bg-slate-500'
    }
  }

  const statusText = (status: string) => {
    switch (status) {
      case 'Online':
        return t.clusterPage.online
      case 'Offline':
        return t.clusterPage.offline
      case 'Testing':
        return t.clusterPage.testing
      default:
        return t.clusterPage.unknown
    }
  }

  const totalVRAM = workers.reduce((sum, worker) => sum + worker.devices.reduce((deviceSum, device) => deviceSum + device.vram_mb, 0), 0)
  const onlineWorkers = workers.filter(worker => worker.status === 'Online').length
  const localWorkers = workers.filter(worker => isLocalWorker(worker.host)).length
  const totalDevices = workers.reduce((sum, worker) => sum + worker.devices.length, 0)
  const workersSorted = useMemo(
    () => [...workers].sort((left, right) => left.name.localeCompare(right.name)),
    [workers],
  )
  const labels = {
    workers: zh ? '\u8282\u70b9' : 'workers',
    subtitle: zh
      ? '\u5728\u4e00\u4e2a\u754c\u9762\u4e2d\u53d1\u73b0 LAN rpc worker\u3001\u6ce8\u518c\u8fdc\u7a0b\u8282\u70b9\uff0c\u5e76\u542f\u52a8\u672c\u5730\u6216 SSH \u7b97\u529b\u3002'
      : 'Discover rpc workers on the LAN, register remote nodes, and bootstrap local or SSH-launched capacity from one place.',
    workerListDesc: zh
      ? '\u76f4\u63a5\u6d4b\u8bd5\u8fde\u63a5\u3001\u67e5\u770b\u8bbe\u5907\u6e05\u5355\uff0c\u5e76\u7ba1\u7406\u672c\u5730 worker\u3002'
      : 'Test connectivity, inspect device inventory, and stop local workers directly from the registry.',
    hideDetails: zh ? '\u6536\u8d77\u8be6\u60c5' : 'Hide Details',
    showDetails: zh ? '\u67e5\u770b\u8be6\u60c5' : 'Show Details',
    clusterNotes: zh ? '\u96c6\u7fa4\u63d0\u793a' : 'Cluster Notes',
    clusterNotesDesc: zh ? '\u7ba1\u7406 worker \u5bb9\u91cf\u65f6\uff0c\u4fdd\u6301\u53d1\u73b0\u72b6\u6001\u548c\u542f\u52a8\u6307\u5f15\u53ef\u89c1\u3002' : 'Keep discovery state and launch guidance visible while you manage worker capacity.',
    auto: zh ? '\u81ea\u52a8' : 'auto',
    devices: zh ? '\u8bbe\u5907' : 'devices',
    host: zh ? '\u4e3b\u673a (IP)' : 'Host (IP)',
    port: zh ? '\u7aef\u53e3' : 'Port',
    workerHost: zh ? 'Worker IP / \u4e3b\u673a' : 'Worker IP / Host',
  }

  return (
    <div className="flex-1 overflow-y-auto p-6" data-testid="cluster-page">
      <div className="mb-6 flex flex-col gap-5 xl:flex-row xl:items-end xl:justify-between">
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <div className="rounded-2xl border border-cyan-500/20 bg-cyan-500/10 p-3 text-cyan-300">
              <Network className="h-5 w-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-3xl font-semibold tracking-tight text-slate-50">{t.clusterPage.title}</h1>
                <Badge tone="slate">
                  {workers.length} {labels.workers}
                </Badge>
              </div>
              <p className="text-sm text-slate-400">{labels.subtitle}</p>
            </div>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <Button
            onClick={handleCopyCmd}
            title={t.clusterPage.copyLaunchCmd}
            icon={copiedId === 'cmd' ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
          >
            {t.clusterPage.copyLaunchCmd}
          </Button>

          {clusterScanning ? (
            <Button
              onClick={handleCancelScan}
              variant="danger"
              icon={<Square className="h-4 w-4" />}
            >
              {t.clusterPage.stopScan}
            </Button>
          ) : (
            <Button
              onClick={handleScan}
              variant="success"
              data-guide="cluster-scan"
              icon={<RefreshCw className="h-4 w-4" />}
            >
              {t.clusterPage.scanLan}
            </Button>
          )}

          <Button
            onClick={() => setShowAddDialog(true)}
            icon={<Plus className="h-4 w-4" />}
          >
            {t.clusterPage.addWorker}
          </Button>

          <Button
            onClick={handleMdnsToggle}
            variant={mdnsActive ? 'primary' : 'secondary'}
            icon={<Radio className="h-4 w-4" />}
          >
            {t.clusterPage.discoverMode}
          </Button>

          <Button
            onClick={() => { setShowLaunchWizard(true); setLaunchStep(0) }}
            variant="violet"
            icon={<Zap className="h-4 w-4" />}
          >
            {t.clusterPage.oneClickLaunch}
          </Button>

          <Button
            onClick={() => { setShowLocalLaunch(true); setLaunchError('') }}
            variant="cyan"
            icon={<Play className="h-4 w-4" />}
          >
            {t.clusterPage.localLaunch}
          </Button>
        </div>
      </div>

      <div className="mb-6 grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          { label: t.clusterPage.online, value: onlineWorkers, tone: 'text-emerald-300 bg-emerald-500/10 border-emerald-500/20' },
          { label: t.clusterPage.localWorker, value: localWorkers, tone: 'text-cyan-300 bg-cyan-500/10 border-cyan-500/20' },
          { label: t.clusterPage.deviceType, value: totalDevices, tone: 'text-violet-300 bg-violet-500/10 border-violet-500/20' },
          { label: t.clusterPage.totalVRAM, value: `${(totalVRAM / 1024).toFixed(1)} GB`, tone: 'text-amber-300 bg-amber-500/10 border-amber-500/20' },
        ].map(card => (
          <MetricCard key={card.label} label={card.label} value={card.value} tone={card.tone} />
        ))}
      </div>

      <div className="grid gap-6 xl:grid-cols-[minmax(0,1fr),320px]">
        <Surface as="section" className="overflow-hidden">
          <div className="border-b border-slate-800 bg-slate-950/90 px-5 py-4">
            <SectionHeader title={t.clusterPage.workerList} description={labels.workerListDesc} />
          </div>

          {workers.length === 0 && !clusterScanning ? (
            <div className="flex min-h-[420px] flex-col items-center justify-center p-10 text-center">
              <Server className="mb-4 h-12 w-12 text-slate-700" />
              <p className="text-base text-slate-300">{t.clusterPage.noWorkers}</p>
            </div>
          ) : (
            <div className="divide-y divide-slate-800 bg-slate-950/30">
              {workersSorted.map(worker => (
                <Fragment key={worker.id}>
                  <div className="px-5 py-4 transition hover:bg-slate-900/70">
                    <div className="flex flex-col gap-4 xl:flex-row xl:items-center xl:justify-between">
                      <div className="min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className={`inline-block h-2.5 w-2.5 rounded-full ${statusTone(worker.status)}`} title={statusText(worker.status)} />
                          <p className="text-sm font-medium text-slate-100">{worker.name}</p>
                          <span className="text-xs text-slate-500">{worker.host}:{worker.port}</span>
                          {worker.auto_discovered && (
                            <Badge tone="slate" className="px-2 py-0.5 text-[11px]">{labels.auto}</Badge>
                          )}
                          {isLocalWorker(worker.host) && (
                            <Badge tone="emerald" className="px-2 py-0.5 text-[11px]">
                              {t.clusterPage.localWorker}
                            </Badge>
                          )}
                        </div>
                        <div className="mt-2 flex flex-wrap items-center gap-4 text-xs text-slate-500">
                          <span>{statusText(worker.status)}</span>
                          <span>{worker.devices.length} {labels.devices}</span>
                          <span>{(worker.devices.reduce((sum, device) => sum + device.vram_mb, 0) / 1024).toFixed(1)} GB VRAM</span>
                        </div>
                      </div>

                      <div className="flex flex-wrap items-center gap-2">
                        <Button
                          onClick={() => void handleTest(worker.host, worker.port)}
                          variant="primary"
                          size="sm"
                        >
                          {t.clusterPage.testConnection}
                        </Button>
                        <Button
                          onClick={() => toggleExpand(worker.id)}
                          size="sm"
                        >
                          {expanded.has(worker.id) ? labels.hideDetails : labels.showDetails}
                        </Button>
                        {isLocalWorker(worker.host) && worker.status === 'Online' && (
                          <Button
                            onClick={() => void handleStopWorker(worker)}
                            variant="danger"
                            size="sm"
                            title={t.clusterPage.stopLocalWorker}
                            icon={<StopCircle className="h-3.5 w-3.5" />}
                          >
                            {t.clusterPage.stopLocalWorker}
                          </Button>
                        )}
                        <Button
                          onClick={() => void handleDelete(worker)}
                          variant="danger"
                          size="sm"
                          title={t.clusterPage.deleteWorker}
                          icon={<Trash2 className="h-3.5 w-3.5" />}
                        >
                          {t.clusterPage.deleteWorker}
                        </Button>
                      </div>
                    </div>
                  </div>

                  {expanded.has(worker.id) && (
                    <div className="bg-slate-950/70 px-5 py-4">
                      {worker.devices.length === 0 ? (
                        <span className="text-sm text-slate-500">{t.clusterPage.noDevices}</span>
                      ) : (
                        <div className="space-y-3">
                          {worker.devices.map((device, index) => {
                            const usedPct = device.vram_mb > 0 ? ((device.vram_mb - device.free_mb) / device.vram_mb) * 100 : 0
                            return (
                              <div key={index} className="rounded-2xl border border-slate-800 bg-slate-900/60 p-4">
                                <div className="mb-2 flex items-center justify-between gap-4">
                                  <div>
                                    <p className="text-sm font-medium text-slate-100">{device.name}</p>
                                    <p className="mt-1 text-xs text-slate-500">{device.device_type}</p>
                                  </div>
                                  <div className="text-right text-xs text-slate-500">
                                    {((device.vram_mb - device.free_mb) / 1024).toFixed(1)} / {(device.vram_mb / 1024).toFixed(1)} GB
                                  </div>
                                </div>
                                <div className="h-2 rounded-full bg-slate-800">
                                  <div className="h-2 rounded-full bg-blue-500" style={{ width: `${usedPct.toFixed(0)}%` }} />
                                </div>
                              </div>
                            )
                          })}
                        </div>
                      )}
                    </div>
                  )}
                </Fragment>
              ))}
            </div>
          )}
        </Surface>

        <Surface as="aside" className="h-fit p-5">
          <div className="mb-5">
            <SectionHeader title={labels.clusterNotes} description={labels.clusterNotesDesc} />
          </div>

          <div className="space-y-4">
            <InsetSurface className="p-4">
              <div className="flex items-center justify-between gap-3">
                <span className="text-sm text-slate-500">{t.clusterPage.discoverMode}</span>
                <Badge tone={mdnsActive ? 'blue' : 'slate'}>
                  {mdnsActive ? t.clusterPage.online : t.clusterPage.offline}
                </Badge>
              </div>
            </InsetSurface>

            <InsetSurface className="p-4">
              <p className="text-sm font-medium text-slate-100">{t.clusterPage.clusterThroughput}</p>
              <div className="mt-3 space-y-3">
                {workers.length === 0 ? (
                  <div className="text-sm text-slate-500">{t.clusterPage.noWorkers}</div>
                ) : (
                  workersSorted.slice(0, 5).map(worker => (
                    <div key={worker.id} className="flex items-center justify-between gap-3">
                      <div className="min-w-0">
                        <p className="truncate text-sm text-slate-200">{worker.name}</p>
                        <p className="mt-1 truncate text-xs text-slate-500">{worker.host}:{worker.port}</p>
                      </div>
                      <span className={`rounded-full px-2 py-1 text-[11px] ${worker.status === 'Online' ? 'bg-emerald-500/10 text-emerald-300' : worker.status === 'Testing' ? 'bg-amber-500/10 text-amber-300' : 'bg-slate-800 text-slate-400'}`}>
                        {statusText(worker.status)}
                      </span>
                    </div>
                  ))
                )}
              </div>
            </InsetSurface>

            <div className="rounded-2xl border border-amber-500/20 bg-amber-500/10 p-4 text-sm leading-6 text-amber-100">
              {t.clusterPage.sshWarning}
            </div>
          </div>
        </Surface>
      </div>

      {showAddDialog && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
          <div className="w-full max-w-md rounded-2xl border border-slate-800 bg-slate-900 shadow-[0_30px_80px_rgba(2,6,23,0.7)]">
            <div className="flex items-center justify-between border-b border-slate-800 px-6 py-4">
              <h3 className="font-semibold text-slate-50">{t.clusterPage.addWorker}</h3>
              <Button onClick={() => setShowAddDialog(false)} variant="subtle" size="icon" aria-label="Close"><X className="h-4 w-4" /></Button>
            </div>
            <div className="space-y-3 p-6">
              <div>
                <label className="mb-1 block text-xs font-medium text-slate-400">{labels.host}</label>
                <TextInput type="text" value={formData.host} onChange={event => setFormData({ ...formData, host: event.target.value })} placeholder="192.168.x.x" className="h-10" />
              </div>
              <div>
                <label className="mb-1 block text-xs font-medium text-slate-400">{labels.port}</label>
                <TextInput type="number" value={formData.port} onChange={event => setFormData({ ...formData, port: parseInt(event.target.value, 10) || 50052 })} className="h-10" />
              </div>
              <div>
                <label className="mb-1 block text-xs font-medium text-slate-400">{t.clusterPage.editWorker}</label>
                <TextInput type="text" value={formData.name} onChange={event => setFormData({ ...formData, name: event.target.value })} placeholder={formData.host || 'Worker'} className="h-10" />
              </div>
            </div>
            <div className="flex justify-end gap-2 border-t border-slate-800 px-6 py-4">
              <Button onClick={() => setShowAddDialog(false)} variant="subtle">{t.common.cancel}</Button>
              <Button onClick={() => void handleAdd()} variant="primary">{t.common.save}</Button>
            </div>
          </div>
        </div>
      )}

      {showLaunchWizard && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
          <div className="w-full max-w-lg rounded-2xl border border-slate-800 bg-slate-900 shadow-[0_30px_80px_rgba(2,6,23,0.7)]">
            <div className="flex items-center justify-between border-b border-slate-800 px-6 py-4">
              <h3 className="font-semibold text-slate-50">{t.clusterPage.launchWizard}</h3>
              <Button onClick={() => { setShowLaunchWizard(false); setLaunchStep(0) }} variant="subtle" size="icon" aria-label="Close"><X className="h-4 w-4" /></Button>
            </div>
            <div className="space-y-4 p-6">
              {launchStep === 0 && (
                <>
                  <div className="rounded-xl border border-amber-500/20 bg-amber-500/10 p-3 text-xs text-amber-200">{t.clusterPage.sshWarning}</div>
                  <div>
                    <label className="mb-1 block text-xs font-medium text-slate-400">{labels.workerHost}</label>
                    <TextInput type="text" value={launchForm.host} onChange={event => setLaunchForm({ ...launchForm, host: event.target.value })} placeholder="192.168.x.x" className="h-10" />
                  </div>
                  <div>
                    <label className="mb-1 block text-xs font-medium text-slate-400">{t.clusterPage.rpcPort}</label>
                    <TextInput type="number" value={launchForm.port} onChange={event => setLaunchForm({ ...launchForm, port: parseInt(event.target.value, 10) || 50052 })} className="h-10" />
                  </div>
                  <div>
                    <label className="mb-1 block text-xs font-medium text-slate-400">{t.clusterPage.sshPort}</label>
                    <TextInput type="number" value={launchForm.sshPort} onChange={event => setLaunchForm({ ...launchForm, sshPort: parseInt(event.target.value, 10) || 22 })} className="h-10" />
                  </div>
                  <div>
                    <label className="mb-1 block text-xs font-medium text-slate-400">{t.clusterPage.remoteOS}</label>
                    <SelectInput value={launchForm.remoteOs} onChange={event => setLaunchForm({ ...launchForm, remoteOs: event.target.value })} className="h-10 w-full">
                      <option value="auto">{t.clusterPage.autoDetect}</option>
                      <option value="linux">Linux</option>
                      <option value="macos">macOS</option>
                      <option value="windows">Windows</option>
                    </SelectInput>
                  </div>
                  <div>
                    <label className="mb-1 block text-xs font-medium text-slate-400">{t.clusterPage.remoteRpcPath}</label>
                    <TextInput type="text" value={launchForm.rpcPath} onChange={event => setLaunchForm({ ...launchForm, rpcPath: event.target.value })} placeholder={t.clusterPage.rpcPathPlaceholder} className="h-10" />
                  </div>
                </>
              )}

              {launchStep === 1 && (
                <>
                  <div>
                    <label className="mb-1 block text-xs font-medium text-slate-400">{t.clusterPage.sshUser}</label>
                    <TextInput type="text" value={launchForm.user} onChange={event => setLaunchForm({ ...launchForm, user: event.target.value })} placeholder="root" className="h-10" />
                  </div>
                  <div>
                    <label className="mb-1 block text-xs font-medium text-slate-400">{t.clusterPage.sshKeyPath}</label>
                    <TextInput type="text" value={launchForm.keyPath} onChange={event => setLaunchForm({ ...launchForm, keyPath: event.target.value })} placeholder="~/.ssh/id_rsa" className="h-10" />
                  </div>
                  <div className="text-xs text-slate-500">Optional: provide a password only when key-based auth is unavailable.</div>
                  <div>
                    <label className="mb-1 block text-xs font-medium text-slate-400">{t.clusterPage.sshPassword}</label>
                    <TextInput type="password" value={launchForm.password} onChange={event => setLaunchForm({ ...launchForm, password: event.target.value })} className="h-10" />
                  </div>
                </>
              )}

              {launchStep === 2 && (
                <div className="space-y-3 text-sm">
                  <div className="flex justify-between"><span className="text-slate-500">Worker</span><span className="text-slate-200">{launchForm.host}:{launchForm.port}</span></div>
                  <div className="flex justify-between"><span className="text-slate-500">SSH</span><span className="text-slate-200">{launchForm.user}@{launchForm.host}:{launchForm.sshPort}</span></div>
                  <div className="rounded-xl border border-blue-500/20 bg-blue-500/10 p-3 text-xs text-blue-200">
                    {t.clusterPage.cmdPreview}: {(launchForm.rpcPath || 'rpc-server')} --host 0.0.0.0 --port {launchForm.port}
                  </div>
                  {launchError && (
                    <div className="whitespace-pre-wrap rounded-xl border border-red-500/20 bg-red-500/10 p-3 text-xs text-red-200">
                      {launchError}
                    </div>
                  )}
                </div>
              )}
            </div>
            <div className="flex justify-between border-t border-slate-800 px-6 py-4">
              <div>
                {launchStep > 0 && (
                  <Button onClick={() => setLaunchStep(step => step - 1)} variant="subtle">{t.clusterPage.prevStep}</Button>
                )}
              </div>
              <div className="flex gap-2">
                <Button onClick={() => { setShowLaunchWizard(false); setLaunchStep(0) }} variant="subtle">{t.common.cancel}</Button>
                {launchStep < 2 ? (
                  <Button onClick={() => setLaunchStep(step => step + 1)} variant="primary">{t.clusterPage.nextStep}</Button>
                ) : (
                  <Button onClick={() => void handleSshLaunch()} disabled={launching} variant="violet">
                    {launching ? t.clusterPage.launching : t.clusterPage.launchWorker}
                  </Button>
                )}
              </div>
            </div>
          </div>
        </div>
      )}

      {showLocalLaunch && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
          <div className="w-full max-w-sm rounded-2xl border border-slate-800 bg-slate-900 shadow-[0_30px_80px_rgba(2,6,23,0.7)]">
            <div className="flex items-center justify-between border-b border-slate-800 px-6 py-4">
              <h3 className="font-semibold text-slate-50">{t.clusterPage.localLaunchTitle}</h3>
              <Button onClick={() => setShowLocalLaunch(false)} variant="subtle" size="icon" aria-label="Close"><X className="h-4 w-4" /></Button>
            </div>
            <div className="space-y-3 p-6">
              <div>
                <label className="mb-1 block text-xs font-medium text-slate-400">{t.clusterPage.rpcPort}</label>
                <TextInput type="number" value={localPort} onChange={event => setLocalPort(parseInt(event.target.value, 10) || 50052)} className="h-10" />
              </div>
              <div>
                <label className="mb-1 block text-xs font-medium text-slate-400">{t.clusterPage.engineDir}</label>
                <div className="mb-2 flex items-center gap-3">
                  <label className={`flex items-center gap-2 text-xs ${localMode === 'engine' ? 'text-blue-300' : 'text-slate-500'}`}>
                    <input type="radio" name="localMode" checked={localMode === 'engine'} onChange={() => setLocalMode('engine')} className="h-3 w-3" />
                    {t.clusterPage.engineMode}
                  </label>
                  <label className={`flex items-center gap-2 text-xs ${localMode === 'custom' ? 'text-blue-300' : 'text-slate-500'}`}>
                    <input type="radio" name="localMode" checked={localMode === 'custom'} onChange={() => setLocalMode('custom')} className="h-3 w-3" />
                    {t.clusterPage.customMode}
                  </label>
                </div>
                {localMode === 'engine' ? (
                  <SelectInput value={localEngine} onChange={event => setLocalEngine(event.target.value)} className="h-10 w-full">
                    {engines.map(engine => (
                      <option key={engine.id} value={engine.dir}>
                        {engine.name}{engine.id === defaultEngineId ? t.clusterPage.defaultEngineLabel : ''}
                      </option>
                    ))}
                  </SelectInput>
                ) : (
                  <div className="flex gap-2">
                    <TextInput type="text" value={localEngine} onChange={event => setLocalEngine(event.target.value)} placeholder={t.clusterPage.engineDirPlaceholder} className="h-10 flex-1" />
                    <Button
                      onClick={async () => {
                        try {
                          const selected = await open({ directory: true, multiple: false })
                          if (selected && typeof selected === 'string') {
                            setLocalEngine(selected)
                          }
                        } catch {
                          // ignore
                        }
                      }}
                      variant="primary"
                      size="sm"
                      title={t.clusterPage.selectEngineDir}
                    >
                      {t.common.browse}
                    </Button>
                  </div>
                )}
              </div>
              {launchError && (
                <div className="rounded-xl border border-red-500/20 bg-red-500/10 p-3 text-xs text-red-200">{launchError}</div>
              )}
            </div>
            <div className="flex justify-end gap-2 border-t border-slate-800 px-6 py-4">
              <Button onClick={() => setShowLocalLaunch(false)} variant="subtle">{t.common.cancel}</Button>
              <Button onClick={() => void handleLocalLaunch()} disabled={launching} variant="cyan">
                {launching ? t.clusterPage.launching : t.instance.start}
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
