import { useState, useMemo } from 'react'
import { useAppStore } from '../../store'
import { Button, TextInput, InsetSurface } from '../ui'

interface Props {
  value: string
  onChange: (v: string) => void
  t: any
}

export default function WorkerSelector({ value, onChange, t }: Props) {
  const { workers } = useAppStore()
  const [manualMode, setManualMode] = useState(false)
  const [manualValue, setManualValue] = useState(value)

  // Parse current rpc_servers into host:port pairs
  const selected = useMemo(() => {
    if (!value) return new Set<string>()
    return new Set(
      value.split(/[, ]+/).filter(Boolean).map(s => {
        const trimmed = s.trim()
        if (trimmed.includes(':')) return trimmed
        return trimmed + ':50052'
      })
    )
  }, [value])

  const onlineWorkers = useMemo(() =>
    workers.filter(w => w.status === 'Online' || w.status === 'Unknown'),
    [workers]
  )

  const toggleWorker = (host: string, port: number) => {
    const addr = `${host}:${port}`
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
    const online = workers.filter(w => w.status === 'Online').map(w => `${w.host}:${w.port}`)
    onChange(online.join(','))
  }

  const selectedList = useMemo(() =>
    Array.from(selected).map(addr => {
      const [h, p] = addr.split(':')
      const worker = workers.find(w => w.host === h && (w.port === parseInt(p) || !p))
      return { addr, host: h, port: p, name: worker?.name || h }
    }),
    [selected, workers]
  )

  if (manualMode) {
    return (
      <div>
        <label className="mb-1 block text-xs font-medium text-slate-400">{`${t.configPage.rpcServers} (--rpc)`}</label>
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
          <div className="mt-2 rounded-lg border border-slate-800 bg-slate-950/60 px-3 py-2 font-mono text-xs text-slate-500">
            {t.clusterPage.cmdPreview}: --rpc {value}
          </div>
        )}
      </div>
    )
  }

  return (
    <div>
      <label className="mb-1 block text-xs font-medium text-slate-400">{t.clusterPage.workerSelector} (--rpc)</label>

      {onlineWorkers.length === 0 ? (
        <InsetSurface className="flex items-center justify-between gap-3 px-3 py-2 text-xs text-slate-500">
          {t.clusterPage.noWorkers}
          <Button onClick={syncFromCluster} variant="subtle" size="sm">{t.clusterPage.syncFromCluster}</Button>
        </InsetSurface>
      ) : (
        <InsetSurface className="max-h-40 space-y-1 overflow-y-auto p-2">
          {onlineWorkers.map(w => {
            const addr = `${w.host}:${w.port}`
            const isChecked = selected.has(addr)
            const statusColor = w.status === 'Online' ? 'bg-emerald-400' : 'bg-slate-500'
            return (
              <label key={w.id} className="flex cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5 text-slate-200 transition hover:bg-slate-900">
                <input
                  type="checkbox"
                  checked={isChecked}
                  onChange={() => toggleWorker(w.host, w.port)}
                  className="w-3.5 h-3.5 rounded"
                />
                <span className={`inline-block w-1.5 h-1.5 rounded-full ${statusColor}`} />
                <span className="min-w-0 flex-1 truncate text-xs">{w.name}</span>
                <span className="text-xs text-slate-500">{addr}</span>
              </label>
            )
          })}
        </InsetSurface>
      )}

      {/* Selected chips */}
      {selectedList.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-1">
          {selectedList.map(s => (
            <span key={s.addr} className="inline-flex items-center gap-1 rounded-md border border-blue-500/20 bg-blue-500/10 px-2 py-0.5 text-xs text-blue-300">
              {s.name}
              <Button onClick={() => toggleWorker(s.host, parseInt(s.port))} variant="subtle" size="sm" className="h-5 px-1 py-0 text-blue-300 hover:text-red-300">&times;</Button>
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
        <div className="mt-2 rounded-lg border border-slate-800 bg-slate-950/60 px-3 py-2 font-mono text-xs text-slate-500">
          {t.clusterPage.cmdPreview}: --rpc {value}
        </div>
      )}
    </div>
  )
}
