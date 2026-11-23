import { useState, useMemo } from 'react'
import { useAppStore } from '../../store'
import type { Translations } from '../../i18n'
import { Button, TextInput, InsetSurface } from '../ui'
import { formatHostPort, parseHostPort } from '../../utils/network'

interface Props {
  value: string
  onChange: (v: string) => void
  t: Translations
  hideLabel?: boolean
}

export default function WorkerSelector({ value, onChange, t, hideLabel = false }: Props) {
  const workers = useAppStore(state => state.workers)
  const [manualMode, setManualMode] = useState(false)
  const [manualValue, setManualValue] = useState(value)

  // Parse current rpc_servers into host:port pairs
  const selected = useMemo(() => {
    if (!value) return new Set<string>()
    return new Set(
      value.split(/[, ]+/).filter(Boolean).map(s => {
        const trimmed = s.trim()
        const endpoint = parseHostPort(trimmed, 50052)
        return formatHostPort(endpoint.host, endpoint.port)
      })
    )
  }, [value])

  const onlineWorkers = useMemo(() =>
    workers.filter(w => !w.auto_discovered && (w.status === 'Online' || w.status === 'Unknown')),
    [workers]
  )

  const toggleWorker = (host: string, port: number) => {
    const addr = formatHostPort(host, port)
    const current = new Set(selected)
    if (current.has(addr)) {
      current.delete(addr)
    } else {
      current.add(addr)
    }
    const newValue = Array.from(current).join(',')
    onChange(newValue)
  }

  const handleManualChange = (v: string) => {
    setManualValue(v)
    // Debounce sync
    onChange(v)
  }

  const syncFromCluster = () => {
    // Select all online workers
    const online = workers.filter(w => !w.auto_discovered && w.status === 'Online').map(w => formatHostPort(w.host, w.port))
    onChange(online.join(','))
  }

  const selectedList = useMemo(() =>
    Array.from(selected).map(addr => {
      const endpoint = parseHostPort(addr, 50052)
      const worker = workers.find(w => w.host === endpoint.host && w.port === endpoint.port)
      return { addr, host: endpoint.host, port: endpoint.port, name: worker?.name || endpoint.host }
    }),
    [selected, workers]
  )

  if (manualMode) {
    return (
      <div>
        {!hideLabel && <label className="mb-1 block text-xs font-medium text-ui-muted">{`${t.configPage.rpcServers} (--rpc)`}</label>}
        <TextInput
          type="text"
          value={manualValue}
          onChange={e => handleManualChange(e.target.value)}
          className="h-10"
          title={t.configPage.rpcServersTip}
        />
        <Button
          onClick={() => { setManualMode(false); setManualValue(value) }}
          variant="subtle"
          size="sm"
          className="mt-1"
        >
          {t.clusterPage.switchToVisual}
        </Button>
        {value && (
          <div className="mt-2 break-all rounded-lg border border-ui-line bg-ui-inset px-3 py-2 font-mono text-xs text-ui-muted">
            {t.clusterPage.cmdPreview}: --rpc {value}
          </div>
        )}
      </div>
    )
  }

  return (
    <div>
      {!hideLabel && <label className="mb-1 block text-xs font-medium text-ui-muted">{t.clusterPage.workerSelector} (--rpc)</label>}

      {onlineWorkers.length === 0 ? (
        <InsetSurface className="flex items-center justify-between gap-3 px-3 py-2 text-xs text-ui-muted">
          {t.clusterPage.noWorkers}
          <Button onClick={syncFromCluster} variant="subtle" size="sm">{t.clusterPage.syncFromCluster}</Button>
        </InsetSurface>
      ) : (
        <InsetSurface className="max-h-40 space-y-1 overflow-y-auto p-2">
          {onlineWorkers.map(w => {
            const addr = formatHostPort(w.host, w.port)
            const isChecked = selected.has(addr)
            const statusColor = w.status === 'Online' ? 'bg-ui-positive' : 'bg-ui-muted'
            return (
              <label key={w.id} className="flex cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5 text-ui-body transition hover:bg-ui-hover">
                <input
                  type="checkbox"
                  checked={isChecked}
                  onChange={() => toggleWorker(w.host, w.port)}
                  className="w-3.5 h-3.5 rounded"
                />
                <span className={`inline-block w-1.5 h-1.5 rounded-full ${statusColor}`} />
                <span className="min-w-0 flex-1 truncate text-xs">{w.name}</span>
                <span className="min-w-0 max-w-[60%] break-all text-xs text-ui-muted">{addr}</span>
              </label>
            )
          })}
        </InsetSurface>
      )}

      {/* Selected chips */}
      {selectedList.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-1">
          {selectedList.map(s => (
            <span key={s.addr} className="inline-flex min-w-0 max-w-full items-center gap-1 rounded-md border border-ui-line bg-ui-info-bg px-2 py-0.5 text-xs text-ui-info">
              <span className="min-w-0 truncate" title={s.name}>{s.name}</span>
              <Button onClick={() => toggleWorker(s.host, s.port)} variant="subtle" size="sm" className="h-5 shrink-0 px-1 py-0 text-ui-info hover:text-ui-danger">&times;</Button>
            </span>
          ))}
        </div>
      )}

      {/* Controls */}
      <div className="mt-2 flex flex-wrap gap-2">
        <Button onClick={syncFromCluster} variant="subtle" size="sm">
          {t.clusterPage.syncFromCluster}
        </Button>
        <Button
          onClick={() => { setManualMode(true); setManualValue(value) }}
          variant="subtle"
          size="sm"
        >
          {t.clusterPage.switchToManual}
        </Button>
      </div>

      {/* Command preview */}
      {value && (
        <div className="mt-2 break-all rounded-lg border border-ui-line bg-ui-inset px-3 py-2 font-mono text-xs text-ui-muted">
          {t.clusterPage.cmdPreview}: --rpc {value}
        </div>
      )}
    </div>
  )
}
