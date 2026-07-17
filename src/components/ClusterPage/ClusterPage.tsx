import { Fragment, useEffect, useMemo, useRef, useState } from 'react'
import { ask, open } from '@tauri-apps/plugin-dialog'
import { invokeApp as invoke } from '../../lib/ipc'
import { Check, Copy, Network, Play, Plus, Radio, RefreshCw, Server, Square, StopCircle, Trash2, X, Zap } from 'lucide-react'
import { useAppStore, type WorkerInfo } from '../../store'
import { useI18n } from '../../i18n'
import { getClusterLabels } from '../../i18n/pageLabels'
import { Badge, Button, InsetSurface, MetricCard, SectionHeader, SelectInput, Surface, TextInput } from '../ui'

export default function ClusterPage() {
  const { t, lang } = useI18n()
  const labels = useMemo(() => getClusterLabels(lang), [lang])
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
  const dialogRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!showAddDialog && !showLaunchWizard && !showLocalLaunch) return

    const dialog = dialogRef.current
    if (!dialog) return
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const focusableSelector = [
      'button:not([disabled])',
      'input:not([disabled])',
      'select:not([disabled])',
      'textarea:not([disabled])',
      '[href]',
      '[tabindex]:not([tabindex="-1"])',
    ].join(',')

    const closeDialog = () => {
      if (showLocalLaunch) {
        setShowLocalLaunch(false)
      } else if (showLaunchWizard) {
        setShowLaunchWizard(false)
        setLaunchStep(0)
      } else {
        setShowAddDialog(false)
      }
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
        closeDialog()
        return
      }
      if (event.key !== 'Tab') return

      const focusable = Array.from(dialog.querySelectorAll<HTMLElement>(focusableSelector))
        .filter(element => element.offsetParent !== null)
      if (focusable.length === 0) {
        event.preventDefault()
        dialog.focus()
        return
      }

      const first = focusable[0]
      const last = focusable[focusable.length - 1]
      const active = document.activeElement
      if (event.shiftKey && (active === first || !dialog.contains(active))) {
        event.preventDefault()
        last.focus()
      } else if (!event.shiftKey && active === last) {
        event.preventDefault()
        first.focus()
      }
    }

    dialog.addEventListener('keydown', handleKeyDown)
    const focusFrame = window.requestAnimationFrame(() => {
      const first = dialog.querySelector<HTMLElement>(focusableSelector)
      ;(first || dialog).focus()
    })

    return () => {
      window.cancelAnimationFrame(focusFrame)
      dialog.removeEventListener('keydown', handleKeyDown)
      if (previousFocus?.isConnected) previousFocus.focus()
    }
  }, [showAddDialog, showLaunchWizard, showLocalLaunch])

  useEffect(() => {
    void invoke<WorkerInfo[]>('get_workers')
      .then(setWorkers)
      .catch(error => console.error('Failed to load workers:', error))
  }, [setWorkers])

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
    if (!launchForm.keyPath.trim()) {
      setLaunchError(labels.selectSshKey)
      setLaunchStep(1)
      return
    }
    setLaunching(true)
    setLaunchError('')
    try {
      const result: any = await invoke('ssh_launch_rpc', {
        host: launchForm.host,
        sshUser: launchForm.user,
        sshKeyPath: launchForm.keyPath || null,
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
  return (
    <div className="space-y-5" data-testid="cluster-page">
      <div className="flex flex-col gap-5 2xl:flex-row 2xl:items-end 2xl:justify-between">
        <div className="space-y-3">
          <div className="flex items-center gap-3">
            <div className="rounded-lg border border-cyan-500/20 bg-cyan-500/10 p-3 text-cyan-300">
              <Network className="h-5 w-5" />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h1 className="text-2xl font-semibold tracking-tight text-slate-50">{t.clusterPage.title}</h1>
                <Badge tone="slate">
                  {workers.length} {labels.workers}
                </Badge>
              </div>
              <p className="mt-1 max-w-3xl text-sm text-slate-400">{labels.subtitle}</p>
            </div>
          </div>
        </div>

        <div className="grid w-full gap-2 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] 2xl:w-auto 2xl:min-w-[680px]">
          <div className="grid min-w-0 grid-cols-2 gap-2 rounded-lg border border-slate-800 bg-slate-950/60 p-1.5">
            {clusterScanning ? (
              <Button
                onClick={handleCancelScan}
                variant="danger"
                className="min-w-0 whitespace-nowrap"
                icon={<Square className="h-4 w-4" />}
              >
                {t.clusterPage.stopScan}
              </Button>
            ) : (
              <Button
                onClick={handleScan}
                variant="success"
                data-guide="cluster-scan"
                className="min-w-0 whitespace-nowrap"
                icon={<RefreshCw className="h-4 w-4" />}
              >
                {t.clusterPage.scanLan}
              </Button>
            )}

            <Button
              onClick={() => setShowAddDialog(true)}
              className="min-w-0 whitespace-nowrap"
              icon={<Plus className="h-4 w-4" />}
            >
              {t.clusterPage.addWorker}
            </Button>
          </div>

          <div className="grid min-w-0 grid-cols-2 gap-2 rounded-lg border border-slate-800 bg-slate-950/60 p-1.5">
            <Button
              onClick={() => { setShowLocalLaunch(true); setLaunchError('') }}
              variant="cyan"
              className="min-w-0 whitespace-nowrap"
              icon={<Play className="h-4 w-4" />}
            >
              {t.clusterPage.localLaunch}
            </Button>

            <Button
              onClick={() => { setShowLaunchWizard(true); setLaunchStep(0) }}
              variant="violet"
              className="min-w-0 whitespace-nowrap"
              icon={<Zap className="h-4 w-4" />}
            >
              {t.clusterPage.oneClickLaunch}
            </Button>
          </div>

          <div className="grid grid-cols-2 gap-2 rounded-lg border border-slate-800 bg-slate-950/60 p-1.5">
            <Button
              onClick={handleCopyCmd}
              size="icon"
              title={t.clusterPage.copyLaunchCmd}
              aria-label={t.clusterPage.copyLaunchCmd}
            >
              {copiedId === 'cmd' ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
            </Button>

            <Button
              onClick={handleMdnsToggle}
              variant={mdnsActive ? 'primary' : 'secondary'}
              size="icon"
              title={t.clusterPage.discoverMode}
              aria-label={t.clusterPage.discoverMode}
            >
              <Radio className="h-4 w-4" />
            </Button>
          </div>
        </div>
      </div>

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {[
          { label: t.clusterPage.online, value: onlineWorkers, tone: 'text-emerald-300 bg-emerald-500/10 border-emerald-500/20' },
          { label: t.clusterPage.localWorker, value: localWorkers, tone: 'text-cyan-300 bg-cyan-500/10 border-cyan-500/20' },
          { label: t.clusterPage.deviceType, value: totalDevices, tone: 'text-violet-300 bg-violet-500/10 border-violet-500/20' },
          { label: t.clusterPage.totalVRAM, value: `${(totalVRAM / 1024).toFixed(1)} GB`, tone: 'text-amber-300 bg-amber-500/10 border-amber-500/20' },
        ].map(card => (
          <MetricCard key={card.label} label={card.label} value={card.value} tone={card.tone} />
        ))}
      </div>

      <div className="grid gap-6 2xl:grid-cols-[minmax(0,1fr)_320px]">
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
                    <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_auto] xl:items-center">
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

                      <div className="flex w-full shrink-0 items-center justify-end gap-2 overflow-x-auto pb-1 xl:w-[420px] xl:overflow-visible xl:pb-0">
                        <Button
                          onClick={() => void handleTest(worker.host, worker.port)}
                          variant="primary"
                          size="sm"
                          className="w-[108px] shrink-0 whitespace-nowrap"
                        >
                          {t.clusterPage.testConnection}
                        </Button>
                        <Button
                          onClick={() => toggleExpand(worker.id)}
                          size="sm"
                          className="w-[108px] shrink-0 whitespace-nowrap"
                        >
                          {expanded.has(worker.id) ? labels.hideDetails : labels.showDetails}
                        </Button>
                        <div className="flex w-[88px] shrink-0 items-center justify-end gap-2">
                          {isLocalWorker(worker.host) && worker.status === 'Online' ? (
                            <Button
                              onClick={() => void handleStopWorker(worker)}
                              variant="danger"
                              size="icon"
                              className="h-8 w-8"
                              title={t.clusterPage.stopLocalWorker}
                              aria-label={t.clusterPage.stopLocalWorker}
                            >
                              <StopCircle className="h-3.5 w-3.5" />
                            </Button>
                          ) : (
                            <span className="h-8 w-8" aria-hidden="true" />
                          )}
                          <Button
                            onClick={() => void handleDelete(worker)}
                            variant="danger"
                            size="icon"
                            className="h-8 w-8"
                            title={t.clusterPage.deleteWorker}
                            aria-label={t.clusterPage.deleteWorker}
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </Button>
                        </div>
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
                              <div key={index} className="rounded-lg border border-slate-800 bg-slate-900/60 p-4">
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

            <div className="rounded-lg border border-amber-200 bg-amber-50 p-4 text-sm leading-6 text-amber-800 dark:border-amber-500/20 dark:bg-amber-500/10 dark:text-amber-200">
              {t.clusterPage.sshWarning}
            </div>
          </div>
        </Surface>
      </div>

      {showAddDialog && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
          onMouseDown={event => { if (event.target === event.currentTarget) setShowAddDialog(false) }}
        >
          <div ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby="add-worker-dialog-title" tabIndex={-1} className="w-full max-w-md rounded-lg border border-slate-800 bg-slate-900 shadow-[0_30px_80px_rgba(2,6,23,0.7)]">
            <div className="flex items-center justify-between border-b border-slate-800 px-6 py-4">
              <h3 id="add-worker-dialog-title" className="font-semibold text-slate-50">{t.clusterPage.addWorker}</h3>
              <Button onClick={() => setShowAddDialog(false)} variant="subtle" size="icon" aria-label={t.common.cancel}><X className="h-4 w-4" /></Button>
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
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
          onMouseDown={event => { if (event.target === event.currentTarget) { setShowLaunchWizard(false); setLaunchStep(0) } }}
        >
          <div ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby="launch-worker-dialog-title" tabIndex={-1} className="w-full max-w-lg rounded-lg border border-slate-800 bg-slate-900 shadow-[0_30px_80px_rgba(2,6,23,0.7)]">
            <div className="flex items-center justify-between border-b border-slate-800 px-6 py-4">
              <h3 id="launch-worker-dialog-title" className="font-semibold text-slate-50">{t.clusterPage.launchWizard}</h3>
              <Button onClick={() => { setShowLaunchWizard(false); setLaunchStep(0) }} variant="subtle" size="icon" aria-label={t.common.cancel}><X className="h-4 w-4" /></Button>
            </div>
            <div className="space-y-4 p-6">
              {launchStep === 0 && (
                <>
                  <div className="rounded-lg border border-amber-200 bg-amber-50 p-3 text-xs text-amber-800 dark:border-amber-500/20 dark:bg-amber-500/10 dark:text-amber-200">{t.clusterPage.sshWarning}</div>
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
                  <div className="text-xs text-slate-500">
                    {labels.sshKeyNotice}
                  </div>
                </>
              )}

              {launchStep === 2 && (
                <div className="space-y-3 text-sm">
                  <div className="flex justify-between"><span className="text-slate-500">Worker</span><span className="text-slate-200">{launchForm.host}:{launchForm.port}</span></div>
                  <div className="flex justify-between"><span className="text-slate-500">SSH</span><span className="text-slate-200">{launchForm.user}@{launchForm.host}:{launchForm.sshPort}</span></div>
                  <div className="rounded-lg border border-blue-500/20 bg-blue-500/10 p-3 text-xs text-blue-200">
                    {t.clusterPage.cmdPreview}: {(launchForm.rpcPath || 'rpc-server')} --host 0.0.0.0 --port {launchForm.port}
                  </div>
                  {launchError && (
                    <div className="whitespace-pre-wrap rounded-lg border border-red-500/20 bg-red-500/10 p-3 text-xs text-red-200">
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
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
          onMouseDown={event => { if (event.target === event.currentTarget) setShowLocalLaunch(false) }}
        >
          <div ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby="local-worker-dialog-title" tabIndex={-1} className="w-full max-w-sm rounded-lg border border-slate-800 bg-slate-900 shadow-[0_30px_80px_rgba(2,6,23,0.7)]">
            <div className="flex items-center justify-between border-b border-slate-800 px-6 py-4">
              <h3 id="local-worker-dialog-title" className="font-semibold text-slate-50">{t.clusterPage.localLaunchTitle}</h3>
              <Button onClick={() => setShowLocalLaunch(false)} variant="subtle" size="icon" aria-label={t.common.cancel}><X className="h-4 w-4" /></Button>
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
                <div className="rounded-lg border border-red-500/20 bg-red-500/10 p-3 text-xs text-red-200">{launchError}</div>
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
