import { useCallback, useEffect, useState } from 'react'
import { Archive, Database, HardDrive, RefreshCw, ShieldCheck, Trash2 } from 'lucide-react'
import { confirm } from '@tauri-apps/plugin-dialog'
import { invokeApp as invoke } from '../lib/ipc'
import { useI18n } from '../i18n'
import { Badge, Button, MetricCard, SectionHeader, Surface } from './ui'

interface StorageArtifactItem {
  path: string
  bytes: number
  modifiedAt: number | null
  eligible: boolean
  safe: boolean
  reason: string | null
}

interface StorageMaintenanceGroup {
  id: string
  ownership: 'manager' | 'platform'
  action: 'confirm' | 'restart'
  automatic: boolean
  itemCount: number
  eligibleCount: number
  totalBytes: number
  eligibleBytes: number
  items: StorageArtifactItem[]
  warnings: string[]
}

interface AuthorizedDirectory {
  purpose: 'engine' | 'model' | 'download'
  root: string
}

interface ExternalArtifactReference {
  instanceId: string
  instanceName: string
  source: string
  flag: string
  artifactKind: string
  ownership: string
  value: string
  locationKind: string
  exists: boolean | null
  sizeBytes: number | null
}

interface StorageMaintenanceInventory {
  generatedAt: number
  appDataRoot: string
  tempRoot: string
  webviewRoot: string | null
  scheduledWebviewCleanup: boolean
  runningInstanceCount: number
  groups: StorageMaintenanceGroup[]
  authorizedDirectories: AuthorizedDirectory[]
  externalArtifacts: {
    references: ExternalArtifactReference[]
    warnings: string[]
  }
  telemetry: {
    databaseBytes: number
    walBytes: number
    sharedMemoryBytes: number
    totalBytes: number
  }
}

interface StorageCleanupReport {
  groupId: string
  removedItems: number
  removedBytes: number
  skippedItems: number
  failures: string[]
}

interface TelemetryOptimizeReport {
  before: { totalBytes: number }
  after: { totalBytes: number }
  reclaimedBytes: number
}

function formatBytes(value: number) {
  if (!Number.isFinite(value) || value <= 0) return '0 B'
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
  const unit = Math.min(units.length - 1, Math.floor(Math.log(value) / Math.log(1024)))
  const scaled = value / 1024 ** unit
  return `${scaled >= 100 || unit === 0 ? scaled.toFixed(0) : scaled.toFixed(1)} ${units[unit]}`
}

function formatTime(value: number | null, fallback: string) {
  return value ? new Date(value).toLocaleString() : fallback
}

export default function StorageMaintenance() {
  const { t } = useI18n()
  const labels = t.storageMaintenance
  const [inventory, setInventory] = useState<StorageMaintenanceInventory | null>(null)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')
  const [feedback, setFeedback] = useState('')

  const load = useCallback(async () => {
    setLoading(true)
    setError('')
    try {
      setInventory(await invoke<StorageMaintenanceInventory>('get_storage_maintenance_inventory'))
    } catch (loadError) {
      setError(String(loadError))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => { void load() }, [load])

  const groupCopy = (id: string) => {
    const groups = labels.groups as Record<string, { title: string; description: string; warning?: string }>
    return groups[id] || { title: id, description: labels.unknownGroup }
  }

  const clean = async (group: StorageMaintenanceGroup) => {
    const copy = groupCopy(group.id)
    const accepted = await confirm(
      labels.cleanupConfirm
        .replace('{group}', copy.title)
        .replace('{count}', String(group.eligibleCount))
        .replace('{size}', formatBytes(group.eligibleBytes)),
      { title: labels.cleanupTitle, kind: 'warning' },
    )
    if (!accepted) return
    setBusy(group.id)
    setError('')
    setFeedback('')
    try {
      const report = await invoke<StorageCleanupReport>('cleanup_storage_group', { groupId: group.id })
      const summary = labels.cleanupResult
        .replace('{count}', String(report.removedItems))
        .replace('{size}', formatBytes(report.removedBytes))
      await load()
      setFeedback(report.failures.length > 0
        ? `${summary} ${labels.partialFailure.replace('{count}', String(report.failures.length))}`
        : summary)
      if (report.failures.length > 0) setError(report.failures.join('; '))
    } catch (cleanupError) {
      setError(String(cleanupError))
    } finally {
      setBusy('')
    }
  }

  const scheduleWebview = async () => {
    if (!inventory) return
    const enabled = !inventory.scheduledWebviewCleanup
    if (enabled && !await confirm(labels.webviewConfirm, { title: labels.webviewTitle, kind: 'warning' })) return
    setBusy('webview-schedule')
    setError('')
    try {
      await invoke<boolean>('schedule_webview_cache_cleanup', { enabled })
      setFeedback(enabled ? labels.webviewScheduled : labels.webviewCancelled)
      await load()
    } catch (scheduleError) {
      setError(String(scheduleError))
    } finally {
      setBusy('')
    }
  }

  const revoke = async (directory: AuthorizedDirectory) => {
    if (!await confirm(labels.revokeConfirm.replace('{path}', directory.root), { title: labels.revokeTitle, kind: 'warning' })) return
    setBusy(`revoke:${directory.purpose}:${directory.root}`)
    setError('')
    try {
      await invoke<boolean>('revoke_authorized_directory', { purpose: directory.purpose, root: directory.root })
      setFeedback(labels.revoked)
      await load()
    } catch (revokeError) {
      setError(String(revokeError))
    } finally {
      setBusy('')
    }
  }

  const optimizeTelemetry = async () => {
    if (!inventory || !await confirm(labels.telemetryConfirm, { title: labels.telemetryTitle, kind: 'warning' })) return
    setBusy('telemetry')
    setError('')
    try {
      const report = await invoke<TelemetryOptimizeReport>('optimize_telemetry_storage')
      setFeedback(labels.telemetryResult.replace('{size}', formatBytes(report.reclaimedBytes)))
      await load()
    } catch (optimizeError) {
      setError(String(optimizeError))
    } finally {
      setBusy('')
    }
  }

  const groups = inventory?.groups || []
  const managerGroups = groups.filter(group => group.ownership === 'manager')
  const platformGroups = groups.filter(group => group.ownership === 'platform')
  const eligibleBytes = groups.reduce((total, group) => total + group.eligibleBytes, 0)
  const eligibleItems = groups.reduce((total, group) => total + group.eligibleCount, 0)

  const renderGroup = (group: StorageMaintenanceGroup) => {
    const copy = groupCopy(group.id)
    const isWebview = group.id === 'webview-cache'
    const blockedByRunning = group.id === 'private-scratch' && (inventory?.runningInstanceCount || 0) > 0
    return (
      <Surface key={group.id} data-storage-group={group.id} className="min-w-0 p-5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <h3 className="font-semibold text-slate-950 dark:text-slate-50">{copy.title}</h3>
            <p className="mt-1 text-sm leading-6 text-slate-500 dark:text-slate-400">{copy.description}</p>
          </div>
          <Badge tone={group.eligibleCount > 0 ? 'amber' : 'slate'}>{group.eligibleCount} / {group.itemCount}</Badge>
        </div>
        {copy.warning ? <div className="mt-3 rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-xs leading-5 text-amber-900 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-100">{copy.warning}</div> : null}
        {group.warnings.map(warning => <div key={warning} className="mt-2 text-xs text-rose-600 dark:text-rose-300">{warning}</div>)}
        <div className="mt-4 grid grid-cols-2 gap-3 text-sm">
          <div className="rounded-lg bg-slate-50 p-3 dark:bg-slate-950"><div className="text-xs text-slate-500">{labels.total}</div><div className="mt-1 font-semibold">{formatBytes(group.totalBytes)}</div></div>
          <div className="rounded-lg bg-slate-50 p-3 dark:bg-slate-950"><div className="text-xs text-slate-500">{labels.reclaimable}</div><div className="mt-1 font-semibold">{formatBytes(group.eligibleBytes)}</div></div>
        </div>
        {group.items.length > 0 ? (
          <details className="mt-4 text-xs text-slate-500 dark:text-slate-400">
            <summary className="cursor-pointer font-medium">{labels.showItems.replace('{count}', String(group.itemCount))}</summary>
            <div className="mt-2 max-h-44 space-y-2 overflow-y-auto">
              {group.items.map(entry => (
                <div key={entry.path} className="rounded-md border border-slate-200 p-2 dark:border-slate-800">
                  <div className="break-all font-mono text-[11px] text-slate-700 dark:text-slate-200">{entry.path}</div>
                  <div className="mt-1 flex flex-wrap gap-x-3 gap-y-1">
                    <span>{formatBytes(entry.bytes)}</span><span>{formatTime(entry.modifiedAt, labels.unknownTime)}</span>
                    {!entry.safe ? <span className="text-rose-600 dark:text-rose-300">{labels.unsafe}</span> : null}
                  </div>
                </div>
              ))}
            </div>
          </details>
        ) : null}
        <div className="mt-4 flex justify-end">
          {isWebview ? (
            <Button
              data-storage-action="webview-schedule"
              onClick={() => void scheduleWebview()}
              disabled={busy !== '' || (!inventory?.scheduledWebviewCleanup && !inventory?.webviewRoot)}
              variant={inventory?.scheduledWebviewCleanup ? 'secondary' : 'primary'}
            >
              {inventory?.scheduledWebviewCleanup ? labels.cancelScheduled : labels.scheduleRestart}
            </Button>
          ) : (
            <Button
              data-storage-action={`cleanup-${group.id}`}
              onClick={() => void clean(group)}
              disabled={busy !== '' || group.eligibleCount === 0 || blockedByRunning}
              variant="danger"
              icon={<Trash2 className="h-4 w-4" />}
            >
              {busy === group.id ? labels.cleaning : labels.cleanup}
            </Button>
          )}
        </div>
      </Surface>
    )
  }

  return (
    <div className="mx-auto w-full max-w-7xl space-y-6 pb-8">
      <Surface className="p-6">
        <SectionHeader
          title={labels.title}
          description={labels.description}
          action={<Button onClick={() => void load()} disabled={loading || busy !== ''} icon={<RefreshCw className={`h-4 w-4 ${loading ? 'animate-spin' : ''}`} />}>{labels.refresh}</Button>}
        />
        <div className="mt-5 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <MetricCard label={labels.reclaimableItems} value={eligibleItems} icon={<Archive className="h-5 w-5" />} />
          <MetricCard label={labels.reclaimableBytes} value={formatBytes(eligibleBytes)} icon={<HardDrive className="h-5 w-5" />} />
          <MetricCard label={labels.telemetrySize} value={formatBytes(inventory?.telemetry.totalBytes || 0)} icon={<Database className="h-5 w-5" />} />
          <MetricCard label={labels.runningInstances} value={inventory?.runningInstanceCount ?? 0} icon={<ShieldCheck className="h-5 w-5" />} />
        </div>
        {feedback ? <div className="mt-4 rounded-lg border border-emerald-300 bg-emerald-50 px-4 py-3 text-sm text-emerald-800 dark:border-emerald-500/30 dark:bg-emerald-500/10 dark:text-emerald-200">{feedback}</div> : null}
        {error ? <div className="mt-4 rounded-lg border border-rose-300 bg-rose-50 px-4 py-3 text-sm text-rose-800 dark:border-rose-500/30 dark:bg-rose-500/10 dark:text-rose-200">{error}</div> : null}
      </Surface>

      <section>
        <SectionHeader title={labels.managerTitle} description={labels.managerDescription} />
        <div className="mt-4 grid gap-4 xl:grid-cols-2">{managerGroups.map(renderGroup)}</div>
      </section>

      <section>
        <SectionHeader title={labels.platformTitle} description={labels.platformDescription} />
        <div className="mt-4 grid gap-4 xl:grid-cols-2">{platformGroups.map(renderGroup)}</div>
      </section>

      <Surface className="p-6">
        <SectionHeader title={labels.telemetryTitle} description={labels.telemetryDescription} />
        <div className="mt-4 flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
          <div className="grid flex-1 grid-cols-3 gap-3 text-sm">
            <div><div className="text-xs text-slate-500">DB</div><div className="mt-1 font-semibold">{formatBytes(inventory?.telemetry.databaseBytes || 0)}</div></div>
            <div><div className="text-xs text-slate-500">WAL</div><div className="mt-1 font-semibold">{formatBytes(inventory?.telemetry.walBytes || 0)}</div></div>
            <div><div className="text-xs text-slate-500">SHM</div><div className="mt-1 font-semibold">{formatBytes(inventory?.telemetry.sharedMemoryBytes || 0)}</div></div>
          </div>
          <Button data-storage-action="telemetry-optimize" onClick={() => void optimizeTelemetry()} disabled={busy !== '' || !inventory || inventory.runningInstanceCount > 0} variant="primary">{busy === 'telemetry' ? labels.optimizing : labels.optimize}</Button>
        </div>
      </Surface>

      <Surface className="p-6">
        <SectionHeader title={labels.authorizedTitle} description={labels.authorizedDescription} />
        <div className="mt-4 space-y-3">
          {inventory?.authorizedDirectories.length ? inventory.authorizedDirectories.map(directory => {
            const key = `${directory.purpose}:${directory.root}`
            return <div key={key} className="flex flex-col gap-3 rounded-lg border border-slate-200 p-3 sm:flex-row sm:items-center dark:border-slate-800">
              <Badge tone="blue">{labels.purposes[directory.purpose]}</Badge>
              <div className="min-w-0 flex-1 break-all font-mono text-xs text-slate-600 dark:text-slate-300">{directory.root}</div>
              <Button data-storage-action="revoke-directory" onClick={() => void revoke(directory)} disabled={busy !== ''} variant="danger" size="sm">{labels.revoke}</Button>
            </div>
          }) : <div className="text-sm text-slate-500">{labels.noAuthorized}</div>}
        </div>
      </Surface>

      <Surface className="p-6">
        <SectionHeader title={labels.externalTitle} description={labels.externalDescription} />
        {inventory?.externalArtifacts.warnings.map(warning => <div key={warning} className="mt-3 text-xs text-amber-700 dark:text-amber-200">{warning}</div>)}
        <div className="mt-4 space-y-3">
          {inventory?.externalArtifacts.references.length ? inventory.externalArtifacts.references.map((reference, index) => (
            <div key={`${reference.instanceId}:${reference.flag}:${reference.value}:${index}`} className="rounded-lg border border-slate-200 p-3 dark:border-slate-800">
              <div className="flex flex-wrap items-center gap-2"><Badge tone="violet">{reference.flag}</Badge><span className="text-sm font-medium">{reference.instanceName}</span><Badge tone="slate">{labels.operatorOwned}</Badge></div>
              <div className="mt-2 break-all font-mono text-xs text-slate-600 dark:text-slate-300">{reference.value}</div>
            </div>
          )) : <div className="text-sm text-slate-500">{labels.noExternal}</div>}
        </div>
      </Surface>
    </div>
  )
}
