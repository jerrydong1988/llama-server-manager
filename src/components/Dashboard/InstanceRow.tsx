import type { Instance } from '../../store/types'
import { useAppStore } from '../../store'
import { useI18n } from '../../i18n'

export default function InstanceRow({ instance }: { instance: Instance }) {
  const { t } = useI18n()
  const { startInstance, stopInstance } = useAppStore()
  const isRunning = instance.status === 'running'

  return (
    <div className="flex items-center justify-between px-4 py-2.5 hover:bg-slate-50 dark:hover:bg-slate-700/50 rounded-lg transition-colors">
      <div className="flex items-center gap-3 min-w-0">
        <div className={`w-2.5 h-2.5 rounded-full shrink-0 ${isRunning ? 'bg-emerald-500 animate-pulse' : 'bg-slate-400'}`} />
        <div className="min-w-0">
          <span className="font-medium text-sm text-slate-900 dark:text-slate-100">{instance.name}</span>
          <span className="ml-3 text-xs text-slate-400 truncate">{instance.model} · :{instance.config.port}</span>
        </div>
      </div>
      <div className="flex items-center gap-2 shrink-0 ml-2">
        {isRunning ? (
          <button onClick={() => stopInstance(instance.id)} className="px-3 py-1 text-xs bg-red-100 dark:bg-red-900/30 text-red-700 dark:text-red-400 rounded-lg hover:bg-red-200 dark:hover:bg-red-900/50 transition-colors">
            {t.instance.stop}
          </button>
        ) : (
          <button onClick={() => startInstance(instance.id)} className="px-3 py-1 text-xs bg-emerald-100 dark:bg-emerald-900/30 text-emerald-700 dark:text-emerald-400 rounded-lg hover:bg-emerald-200 dark:hover:bg-emerald-900/50 transition-colors">
            {t.instance.start}
          </button>
        )}
      </div>
    </div>
  )
}
