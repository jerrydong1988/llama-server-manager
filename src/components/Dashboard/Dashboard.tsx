import { useState, useEffect } from 'react'
import { Server, Database, Cpu, BarChart3, ArrowRight } from 'lucide-react'
import { invoke } from '@tauri-apps/api/core'
import { useAppStore } from '../../store'
import { useI18n } from '../../i18n'
import StatCard from './StatCard'
import SysResourceBar from './SysResourceBar'
import InstanceRow from './InstanceRow'

export default function Dashboard() {
  const { t } = useI18n()
  const { instances, models, engines, setActiveTab } = useAppStore()
  const [sysMetrics, setSysMetrics] = useState<any>(null)

  const runningInstances = instances.filter(i => i.status === 'running')
  const stoppedInstances = instances.filter(i => i.status !== 'running')

  // Poll system health every 5s — delay first call to avoid startup storm
  useEffect(() => {
    const fetch = () => invoke<any>('get_system_health').then(setSysMetrics).catch(() => {})
    setTimeout(fetch, 500)
    const timer = setInterval(fetch, 5000)
    return () => clearInterval(timer)
  }, [])

  return (
    <div className="flex-1 p-6 overflow-y-auto space-y-6">
      <h1 className="text-2xl font-bold text-slate-900 dark:text-slate-100">{t.dashboard?.title || 'Dashboard'}</h1>

      {/* 统计卡片 */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4" data-guide="dashboard">
        <StatCard icon={<Server className="w-6 h-6" />} label={t.dashboard?.runningInstances || 'Running'} value={runningInstances.length} color="emerald" />
        <StatCard icon={<Database className="w-6 h-6" />} label={t.dashboard?.totalModels || 'Models'} value={models.filter(m => !m.is_shard && m.file_type === 'model').length} color="blue" />
        <StatCard icon={<Cpu className="w-6 h-6" />} label={t.dashboard?.totalEngines || 'Engines'} value={engines.length} color="purple" />
        <StatCard icon={<BarChart3 className="w-6 h-6" />} label={t.dashboard?.totalInstances || 'Instances'} value={instances.length} color="slate" />
      </div>

      {/* 系统资源条 */}
      <SysResourceBar metrics={sysMetrics} />

      {/* 运行中实例 */}
      {runningInstances.length > 0 ? (
        <div className="bg-white dark:bg-slate-800 rounded-xl border border-slate-200 dark:border-slate-700 overflow-hidden">
          <div className="px-5 py-3 border-b border-slate-200 dark:border-slate-700">
            <h2 className="text-sm font-semibold text-slate-900 dark:text-slate-100">
              {t.dashboard?.runningInstances || 'Running Instances'} ({runningInstances.length})
            </h2>
          </div>
          <div className="divide-y divide-slate-100 dark:divide-slate-700/50">
            {runningInstances.map(i => <InstanceRow key={i.id} instance={i} />)}
          </div>
        </div>
      ) : (
        <div className="bg-white dark:bg-slate-800 rounded-xl border border-slate-200 dark:border-slate-700 p-12 text-center">
          <Server className="w-12 h-12 mx-auto mb-3 text-slate-300 dark:text-slate-600" />
          <p className="text-slate-500 mb-3">{t.dashboard?.noRunning || 'No running instances'}</p>
          <button onClick={() => setActiveTab('instances')} className="inline-flex items-center gap-2 px-4 py-2 bg-indigo-600 hover:bg-indigo-700 text-white rounded-lg text-sm transition-colors">
            {t.dashboard?.goToInstances || 'Go to Instances'} <ArrowRight className="w-4 h-4" />
          </button>
        </div>
      )}

      {/* 已停止实例 */}
      {stoppedInstances.length > 0 && (
        <div className="bg-white dark:bg-slate-800 rounded-xl border border-slate-200 dark:border-slate-700 overflow-hidden">
          <div className="px-5 py-3 border-b border-slate-200 dark:border-slate-700">
            <h2 className="text-sm font-semibold text-slate-900 dark:text-slate-100">
              {t.dashboard?.stoppedInstances || 'Stopped Instances'} ({stoppedInstances.length})
            </h2>
          </div>
          <div className="divide-y divide-slate-100 dark:divide-slate-700/50">
            {stoppedInstances.map(i => <InstanceRow key={i.id} instance={i} />)}
          </div>
        </div>
      )}
    </div>
  )
}
