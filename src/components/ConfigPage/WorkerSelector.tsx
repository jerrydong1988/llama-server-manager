import { useMemo } from 'react'
import { useAppStore } from '../../store'
import type { Translations } from '../../i18n'
import { InsetSurface } from '../ui'
import { formatHostPort, parseHostPort } from '../../utils/network'

interface Props {
  value: string
  onChange: (value: string) => void
  t: Translations
  hideLabel?: boolean
}

/**
 * Distributed inference is selectable only through a cryptographically
 * enrolled Secure Worker Agent. Manual host:port trust was intentionally
 * removed because rpc-server has no peer identity of its own.
 */
export default function WorkerSelector({ value, onChange, t, hideLabel = false }: Props) {
  const workers = useAppStore(state => state.workers)
  const selected = useMemo(() => new Set(
    value
      .split(/[, ]+/)
      .filter(Boolean)
      .map(entry => {
        const endpoint = parseHostPort(entry.trim(), 50052)
        return formatHostPort(endpoint.host, endpoint.port)
      }),
  ), [value])
  const agents = useMemo(
    () => workers.filter(worker => worker.origin === 'agent' && worker.agent && worker.status === 'Online'),
    [workers],
  )

  const toggle = (host: string, port: number) => {
    const address = formatHostPort(host, port)
    const next = new Set(selected)
    if (next.has(address)) next.delete(address)
    else next.add(address)
    onChange(Array.from(next).join(','))
  }

  return (
    <div>
      {!hideLabel && <label className="mb-1 block text-xs font-medium text-slate-400">{t.clusterPage.workerSelector} (--rpc)</label>}
      {agents.length === 0 ? (
        <InsetSurface className="px-3 py-3 text-xs text-slate-500">
          {t.clusterPage.agentComputeUnavailable}
        </InsetSurface>
      ) : (
        <InsetSurface className="max-h-40 space-y-1 overflow-y-auto p-2">
          {agents.map(worker => {
            const address = formatHostPort(worker.host, worker.port)
            return (
              <label key={worker.id} className="flex cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5 text-slate-200 transition hover:bg-slate-900">
                <input
                  type="checkbox"
                  checked={selected.has(address)}
                  onChange={() => toggle(worker.host, worker.port)}
                  className="h-3.5 w-3.5 rounded"
                />
                <span className="h-1.5 w-1.5 rounded-full bg-emerald-400" />
                <span className="min-w-0 flex-1 truncate text-xs">{worker.name}</span>
                <span className="text-xs text-slate-500">{address}</span>
              </label>
            )
          })}
        </InsetSurface>
      )}
    </div>
  )
}
