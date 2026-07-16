import { useMemo } from 'react'
import { Activity, BarChart3, Download, Package, Play, RefreshCw, Route, Server, Square, Terminal, Wrench } from 'lucide-react'
import { formatMessage, useI18n } from '../../i18n'
import { useAppStore } from '../../store'
import type { CommandAction, ProductIssue } from './CommandCenter'

export function useCommandCenterModel() {
  const instances = useAppStore(state => state.instances)
  const models = useAppStore(state => state.models)
  const engines = useAppStore(state => state.engines)
  const downloadTasks = useAppStore(state => state.downloadTasks)
  const setActiveTab = useAppStore(state => state.setActiveTab)
  const loadInitialData = useAppStore(state => state.loadInitialData)
  const startInstance = useAppStore(state => state.startInstance)
  const stopInstance = useAppStore(state => state.stopInstance)
  const { t } = useI18n()
  const copy = t.commandCenter
  const actionsCopy = copy.actions

  const productIssues = useMemo<ProductIssue[]>(() => {
    const issues: ProductIssue[] = []
    const unhealthy = instances.filter(instance => instance.status === 'running' && instance.healthCheck === 'fail').length
    const errored = instances.filter(instance => instance.status === 'error').length
    const modelCount = models.filter(model => !model.is_shard && model.file_type === 'model').length
    const failedDownloads = Object.values(downloadTasks).filter(task => task.status === 'error').length

    if (engines.length === 0) issues.push({ id: 'no-engines', title: copy.noEnginesTitle, description: copy.noEnginesDescription, severity: 'critical', actionLabel: copy.openEngines, action: () => setActiveTab('engine') })
    if (modelCount === 0) issues.push({ id: 'no-models', title: copy.noModelsTitle, description: copy.noModelsDescription, severity: 'warning', actionLabel: copy.openModels, action: () => setActiveTab('model-repo') })
    if (instances.length === 0) issues.push({ id: 'no-instances', title: copy.noInstancesTitle, description: copy.noInstancesDescription, severity: 'info', actionLabel: copy.createInstance, action: () => setActiveTab('instances') })
    if (errored > 0 || unhealthy > 0) issues.push({
      id: 'instance-health', title: copy.unhealthyTitle,
      description: formatMessage(copy.unhealthyDescription, { errored, unhealthy }),
      severity: 'critical', actionLabel: copy.reviewInstances, action: () => setActiveTab('instances'),
    })
    if (failedDownloads > 0) issues.push({
      id: 'failed-downloads', title: copy.failedDownloadsTitle,
      description: formatMessage(copy.failedDownloadsDescription, { count: failedDownloads }),
      severity: 'warning', actionLabel: copy.openDownloads, action: () => setActiveTab('downloads'),
    })
    return issues
  }, [copy, downloadTasks, engines.length, instances, models, setActiveTab])

  const commandActions = useMemo<CommandAction[]>(() => {
    const go = (id: string) => () => setActiveTab(id)
    const actions: CommandAction[] = [
      { id: 'dashboard', title: actionsCopy.dashboardTitle, description: actionsCopy.dashboardDescription, group: actionsCopy.groupNavigation, icon: <BarChart3 className="h-4 w-4" />, action: go('dashboard') },
      { id: 'instances', title: actionsCopy.instancesTitle, description: actionsCopy.instancesDescription, group: actionsCopy.groupRuntime, icon: <Server className="h-4 w-4" />, action: go('instances') },
      { id: 'models', title: actionsCopy.modelsTitle, description: actionsCopy.modelsDescription, group: actionsCopy.groupResources, icon: <Package className="h-4 w-4" />, action: go('model-repo') },
      { id: 'downloads', title: actionsCopy.downloadsTitle, description: actionsCopy.downloadsDescription, group: actionsCopy.groupResources, icon: <Download className="h-4 w-4" />, action: go('downloads') },
      { id: 'engines', title: actionsCopy.enginesTitle, description: actionsCopy.enginesDescription, group: actionsCopy.groupSetup, icon: <Wrench className="h-4 w-4" />, action: go('engine') },
      { id: 'performance', title: actionsCopy.performanceTitle, description: actionsCopy.performanceDescription, group: actionsCopy.groupDiagnostics, icon: <Activity className="h-4 w-4" />, action: go('perf') },
      { id: 'routing', title: actionsCopy.proxyTitle, description: actionsCopy.proxyDescription, group: actionsCopy.groupServices, icon: <Route className="h-4 w-4" />, action: go('proxy') },
      { id: 'logs', title: actionsCopy.logsTitle, description: actionsCopy.logsDescription, group: actionsCopy.groupDiagnostics, icon: <Terminal className="h-4 w-4" />, action: go('logs') },
      { id: 'refresh', title: actionsCopy.refreshTitle, description: actionsCopy.refreshDescription, group: actionsCopy.groupMaintenance, icon: <RefreshCw className="h-4 w-4" />, action: () => void loadInitialData() },
    ]
    const firstStopped = instances.find(instance => instance.status !== 'running')
    const firstRunning = instances.find(instance => instance.status === 'running')
    if (firstStopped) actions.push({ id: 'start-first', title: `${actionsCopy.startPrefix} ${firstStopped.name}`, description: actionsCopy.startDescription, group: actionsCopy.groupRuntime, icon: <Play className="h-4 w-4" />, action: () => void startInstance(firstStopped.id).catch(() => {}) })
    if (firstRunning) actions.push({ id: 'stop-first', title: `${actionsCopy.stopPrefix} ${firstRunning.name}`, description: actionsCopy.stopDescription, group: actionsCopy.groupRuntime, icon: <Square className="h-4 w-4" />, action: () => void stopInstance(firstRunning.id).catch(() => {}) })
    return actions
  }, [actionsCopy, instances, loadInitialData, setActiveTab, startInstance, stopInstance])

  return { productIssues, commandActions }
}
