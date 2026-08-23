import { Fragment, useEffect, useMemo, useRef, useState } from 'react'
import { ask } from '@tauri-apps/plugin-dialog'
import { Network, Play, Server, StopCircle, Trash2, X, Zap } from 'lucide-react'
import { invokeApp as invoke } from '../../lib/ipc'
import { useAppStore, type WorkerInfo } from '../../store'
import { useI18n } from '../../i18n'
import { getClusterLabels } from '../../i18n/pageLabels'
import { formatPathForDisplay } from '../../utils/path'
import { Badge, Button, InsetSurface, MetricCard, SectionHeader, Surface, TextInput } from '../ui'
import ResidencySchedulerPanel from './ResidencySchedulerPanel'

const errorMessage = (error: unknown) => error instanceof Error ? error.message : String(error)

type WorkerAgentStatus = { rpc_running: boolean }
type WorkerAgentAuditEntry = {
  sequence: number
  timestamp: string
  event: string
  outcome: string
  detail: string
  hash: string
}

export default function ClusterPage() {
  const { t, lang } = useI18n()
  const labels = useMemo(() => getClusterLabels(lang), [lang])
  const workers = useAppStore(state => state.workers)
  const setWorkers = useAppStore(state => state.setWorkers)
  const removeWorker = useAppStore(state => state.removeWorker)
  const updateWorker = useAppStore(state => state.updateWorker)
  const addRuntimeWarning = useAppStore(state => state.addRuntimeWarning)
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [showAgentDialog, setShowAgentDialog] = useState(false)
  const [launching, setLaunching] = useState(false)
  const [launchError, setLaunchError] = useState('')
  const [agentAudit, setAgentAudit] = useState<Record<string, WorkerAgentAuditEntry[]>>({})
  const [agentAuditLoading, setAgentAuditLoading] = useState<string | null>(null)
  const [agentForm, setAgentForm] = useState({
    name: '',
    controlHost: '',
    controlPort: 7443,
    tunnelHost: '',
    tunnelPort: 7444,
    tlsServerName: '',
    tlsCertPath: '',
    tokenPath: '',
    localPort: 0,
  })
  const dialogRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    void invoke<WorkerInfo[]>('get_workers')
      .then(setWorkers)
      .catch(error => addRuntimeWarning(`${labels.workerLoadFailed}: ${errorMessage(error)}`))
  }, [addRuntimeWarning, labels.workerLoadFailed, setWorkers])

  useEffect(() => {
    if (!showAgentDialog) return
    const dialog = dialogRef.current
    if (!dialog) return
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const focusable = dialog.querySelector<HTMLElement>('button:not([disabled]),input:not([disabled])')
    focusable?.focus()
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
        setShowAgentDialog(false)
      }
    }
    dialog.addEventListener('keydown', handleKeyDown)
    return () => {
      dialog.removeEventListener('keydown', handleKeyDown)
      if (previousFocus?.isConnected) previousFocus.focus()
    }
  }, [showAgentDialog])

  const refreshWorkers = async () => {
    const all = await invoke<WorkerInfo[]>('get_workers')
    setWorkers(all)
  }

  const handleAgentEnroll = async () => {
    if (!agentForm.controlHost.trim() || !agentForm.tunnelHost.trim() || !agentForm.tlsServerName.trim()
      || !agentForm.tlsCertPath.trim() || !agentForm.tokenPath.trim()) {
      setLaunchError(t.clusterPage.agentSecurityNote)
      return
    }
    setLaunching(true)
    setLaunchError('')
    try {
      await invoke<WorkerInfo>('enroll_worker_agent', { enrollment: agentForm })
      await refreshWorkers()
      setShowAgentDialog(false)
    } catch (error) {
      setLaunchError(errorMessage(error))
    } finally {
      setLaunching(false)
    }
  }

  const handleTest = async (worker: WorkerInfo) => {
    updateWorker(worker.id, { status: 'Testing' })
    try {
      await invoke<WorkerAgentStatus>('test_worker_agent', { id: worker.id })
      await refreshWorkers()
    } catch (error) {
      updateWorker(worker.id, { status: 'Offline' })
      addRuntimeWarning(`secure Agent test failed: ${errorMessage(error)}`)
    }
  }

  const handleStop = async (worker: WorkerInfo) => {
    try {
      await invoke<WorkerAgentStatus>('stop_worker_agent', { id: worker.id })
      await refreshWorkers()
    } catch (error) {
      addRuntimeWarning(`secure Agent stop failed: ${errorMessage(error)}`)
    }
  }

  const handleDelete = async (worker: WorkerInfo) => {
    if (!await ask(t.clusterPage.confirmDelete, { kind: 'warning' })) return
    try {
      await invoke('remove_worker', { id: worker.id })
      removeWorker(worker.id)
    } catch (error) {
      addRuntimeWarning(`secure Agent removal failed: ${errorMessage(error)}`)
    }
  }

  const handleLoadAudit = async (worker: WorkerInfo) => {
    setAgentAuditLoading(worker.id)
    try {
      const entries = await invoke<WorkerAgentAuditEntry[]>('list_worker_agent_audit', { id: worker.id, limit: 20 })
      setAgentAudit(current => ({ ...current, [worker.id]: entries }))
    } catch (error) {
      addRuntimeWarning(`secure Agent audit failed: ${errorMessage(error)}`)
    } finally {
      setAgentAuditLoading(null)
    }
  }

  const toggleExpand = (id: string) => {
    setExpanded(current => {
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const statusTone = (status: string) => status === 'Online'
    ? 'bg-emerald-400'
    : status === 'Offline'
      ? 'bg-red-400'
      : status === 'Testing'
        ? 'bg-amber-400'
        : 'bg-slate-500'
  const statusText = (status: string) => status === 'Online'
    ? t.clusterPage.online
    : status === 'Offline'
      ? t.clusterPage.offline
      : status === 'Testing'
        ? t.clusterPage.testing
        : t.clusterPage.unknown
  const sorted = useMemo(() => [...workers].sort((a, b) => a.name.localeCompare(b.name)), [workers])
  const online = workers.filter(worker => worker.status === 'Online').length
  const devices = workers.reduce((sum, worker) => sum + worker.devices.length, 0)
  const totalVram = workers.reduce(
    (sum, worker) => sum + worker.devices.reduce((deviceSum, device) => deviceSum + device.vram_mb, 0),
    0,
  )

  return (
    <div className="space-y-5" data-testid="cluster-page">
      <div className="flex flex-col gap-5 xl:flex-row xl:items-end xl:justify-between">
        <div className="flex items-center gap-3">
          <div className="rounded-lg border border-violet-500/20 bg-violet-500/10 p-3 text-violet-300">
            <Network className="h-5 w-5" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h1 className="text-2xl font-semibold tracking-tight text-slate-50">{t.clusterPage.title}</h1>
              <Badge tone="violet">{workers.length} Secure Agents</Badge>
            </div>
            <p className="mt-1 max-w-3xl text-sm text-slate-400">{t.clusterPage.agentSecurityNote}</p>
          </div>
        </div>
        <Button
          data-guide="cluster-agent"
          onClick={() => { setShowAgentDialog(true); setLaunchError('') }}
          variant="violet"
          icon={<Zap className="h-4 w-4" />}
        >
          {t.clusterPage.secureAgent}
        </Button>
      </div>

      <div className="grid gap-4 md:grid-cols-3">
        <MetricCard label={t.clusterPage.online} value={online} tone="text-emerald-300 bg-emerald-500/10 border-emerald-500/20" />
        <MetricCard label={t.clusterPage.deviceType} value={devices} tone="text-violet-300 bg-violet-500/10 border-violet-500/20" />
        <MetricCard label={t.clusterPage.totalVRAM} value={`${(totalVram / 1024).toFixed(1)} GB`} tone="text-amber-300 bg-amber-500/10 border-amber-500/20" />
      </div>

      <ResidencySchedulerPanel />

      <Surface as="section" className="overflow-hidden">
        <div className="border-b border-slate-800 bg-slate-950/90 px-5 py-4">
          <SectionHeader title={t.clusterPage.workerList} description={t.clusterPage.agentSecurityNote} />
        </div>
        {workers.length === 0 ? (
          <div className="flex min-h-[360px] flex-col items-center justify-center p-10 text-center">
            <Server className="mb-4 h-12 w-12 text-slate-700" />
            <p className="text-base text-slate-300">{t.clusterPage.noWorkers}</p>
          </div>
        ) : (
          <div className="divide-y divide-slate-800 bg-slate-950/30">
            {sorted.map(worker => (
              <Fragment key={worker.id}>
                <div className="px-5 py-4">
                  <div className="flex flex-col gap-4 xl:flex-row xl:items-center xl:justify-between">
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className={`h-2.5 w-2.5 rounded-full ${statusTone(worker.status)}`} />
                        <span className="text-sm font-medium text-slate-100">{worker.name}</span>
                        <Badge tone="violet">{t.clusterPage.agentBadge}</Badge>
                        <span className="text-xs text-slate-500">{worker.host}:{worker.port}</span>
                      </div>
                      <p className="mt-2 text-xs text-slate-500">{statusText(worker.status)} · {worker.devices.length} {labels.devices}</p>
                    </div>
                    <div className="flex flex-wrap items-center gap-2">
                      <Button onClick={() => void handleTest(worker)} size="sm" variant="primary">{t.clusterPage.testConnection}</Button>
                      <Button onClick={() => toggleExpand(worker.id)} size="sm">{expanded.has(worker.id) ? labels.hideDetails : labels.showDetails}</Button>
                      {worker.status === 'Online' ? (
                        <Button onClick={() => void handleStop(worker)} variant="danger" size="icon" title={t.clusterPage.stopAgent}>
                          <StopCircle className="h-4 w-4" />
                        </Button>
                      ) : (
                        <Button disabled variant="success" size="icon" title={t.clusterPage.agentComputeUnavailable}>
                          <Play className="h-4 w-4" />
                        </Button>
                      )}
                      <Button onClick={() => void handleDelete(worker)} variant="danger" size="icon" title={t.clusterPage.deleteWorker}>
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  </div>
                </div>
                {expanded.has(worker.id) && worker.agent && (
                  <div className="space-y-4 bg-slate-950/70 px-5 py-4">
                    <InsetSurface className="p-4">
                      <div className="grid gap-2 text-xs text-violet-100 md:grid-cols-2">
                        <span className="truncate">Agent: {worker.agent.agent_id}</span>
                        <span className="truncate">TLS: {worker.agent.tls_server_name}</span>
                        <span className="truncate md:col-span-2">{t.clusterPage.certificateFingerprint}: {worker.agent.certificate_sha256}</span>
                      </div>
                      <div className="mt-3 flex items-center justify-between gap-3">
                        <span className="text-xs font-medium text-violet-200">{t.clusterPage.auditLog}</span>
                        <Button onClick={() => void handleLoadAudit(worker)} variant="violet" size="sm" disabled={agentAuditLoading === worker.id}>{t.clusterPage.loadAudit}</Button>
                      </div>
                      {agentAudit[worker.id] && (
                        <div className="mt-3 max-h-48 space-y-2 overflow-y-auto">
                          {agentAudit[worker.id].map(entry => (
                            <div key={entry.sequence} className="rounded border border-violet-500/10 bg-slate-950/40 px-3 py-2 text-xs">
                              <div className="flex flex-wrap justify-between gap-2 text-violet-200">
                                <span>#{entry.sequence} {entry.event} · {entry.outcome}</span>
                                <span>{new Date(entry.timestamp).toLocaleString()}</span>
                              </div>
                              <p className="mt-1 text-slate-400">{entry.detail}</p>
                            </div>
                          ))}
                        </div>
                      )}
                    </InsetSurface>
                  </div>
                )}
              </Fragment>
            ))}
          </div>
        )}
      </Surface>

      {showAgentDialog && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onMouseDown={event => { if (event.target === event.currentTarget) setShowAgentDialog(false) }}>
          <div ref={dialogRef} role="dialog" aria-modal="true" aria-labelledby="agent-worker-dialog-title" tabIndex={-1} className="max-h-[90vh] w-full max-w-2xl overflow-y-auto rounded-lg border border-violet-500/30 bg-slate-900 shadow-[0_30px_80px_rgba(2,6,23,0.7)]">
            <div className="flex items-center justify-between border-b border-slate-800 px-6 py-4">
              <h3 id="agent-worker-dialog-title" className="font-semibold text-slate-50">{t.clusterPage.agentDialogTitle}</h3>
              <Button onClick={() => setShowAgentDialog(false)} variant="subtle" size="icon" aria-label={t.common.cancel}><X className="h-4 w-4" /></Button>
            </div>
            <div className="space-y-4 p-6">
              <div className="rounded-lg border border-violet-500/20 bg-violet-500/10 p-3 text-xs leading-5 text-violet-200">{t.clusterPage.agentSecurityNote}</div>
              <div className="grid gap-3 md:grid-cols-2">
                <div className="md:col-span-2"><label className="mb-1 block text-xs text-slate-400">{t.clusterPage.agentName}</label><TextInput value={agentForm.name} onChange={event => setAgentForm({ ...agentForm, name: event.target.value })} /></div>
                <div><label className="mb-1 block text-xs text-slate-400">{t.clusterPage.controlHost}</label><TextInput value={agentForm.controlHost} onChange={event => setAgentForm({ ...agentForm, controlHost: event.target.value })} /></div>
                <div><label className="mb-1 block text-xs text-slate-400">{t.clusterPage.controlPort}</label><TextInput type="number" value={agentForm.controlPort} onChange={event => setAgentForm({ ...agentForm, controlPort: parseInt(event.target.value, 10) || 7443 })} /></div>
                <div><label className="mb-1 block text-xs text-slate-400">{t.clusterPage.tunnelHost}</label><TextInput value={agentForm.tunnelHost} onChange={event => setAgentForm({ ...agentForm, tunnelHost: event.target.value })} /></div>
                <div><label className="mb-1 block text-xs text-slate-400">{t.clusterPage.tunnelPort}</label><TextInput type="number" value={agentForm.tunnelPort} onChange={event => setAgentForm({ ...agentForm, tunnelPort: parseInt(event.target.value, 10) || 7444 })} /></div>
                <div className="md:col-span-2"><label className="mb-1 block text-xs text-slate-400">{t.clusterPage.tlsServerName}</label><TextInput value={agentForm.tlsServerName} onChange={event => setAgentForm({ ...agentForm, tlsServerName: event.target.value })} /></div>
                <div className="md:col-span-2"><label className="mb-1 block text-xs text-slate-400">{t.clusterPage.tlsCertPath}</label><TextInput value={formatPathForDisplay(agentForm.tlsCertPath)} onChange={event => setAgentForm({ ...agentForm, tlsCertPath: event.target.value })} /></div>
                <div className="md:col-span-2"><label className="mb-1 block text-xs text-slate-400">{t.clusterPage.tokenPath}</label><TextInput type="password" value={formatPathForDisplay(agentForm.tokenPath)} onChange={event => setAgentForm({ ...agentForm, tokenPath: event.target.value })} /></div>
                <div><label className="mb-1 block text-xs text-slate-400">{t.clusterPage.localBridgePort}</label><TextInput type="number" value={agentForm.localPort} onChange={event => setAgentForm({ ...agentForm, localPort: Math.max(0, parseInt(event.target.value, 10) || 0) })} /></div>
              </div>
              {launchError && <div className="whitespace-pre-wrap rounded-lg border border-red-500/20 bg-red-500/10 p-3 text-xs text-red-200">{launchError}</div>}
            </div>
            <div className="flex justify-end gap-2 border-t border-slate-800 px-6 py-4">
              <Button onClick={() => setShowAgentDialog(false)} variant="subtle">{t.common.cancel}</Button>
              <Button onClick={() => void handleAgentEnroll()} disabled={launching} variant="violet">{launching ? t.clusterPage.launching : t.clusterPage.enrollAgent}</Button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
