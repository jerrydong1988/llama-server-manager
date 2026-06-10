import { useState, useMemo } from 'react'
import { useAppStore } from '../../store'

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
        <label className="block text-xs font-medium mb-1 text-gray-500">{`${t.configPage.rpcServers} (--rpc)`}</label>
        <input
          type="text"
          value={manualValue}
          onChange={e => handleManualChange(e.target.value)}
          className="w-full px-3 py-1.5 text-sm border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-900"
          title={t.configPage.rpcServersTip}
        />
        <button
          onClick={() => { setManualMode(false); setManualValue(value) }}
          className="text-xs text-blue-500 hover:text-blue-600 mt-1"
        >
          {t.clusterPage.switchToVisual}
        </button>
        {value && (
          <div className="mt-1 text-xs text-gray-400 font-mono">
            {t.clusterPage.cmdPreview}: --rpc {value}
          </div>
        )}
      </div>
    )
  }

  return (
    <div>
      <label className="block text-xs font-medium mb-1 text-gray-500">{t.clusterPage.workerSelector} (--rpc)</label>

      {onlineWorkers.length === 0 ? (
        <div className="text-xs text-gray-400 py-2">
          {t.clusterPage.noWorkers}
          <button onClick={syncFromCluster} className="ml-2 text-blue-500 hover:text-blue-600">{t.clusterPage.syncFromCluster}</button>
        </div>
      ) : (
        <div className="space-y-1 max-h-40 overflow-y-auto border dark:border-gray-700 rounded-lg p-2 bg-white dark:bg-gray-900">
          {onlineWorkers.map(w => {
            const addr = `${w.host}:${w.port}`
            const isChecked = selected.has(addr)
            const statusColor = w.status === 'Online' ? 'text-green-500' : 'text-gray-400'
            return (
              <label key={w.id} className="flex items-center gap-2 py-0.5 cursor-pointer hover:bg-gray-50 dark:hover:bg-gray-800 rounded px-1">
                <input
                  type="checkbox"
                  checked={isChecked}
                  onChange={() => toggleWorker(w.host, w.port)}
                  className="w-3.5 h-3.5 rounded"
                />
                <span className={`inline-block w-1.5 h-1.5 rounded-full ${statusColor}`} />
                <span className="text-xs flex-1">{w.name}</span>
                <span className="text-xs text-gray-400">{addr}</span>
              </label>
            )
          })}
        </div>
      )}

      {/* Selected chips */}
      {selectedList.length > 0 && (
        <div className="flex flex-wrap gap-1 mt-2">
          {selectedList.map(s => (
            <span key={s.addr} className="inline-flex items-center gap-1 px-2 py-0.5 bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-300 rounded-full text-xs">
              {s.name}
              <button onClick={() => toggleWorker(s.host, parseInt(s.port))} className="hover:text-red-500">&times;</button>
            </span>
          ))}
        </div>
      )}

      {/* Controls */}
      <div className="flex gap-2 mt-1">
        <button onClick={syncFromCluster} className="text-xs text-blue-500 hover:text-blue-600">
          {t.clusterPage.syncFromCluster}
        </button>
        <button
          onClick={() => { setManualMode(true); setManualValue(value) }}
          className="text-xs text-gray-400 hover:text-gray-600"
        >
          {t.clusterPage.switchToManual}
        </button>
      </div>

      {/* Command preview */}
      {value && (
        <div className="mt-1 text-xs text-gray-400 font-mono">
          {t.clusterPage.cmdPreview}: --rpc {value}
        </div>
      )}
    </div>
  )
}
