import { useState } from 'react'
import { useAppStore } from '../store'
import { Trash2 } from 'lucide-react'
import { useI18n } from '../i18n'

const LogsViewer = () => {
  const { instances, logs, clearLogs } = useAppStore()
  const { t } = useI18n()
  const [selectedInstanceId, setSelectedInstanceId] = useState<string>('')

  const instanceLogs = selectedInstanceId ? (logs[selectedInstanceId] || []) : []
  const allLogs = Object.values(logs).flat().sort((a, b) => a.timestamp - b.timestamp)

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-4">
        <label className="text-sm font-medium">{t.logs.selectInstance}</label>
        <select value={selectedInstanceId} onChange={(e) => setSelectedInstanceId(e.target.value)}
          className="px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800 min-w-[250px]">
          <option value="">{t.logs.allInstances}</option>
          {instances.map((inst) => (
            <option key={inst.id} value={inst.id}>
              {inst.name} {inst.status === 'running' ? t.logs.runningTag : ''}
            </option>
          ))}
        </select>
        {selectedInstanceId && (
          <button onClick={() => clearLogs(selectedInstanceId)}
            className="flex items-center gap-1 px-3 py-2 text-red-600 hover:bg-red-50 dark:hover:bg-red-900/20 rounded-lg transition-colors text-sm">
            <Trash2 className="w-4 h-4" /> {t.logs.clear}
          </button>
        )}
      </div>

      <div className="bg-gray-900 dark:bg-gray-950 p-4 rounded-lg h-[500px] overflow-y-auto font-mono text-sm leading-relaxed">
        {(selectedInstanceId ? instanceLogs : allLogs).length === 0 ? (
          <p className="text-gray-500 text-center py-8">
            {instances.length === 0 ? t.logs.noInstances : selectedInstanceId ? t.logs.noLogsForInstance : t.logs.noLogs}
          </p>
        ) : (
          (selectedInstanceId ? instanceLogs : allLogs).map((entry, idx) => {
            const time = new Date(entry.timestamp).toLocaleTimeString()
            const instName = instances.find((i) => i.id === entry.instanceId)?.name || entry.instanceId
            const text = entry.text
            let colorClass = 'text-gray-300'
            const lower = text.toLowerCase()
            if (/error|fail|panic|fatal/.test(lower)) colorClass = 'text-red-400'
            else if (/warn|warning/.test(lower)) colorClass = 'text-yellow-400'
            else if (/listening|ready|ok|success|loaded/.test(lower)) colorClass = 'text-green-400'
            else if (/token|speed|t\/s/.test(lower)) colorClass = 'text-cyan-400'
            return (
              <div key={idx} className={`${colorClass} whitespace-pre-wrap break-all`}>
                {!selectedInstanceId && <span className="text-gray-500">[{time}] [{instName}] </span>}
                {text}
              </div>
            )
          })
        )}
      </div>
      <p className="text-xs text-gray-500 dark:text-gray-400">{t.logs.hint}</p>
    </div>
  )
}

export default LogsViewer
