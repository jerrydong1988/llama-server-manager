import { useState } from 'react'
import { useAppStore } from '../store'
import { Trash2 } from 'lucide-react'

const LogsViewer = () => {
  const { instances, logs, clearLogs } = useAppStore()
  const [selectedInstanceId, setSelectedInstanceId] = useState<string>('')

  const instanceLogs = selectedInstanceId ? (logs[selectedInstanceId] || []) : []
  const allLogs = Object.values(logs).flat().sort((a, b) => a.timestamp - b.timestamp)

  return (
    <div className="space-y-4">
      {/* 实例选择器 */}
      <div className="flex items-center gap-4">
        <label className="text-sm font-medium">查看实例日志：</label>
        <select
          value={selectedInstanceId}
          onChange={(e) => setSelectedInstanceId(e.target.value)}
          className="px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800 min-w-[250px]"
        >
          <option value="">所有实例</option>
          {instances.map((inst) => (
            <option key={inst.id} value={inst.id}>
              {inst.name} {inst.status === 'running' ? '(运行中)' : ''} - 端口 {inst.config.port}
            </option>
          ))}
        </select>

        {selectedInstanceId && (
          <button
            onClick={() => clearLogs(selectedInstanceId)}
            className="flex items-center gap-1 px-3 py-2 text-red-600 hover:bg-red-50 dark:hover:bg-red-900/20 rounded-lg transition-colors text-sm"
          >
            <Trash2 className="w-4 h-4" /> 清空日志
          </button>
        )}
      </div>

      {/* 日志内容 */}
      <div className="bg-gray-900 dark:bg-gray-950 p-4 rounded-lg h-[500px] overflow-y-auto font-mono text-sm leading-relaxed">
        {(selectedInstanceId ? instanceLogs : allLogs).length === 0 ? (
          <p className="text-gray-500 text-center py-8">
            {instances.length === 0
              ? '暂无实例运行，请先创建并启动一个实例'
              : selectedInstanceId
                ? '该实例暂无日志输出'
                : '暂无日志，启动实例后日志将实时显示在这里'}
          </p>
        ) : (
          (selectedInstanceId ? instanceLogs : allLogs).map((entry, idx) => {
            const time = new Date(entry.timestamp).toLocaleTimeString()
            const instName = instances.find((i) => i.id === entry.instanceId)?.name || entry.instanceId
            const text = entry.text

            // Keyword highlighting
            let colorClass = 'text-gray-300'
            const lower = text.toLowerCase()
            if (/error|fail|panic|fatal/.test(lower)) colorClass = 'text-red-400'
            else if (/warn|warning/.test(lower)) colorClass = 'text-yellow-400'
            else if (/listening|ready|启动|就绪|ok|success|loaded/.test(lower)) colorClass = 'text-green-400'
            else if (/token|speed|t\/s/.test(lower)) colorClass = 'text-cyan-400'

            return (
              <div key={idx} className={`${colorClass} whitespace-pre-wrap break-all`}>
                {!selectedInstanceId && (
                  <span className="text-gray-500">[{time}] [{instName}] </span>
                )}
                {text}
              </div>
            )
          })
        )}
      </div>

      <p className="text-xs text-gray-500 dark:text-gray-400">
        日志实时显示，启动实例后自动捕获输出。选择实例可查看分组日志。
      </p>
    </div>
  )
}

export default LogsViewer
