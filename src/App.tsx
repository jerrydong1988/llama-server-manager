import { useState, useEffect, useMemo, useRef, lazy, Suspense, Component, ReactNode } from 'react'
import { Activity, BarChart3, BookOpen, Cpu, Database, Download, Monitor, Network, Package, Play, RefreshCw, Route, Search, Server, Settings, Square, Terminal, Wrench, X } from 'lucide-react'
import { version } from '../package.json'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import ModelRepo from './components/ModelRepo'
import EngineManager from './components/EngineManager'
import InstanceManager from './components/InstanceManager'
const LogsViewer = lazy(() => import('./components/LogsViewer'))
import ConfigPage from './components/ConfigPage'
const PerformancePage = lazy(() => import('./components/PerformancePage/PerformancePage'))
const ClusterPage = lazy(() => import('./components/ClusterPage/ClusterPage'))
const DownloadManager = lazy(() => import('./components/DownloadManager'))
const ProxyPage = lazy(() => import('./components/ProxyPage'))
const BigScreenPage = lazy(() => import('./components/BigScreenPage'))
import Dashboard from './components/Dashboard/Dashboard'
const GuidePage = lazy(() => import('./components/GuidePage'))
import { _startupTimings } from './store'
import { useAppStore } from './store'
import { parseHostPort } from './utils/network'
import type { WorkerInfo } from './store'
import { I18nProvider, useI18n } from './i18n'
import { Badge, Button, TextInput } from './components/ui'
import { AppShell, type ShellStatusChip } from './components/shell/AppShell'

type ProductIssue = {
  id: string
  title: string
  description: string
  severity: 'info' | 'warning' | 'critical'
  actionLabel: string
  action: () => void
}

type CommandAction = {
  id: string
  title: string
  description: string
  group: string
  icon: ReactNode
  action: () => void
}

function CommandCenter({
  open,
  lang,
  issues,
  commands,
  onClose,
}: {
  open: boolean
  lang: string
  issues: ProductIssue[]
  commands: CommandAction[]
  onClose: () => void
}) {
  const zh = lang === 'zh-CN'
  const [query, setQuery] = useState('')

  useEffect(() => {
    if (!open) return
    setQuery('')
    const handler = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [open, onClose])

  const filteredCommands = useMemo(() => {
    const normalized = query.trim().toLowerCase()
    if (!normalized) return commands
    return commands.filter(command =>
      command.title.toLowerCase().includes(normalized)
      || command.description.toLowerCase().includes(normalized)
      || command.group.toLowerCase().includes(normalized),
    )
  }, [commands, query])

  if (!open) return null

  const severityTone: Record<ProductIssue['severity'], 'blue' | 'amber' | 'red'> = {
    info: 'blue',
    warning: 'amber',
    critical: 'red',
  }

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-slate-950/55 px-4 py-12 backdrop-blur-sm" role="dialog" aria-modal="true">
      <div className="w-full max-w-4xl overflow-hidden rounded-lg border border-slate-200 bg-white text-slate-900 shadow-2xl dark:border-slate-800 dark:bg-slate-950 dark:text-slate-100">
        <div className="flex items-start justify-between gap-4 border-b border-slate-200 px-5 py-4 dark:border-slate-800">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <Search className="h-5 w-5 text-blue-500" />
              <h2 className="text-lg font-semibold">{zh ? '任务中心' : 'Command Center'}</h2>
              {issues.length > 0 ? <Badge tone="amber">{issues.length} {zh ? '项需关注' : 'need attention'}</Badge> : <Badge tone="emerald">{zh ? '状态良好' : 'Healthy'}</Badge>}
            </div>
            <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">
              {zh ? '从这里快速处理配置缺口、运行异常和高频工作流。' : 'Handle setup gaps, runtime issues, and frequent workflows from one place.'}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label={zh ? '关闭任务中心' : 'Close command center'}
            className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-slate-200 text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 dark:border-slate-800 dark:hover:bg-slate-900 dark:hover:text-white"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="grid max-h-[74vh] overflow-y-auto lg:grid-cols-[minmax(0,1fr)_320px]">
          <div className="min-w-0 space-y-4 p-5">
            <TextInput
              value={query}
              onChange={event => setQuery(event.target.value)}
              placeholder={zh ? '搜索页面、任务或操作...' : 'Search pages, tasks, or actions...'}
              leadingIcon={<Search className="h-4 w-4" />}
              autoFocus
            />

            <section className="space-y-2">
              <div className="text-xs font-semibold uppercase tracking-wide text-slate-500 dark:text-slate-400">{zh ? '常用任务' : 'Common Tasks'}</div>
              <div className="grid gap-2 md:grid-cols-2">
                {filteredCommands.map(command => (
                  <button
                    key={command.id}
                    type="button"
                    onClick={() => {
                      command.action()
                      onClose()
                    }}
                    className="flex min-w-0 items-start gap-3 rounded-lg border border-slate-200 bg-slate-50 px-3 py-3 text-left transition hover:border-blue-300 hover:bg-blue-50 dark:border-slate-800 dark:bg-slate-900/70 dark:hover:border-blue-500/40 dark:hover:bg-blue-500/10"
                  >
                    <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-slate-200 bg-white text-blue-600 dark:border-slate-800 dark:bg-slate-950 dark:text-blue-300">
                      {command.icon}
                    </span>
                    <span className="min-w-0">
                      <span className="block truncate text-sm font-semibold text-slate-900 dark:text-slate-100">{command.title}</span>
                      <span className="mt-1 line-clamp-2 block text-xs leading-5 text-slate-500 dark:text-slate-400">{command.description}</span>
                    </span>
                  </button>
                ))}
              </div>
            </section>
          </div>

          <aside className="border-t border-slate-200 bg-slate-50 p-5 dark:border-slate-800 dark:bg-slate-900/55 lg:border-l lg:border-t-0">
            <div className="mb-3 flex items-center justify-between gap-3">
              <div className="text-sm font-semibold">{zh ? '问题中心' : 'Attention Center'}</div>
              <Badge tone={issues.length > 0 ? 'amber' : 'emerald'}>{issues.length}</Badge>
            </div>
            {issues.length === 0 ? (
              <div className="rounded-lg border border-slate-200 bg-white px-4 py-5 text-sm text-slate-500 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-400">
                {zh ? '当前没有需要立即处理的配置或运行问题。' : 'No setup or runtime issues need immediate action.'}
              </div>
            ) : (
              <div className="space-y-2">
                {issues.map(issue => (
                  <div key={issue.id} className="rounded-lg border border-slate-200 bg-white p-3 dark:border-slate-800 dark:bg-slate-950">
                    <div className="mb-2 flex items-center gap-2">
                      <Badge tone={severityTone[issue.severity]}>{issue.severity === 'critical' ? (zh ? '严重' : 'Critical') : issue.severity === 'warning' ? (zh ? '提醒' : 'Warning') : (zh ? '信息' : 'Info')}</Badge>
                      <div className="min-w-0 truncate text-sm font-semibold text-slate-900 dark:text-slate-100">{issue.title}</div>
                    </div>
                    <p className="text-xs leading-5 text-slate-500 dark:text-slate-400">{issue.description}</p>
                    <Button
                      onClick={() => {
                        issue.action()
                        onClose()
                      }}
                      size="sm"
                      className="mt-3 w-full"
                    >
                      {issue.actionLabel}
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </aside>
        </div>
      </div>
    </div>
  )
}

class ErrorBoundary extends Component<{ children: ReactNode }, { hasError: boolean; error: Error | null }> {
  constructor(props: { children: ReactNode }) {
    super(props)
    this.state = { hasError: false, error: null }
  }

  static getDerivedStateFromError(error: Error) {
    return { hasError: true, error }
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="flex h-screen items-center justify-center bg-slate-100 px-6 text-slate-900 dark:bg-slate-950 dark:text-slate-100">
          <div className="w-full max-w-md rounded-lg border border-slate-200 bg-white p-6 shadow-sm dark:border-slate-800 dark:bg-slate-900">
            <div className="mb-4 flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-rose-100 text-rose-600 dark:bg-rose-950/50 dark:text-rose-300">
                <Terminal className="h-5 w-5" />
              </div>
              <div>
                <h2 className="text-lg font-semibold">Something went wrong</h2>
                <p className="text-sm text-slate-500 dark:text-slate-400">The current view crashed and can be reloaded.</p>
              </div>
            </div>
            <div className="mb-5 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-sm text-slate-600 dark:border-slate-800 dark:bg-slate-950 dark:text-slate-300">
              {this.state.error?.message || 'Unknown error'}
            </div>
            <Button
              onClick={() => this.setState({ hasError: false, error: null })}
              variant="primary"
            >
              Reload view
            </Button>
          </div>
        </div>
      )
    }

    return this.props.children
  }
}

const TAB_CONTENT: Record<string, () => React.ReactElement> = {
  instances: () => <InstanceManager />,
  'model-repo': () => <ModelRepo />,
  dashboard: () => <Dashboard />,
  engine: () => <EngineManager />,
  config: () => <ConfigPage />,
  perf: () => <PerformancePage />,
  logs: () => <LogsViewer />,
  cluster: () => <ClusterPage />,
  proxy: () => <ProxyPage />,
  downloads: () => <DownloadManager />,
  bigscreen: () => <BigScreenPage />,
  guide: () => <GuidePage />,
}

const renderTabContent = (tabId: string) => {
  const Renderer = TAB_CONTENT[tabId]
  return Renderer ? <Renderer /> : <div className="p-6 text-center text-slate-500">...</div>
}

function PageFallback({ label }: { label: string }) {
  return (
    <div className="flex min-h-[280px] items-center justify-center">
      <div className="inline-flex items-center gap-3 rounded-lg border border-slate-200 bg-white px-4 py-3 text-sm text-slate-500 shadow-sm dark:border-slate-800 dark:bg-slate-900 dark:text-slate-300">
        <div className="h-4 w-4 animate-spin rounded-full border-2 border-blue-500 border-t-transparent" />
        <span>{label}</span>
      </div>
    </div>
  )
}

function AppInner() {
  const activeTab = useAppStore(s => s.activeTab)
  const setActiveTab = useAppStore(s => s.setActiveTab)
  const loadConfig = useAppStore(s => s.loadConfig)
  const loadInitialData = useAppStore(s => s.loadInitialData)
  const instances = useAppStore(s => s.instances)
  const models = useAppStore(s => s.models)
  const engines = useAppStore(s => s.engines)
  const startInstance = useAppStore(s => s.startInstance)
  const stopInstance = useAppStore(s => s.stopInstance)
  const darkMode = useAppStore(s => s.darkMode)
  const setDarkMode = useAppStore(s => s.setDarkMode)
  const runtimeWarnings = useAppStore(s => s.runtimeWarnings)
  const clearRuntimeWarnings = useAppStore(s => s.clearRuntimeWarnings)
  const { t, lang, setLang } = useI18n()
  const [updateInfo, setUpdateInfo] = useState<{ latest_version: string; url: string } | null>(null)
  const [autoStartEnabled, setAutoStartEnabled] = useState(false)
  const [autoStarted, setAutoStarted] = useState(false)
  const [commandCenterOpen, setCommandCenterOpen] = useState(false)
  const [proxyExitConfirmOpen, setProxyExitConfirmOpen] = useState(false)
  const [proxyExitKeepAliveEnabled, setProxyExitKeepAliveEnabled] = useState(false)

  useEffect(() => {
    invoke<boolean>('is_autostart_enabled').then(setAutoStartEnabled).catch(() => {})
  }, [])

  const upCount = useMemo(() => instances.filter(i => i.status === 'running').length, [instances])
  const downCount = useMemo(() => instances.filter(i => i.status !== 'running').length, [instances])
  const downloadTasks = useAppStore(s => s.downloadTasks)
  const activeDownloadCount = useMemo(
    () => Object.values(downloadTasks).filter(t => t.status === 'active' || t.status === 'paused' || t.status === 'pausing').length,
    [downloadTasks],
  )
  const failedDownloadCount = useMemo(
    () => Object.values(downloadTasks).filter(t => t.status === 'error').length,
    [downloadTasks],
  )

  const navigation = useMemo(() => [
    { id: 'dashboard', name: t.nav.dashboard || 'Dashboard', icon: BarChart3 },
    { id: 'model-repo', name: t.nav.modelRepo, icon: Database },
    { id: 'downloads', name: t.nav.downloads || 'Downloads', icon: Download, badge: activeDownloadCount },
    { id: 'engine', name: t.nav.engine, icon: Cpu },
    { id: 'instances', name: t.nav.instances, icon: Server, badge: upCount },
    { id: 'config', name: t.nav.config, icon: Settings, separator: true },
    { id: 'cluster', name: t.nav.cluster, icon: Network },
    { id: 'proxy', name: t.nav.proxy || (lang === 'zh-CN' ? '\u5b9e\u4f8b\u8def\u7531' : 'Routing'), icon: Route },
    { id: 'perf', name: t.nav.perf, icon: Activity },
    { id: 'bigscreen', name: t.nav.bigScreen || (lang === 'zh-CN' ? '\u5927\u5c4f\u6a21\u5f0f' : 'Big Screen'), icon: Monitor },
    { id: 'logs', name: t.nav.logs, icon: Terminal },
    { id: 'guide', name: lang === 'zh-CN' ? '\u4f7f\u7528\u8bf4\u660e' : 'Guide', icon: BookOpen, separator: true },
  ], [t, lang, upCount, activeDownloadCount])

  const productIssues = useMemo<ProductIssue[]>(() => {
    const zh = lang === 'zh-CN'
    const items: ProductIssue[] = []
    const unhealthyRunning = instances.filter(instance => instance.status === 'running' && instance.healthCheck === 'fail').length
    const erroredInstances = instances.filter(instance => instance.status === 'error').length
    const modelCount = models.filter(model => !model.is_shard && model.file_type === 'model').length

    if (engines.length === 0) {
      items.push({
        id: 'no-engines',
        title: zh ? '尚未登记运行引擎' : 'No runtime engines registered',
        description: zh ? '需要先扫描或添加 llama-server 二进制文件，实例才能启动。' : 'Scan or add llama-server binaries before starting instances.',
        severity: 'critical',
        actionLabel: zh ? '打开引擎管理' : 'Open engines',
        action: () => setActiveTab('engine'),
      })
    }

    if (modelCount === 0) {
      items.push({
        id: 'no-models',
        title: zh ? '模型仓库为空' : 'Model inventory is empty',
        description: zh ? '添加本地模型目录或从下载管理导入模型，实例配置会更顺畅。' : 'Add a local model folder or download models to make instance setup smoother.',
        severity: 'warning',
        actionLabel: zh ? '打开模型仓库' : 'Open models',
        action: () => setActiveTab('model-repo'),
      })
    }

    if (instances.length === 0) {
      items.push({
        id: 'no-instances',
        title: zh ? '还没有服务实例' : 'No server instances',
        description: zh ? '创建第一个实例后，端口、模型、引擎和运行状态会集中展示。' : 'Create the first instance to manage ports, models, engines, and runtime state.',
        severity: 'info',
        actionLabel: zh ? '创建实例' : 'Create instance',
        action: () => setActiveTab('instances'),
      })
    }

    if (erroredInstances > 0 || unhealthyRunning > 0) {
      items.push({
        id: 'instance-health',
        title: zh ? '存在异常实例' : 'Instance health needs attention',
        description: zh
          ? erroredInstances + ' 个实例错误，' + unhealthyRunning + ' 个运行实例健康检查失败。'
          : erroredInstances + ' errored instance(s), ' + unhealthyRunning + ' running health check failure(s).',
        severity: 'critical',
        actionLabel: zh ? '查看实例' : 'Review instances',
        action: () => setActiveTab('instances'),
      })
    }

    if (failedDownloadCount > 0) {
      items.push({
        id: 'failed-downloads',
        title: zh ? '存在失败下载任务' : 'Failed downloads detected',
        description: zh ? failedDownloadCount + ' 个下载任务需要重试、清理或重新排队。' : failedDownloadCount + ' download task(s) need retry, cleanup, or requeue.',
        severity: 'warning',
        actionLabel: zh ? '打开下载管理' : 'Open downloads',
        action: () => setActiveTab('downloads'),
      })
    }

    return items
  }, [engines.length, failedDownloadCount, instances, lang, models, setActiveTab])

  const commandActions = useMemo<CommandAction[]>(() => {
    const zh = lang === 'zh-CN'
    const firstStopped = instances.find(instance => instance.status !== 'running')
    const firstRunning = instances.find(instance => instance.status === 'running')
    const go = (id: string) => () => setActiveTab(id)

    const actions: CommandAction[] = [
      {
        id: 'dashboard',
        title: zh ? '打开系统驾驶舱' : 'Open system cockpit',
        description: zh ? '汇总资源、实例、下载与关键操作入口。' : 'Review resources, instances, downloads, and key actions.',
        group: zh ? '导航' : 'Navigation',
        icon: <BarChart3 className="h-4 w-4" />,
        action: go('dashboard'),
      },
      {
        id: 'instances',
        title: zh ? '管理服务实例' : 'Manage server instances',
        description: zh ? '启动、停止、打开端点、调整实例配置。' : 'Start, stop, open endpoints, and adjust instance configuration.',
        group: zh ? '运行' : 'Runtime',
        icon: <Server className="h-4 w-4" />,
        action: go('instances'),
      },
      {
        id: 'models',
        title: zh ? '整理模型仓库' : 'Organize model inventory',
        description: zh ? '扫描本地模型，查看分片与目录状态。' : 'Scan local models and review shards and folders.',
        group: zh ? '资源' : 'Resources',
        icon: <Package className="h-4 w-4" />,
        action: go('model-repo'),
      },
      {
        id: 'downloads',
        title: zh ? '处理下载队列' : 'Manage download queue',
        description: zh ? '继续、暂停、重试模型下载并调整传输策略。' : 'Resume, pause, retry model downloads, and adjust transfer policy.',
        group: zh ? '资源' : 'Resources',
        icon: <Download className="h-4 w-4" />,
        action: go('downloads'),
      },
      {
        id: 'engines',
        title: zh ? '检查运行引擎' : 'Check runtime engines',
        description: zh ? '扫描二进制、设置默认引擎并确认执行后端。' : 'Scan binaries, set defaults, and verify execution backend.',
        group: zh ? '配置' : 'Setup',
        icon: <Wrench className="h-4 w-4" />,
        action: go('engine'),
      },
      {
        id: 'performance',
        title: zh ? '打开性能分析' : 'Open performance analysis',
        description: zh ? '查看遥测、资源瓶颈、实例吞吐与智能诊断。' : 'Inspect telemetry, bottlenecks, instance throughput, and diagnostics.',
        group: zh ? '诊断' : 'Diagnostics',
        icon: <Activity className="h-4 w-4" />,
        action: go('perf'),
      },
      {
        id: 'routing',
        title: zh ? '管理实例路由' : 'Manage instance routing',
        description: zh ? '配置统一 OpenAI 兼容入口，将请求按模型名分发到运行中的实例。' : 'Configure the unified OpenAI-compatible endpoint and route requests by model name.',
        group: zh ? '服务' : 'Services',
        icon: <Route className="h-4 w-4" />,
        action: go('proxy'),
      },
      {
        id: 'logs',
        title: zh ? '查看运行日志' : 'Review runtime logs',
        description: zh ? '定位启动失败、下载错误和健康检查线索。' : 'Trace startup failures, download errors, and health check clues.',
        group: zh ? '诊断' : 'Diagnostics',
        icon: <Terminal className="h-4 w-4" />,
        action: go('logs'),
      },
      {
        id: 'refresh',
        title: zh ? '刷新全部状态' : 'Refresh all state',
        description: zh ? '重新加载实例、模型、引擎与队列状态。' : 'Reload instances, models, engines, and queue state.',
        group: zh ? '维护' : 'Maintenance',
        icon: <RefreshCw className="h-4 w-4" />,
        action: () => void loadInitialData(),
      },
    ]

    if (firstStopped) {
      actions.push({
        id: 'start-first',
        title: zh ? '启动 ' + firstStopped.name : 'Start ' + firstStopped.name,
        description: zh ? '快速启动第一个未运行实例。' : 'Quickly start the first stopped instance.',
        group: zh ? '运行' : 'Runtime',
        icon: <Play className="h-4 w-4" />,
        action: () => void startInstance(firstStopped.id).catch(() => {}),
      })
    }

    if (firstRunning) {
      actions.push({
        id: 'stop-first',
        title: zh ? '停止 ' + firstRunning.name : 'Stop ' + firstRunning.name,
        description: zh ? '快速停止当前第一个运行实例。' : 'Quickly stop the first running instance.',
        group: zh ? '运行' : 'Runtime',
        icon: <Square className="h-4 w-4" />,
        action: () => void stopInstance(firstRunning.id).catch(() => {}),
      })
    }

    return actions
  }, [instances, lang, loadInitialData, setActiveTab, startInstance, stopInstance])

  const mountTimeRef = useRef(performance.now())
  const [configLoaded, setConfigLoaded] = useState(false)
  const windowShownRef = useRef(false)

  useEffect(() => {
    _startupTimings.push({ name: 'app-mount', ms: Math.round(performance.now() - mountTimeRef.current) })
    if (!windowShownRef.current) {
      windowShownRef.current = true
      invoke('show_window').catch(() => {})
    }
    loadConfig().then(() => setConfigLoaded(true)).catch(() => setConfigLoaded(true))
  }, [loadConfig])

  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | undefined

    listen('proxy-exit-confirmation-requested', event => {
      const payload = event.payload && typeof event.payload === 'object'
        ? event.payload as Record<string, unknown>
        : {}
      setProxyExitKeepAliveEnabled(payload.backgroundServiceMode === true || payload.background_service_mode === true)
      setProxyExitConfirmOpen(true)
    }).then(cleanup => {
      if (disposed) cleanup()
      else unlisten = cleanup
    }).catch(() => {})

    return () => {
      disposed = true
      if (unlisten) unlisten()
    }
  }, [])

  useEffect(() => {
    if (autoStarted || instances.length === 0) return
    const toBoot = instances.filter(i => i.config.auto_start && i.status !== 'running')
    if (toBoot.length === 0) {
      setAutoStarted(true)
      return
    }
    if (engines.length === 0) return

    let cancelled = false
    const boot = async () => {
      const currentWorkers = useAppStore.getState().workers.length > 0
        ? useAppStore.getState().workers
        : await invoke<WorkerInfo[]>('get_workers').catch(() => [])

      for (const inst of toBoot) {
        if (cancelled) return
        if (inst.config.rpc_servers) {
          const configuredServers = inst.config.rpc_servers.split(/[, ]+/).filter(Boolean)
          const hasMatchingWorker = currentWorkers.some(w =>
            configuredServers.some(s => {
              const endpoint = parseHostPort(s, 50052)
              return w.host === endpoint.host && w.port === endpoint.port
            }),
          )
          if (!hasMatchingWorker) {
            console.warn(`Instance "${inst.name}" requires a cluster worker but no matching worker is available; skipping auto-start`)
            continue
          }
        }

        try {
          await startInstance(inst.id)
        } catch {
          // ignore failed auto-start
        }
        await new Promise(resolve => setTimeout(resolve, 3000))
      }
      setAutoStarted(true)
    }

    boot()
    return () => { cancelled = true }
  }, [instances, engines, autoStarted, startInstance])

  useEffect(() => {
    const controller = new AbortController()
    fetch('https://api.github.com/repos/jerrydong1988/llama-server-manager/releases/latest', {
      headers: { Accept: 'application/vnd.github+json' },
      signal: controller.signal,
    })
      .then(r => r.json())
      .then(json => {
        const latest = (json.tag_name || '').replace(/^v/, '')
        const current = version
        const l = latest.split('.').map(Number)
        const c = current.split('.').map(Number)
        const maxLen = Math.max(l.length, c.length)
        let has = false
        for (let i = 0; i < maxLen; i++) {
          const lv = l[i] || 0
          const cv = c[i] || 0
          if (lv > cv) {
            has = true
            break
          }
          if (lv < cv) {
            has = false
            break
          }
        }
        if (has) setUpdateInfo({ latest_version: latest, url: json.html_url || '' })
      })
      .catch(() => {})
    return () => controller.abort()
  }, [])

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!e.ctrlKey) return
      if (e.key === 'Enter') {
        e.preventDefault()
        const { instances, startInstance, stopInstance } = useAppStore.getState()
        if (instances.length > 0) {
          const running = instances.find(i => i.status === 'running')
          const stopped = instances.find(i => i.status !== 'running')
          if (running) void stopInstance(running.id).catch(() => {})
          else if (stopped) void startInstance(stopped.id).catch(() => {})
        }
      }
      if (e.key === 's' || e.key === 'S') {
        e.preventDefault()
        void useAppStore.getState().saveConfig().catch(() => {})
      }
      if (e.key === 'k' || e.key === 'K') {
        e.preventDefault()
        setCommandCenterOpen(true)
      }
    }

    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])

  const immersiveContent = activeTab === 'bigscreen'
  const layoutWide = activeTab === 'logs' || activeTab === 'guide' || immersiveContent || activeTab === 'proxy'
  const statusChips: ShellStatusChip[] = [
    { label: t.nav.up || 'up', value: upCount, tone: 'emerald' },
    { label: t.nav.down || 'down', value: downCount, tone: 'slate' },
    { label: t.nav.downloads || 'Downloads', value: activeDownloadCount, tone: 'blue' },
  ]
  const pageContext: Record<string, string> = {
    dashboard: lang === 'zh-CN' ? '\u7cfb\u7edf\u603b\u89c8\u4e0e llama-server \u5b9e\u4f8b\u6d3b\u52a8' : 'System overview and current llama-server activity',
    'model-repo': lang === 'zh-CN' ? '\u6d4f\u89c8\u672c\u5730\u4e0e\u8fdc\u7a0b\u6a21\u578b\u8d44\u6e90' : 'Browse local and remote model assets',
    downloads: lang === 'zh-CN' ? '\u6a21\u578b\u4f20\u8f93\u961f\u5217\u4e0e\u6d3b\u52a8\u4efb\u52a1' : 'Model transfer queue and active tasks',
    engine: lang === 'zh-CN' ? '\u8fd0\u884c\u65f6\u4e8c\u8fdb\u5236\u4e0e\u6267\u884c\u540e\u7aef' : 'Runtime binaries and execution backends',
    instances: lang === 'zh-CN' ? '\u670d\u52a1\u5b9e\u4f8b\u3001\u7aef\u53e3\u4e0e\u8fdb\u7a0b\u72b6\u6001' : 'Server instances, ports, and process state',
    config: lang === 'zh-CN' ? '\u5e94\u7528\u9ed8\u8ba4\u503c\u4e0e\u8fd0\u884c\u65f6\u504f\u597d' : 'Application defaults and runtime preferences',
    cluster: lang === 'zh-CN' ? '\u5206\u5e03\u5f0f worker \u4e0e\u7f51\u7edc\u53ef\u7528\u6027' : 'Distributed workers and network availability',
    proxy: lang === 'zh-CN' ? '\u7edf\u4e00 API \u5165\u53e3\u4e0e\u6a21\u578b\u522b\u540d\u8def\u7531' : 'Unified API endpoint and model alias routing',
    perf: lang === 'zh-CN' ? '\u6027\u80fd\u9065\u6d4b\u4e0e\u8d44\u6e90\u4fe1\u53f7' : 'Performance telemetry and resource signals',
    logs: lang === 'zh-CN' ? '\u8fd0\u884c\u65e5\u5fd7\u4e0e\u8bca\u65ad\u8f93\u51fa' : 'Runtime logs and diagnostic output',
    guide: lang === 'zh-CN' ? '\u53c2\u8003\u8d44\u6599\u4e0e\u64cd\u4f5c\u8bf4\u660e' : 'Reference material and operating notes',
  }
  const handleAutoStartChange = (next: boolean) => {
    setAutoStartEnabled(next)
    ;(async () => {
      try {
        if (next) await invoke('enable_autostart')
        else await invoke('disable_autostart')
      } catch {
        setAutoStartEnabled(!next)
      }
    })()
  }
  const handleStopProxyAndQuit = async () => {
    try {
      await invoke('stop_proxy')
      await invoke('quit_app')
    } catch {
      setProxyExitConfirmOpen(false)
      setProxyExitKeepAliveEnabled(false)
      setActiveTab('proxy')
    }
  }
  const handleKeepProxyInTray = () => {
    setProxyExitConfirmOpen(false)
    setProxyExitKeepAliveEnabled(false)
    invoke('hide_window').catch(() => {})
  }

  return (
    <AppShell
      appTitle={t.common.appTitle}
      version={version}
      navigation={navigation}
      activeId={activeTab}
      onNavigate={setActiveTab}
      pageDescription={pageContext[activeTab] || (lang === 'zh-CN' ? '\u7ba1\u7406\u672c\u5730 llama-server \u8fd0\u884c\u72b6\u6001' : 'Manage local llama-server runtime state')}
      statusChips={statusChips}
      updateInfo={updateInfo}
      autoStartLabel={t.common.autoStart || 'Auto Start'}
      autoStartEnabled={autoStartEnabled}
      onAutoStartChange={handleAutoStartChange}
      darkMode={darkMode}
      onToggleDarkMode={() => setDarkMode(!darkMode)}
      darkModeTitle={darkMode ? (lang === 'zh-CN' ? '\u5207\u6362\u5230\u660e\u4eae\u6a21\u5f0f' : 'Switch to light mode') : (lang === 'zh-CN' ? '\u5207\u6362\u5230\u6df1\u8272\u6a21\u5f0f' : 'Switch to dark mode')}
      languageLabel={lang === 'zh-CN' ? 'EN' : '\u4E2D\u6587'}
      languageTitle={lang === 'zh-CN' ? 'Switch to English' : '\u5207\u6362\u4E3A\u4E2D\u6587'}
      onToggleLanguage={() => setLang(lang === 'zh-CN' ? 'en-US' : 'zh-CN')}
      commandLabel={lang === 'zh-CN' ? '\u4efb\u52a1\u4e2d\u5fc3' : 'Command Center'}
      attentionLabel={lang === 'zh-CN' ? '\u5173\u6ce8\u4e2d\u5fc3' : 'Attention Center'}
      attentionCount={productIssues.length}
      onOpenCommandCenter={() => setCommandCenterOpen(true)}
      wideContent={layoutWide}
      immersiveContent={immersiveContent}
      constrainContent={activeTab === 'guide'}
    >
      {!immersiveContent && runtimeWarnings.length > 0 && (
        <div className="mx-auto mb-4 flex w-full max-w-7xl items-start justify-between gap-4 rounded-lg border border-amber-300 bg-amber-50 px-4 py-3 text-sm text-amber-900 shadow-sm dark:border-amber-500/40 dark:bg-amber-500/10 dark:text-amber-100">
          <div className="min-w-0">
            <div className="font-semibold">{lang === 'zh-CN' ? '\u8fd0\u884c\u8b66\u544a' : 'Runtime warnings'}</div>
            <div className="mt-1 truncate text-xs opacity-90" title={runtimeWarnings[0]}>{runtimeWarnings[0]}</div>
          </div>
          <button
            type="button"
            onClick={clearRuntimeWarnings}
            className="shrink-0 rounded-md border border-amber-300 px-2 py-1 text-xs font-medium transition hover:bg-amber-100 dark:border-amber-400/40 dark:hover:bg-amber-500/20"
          >
            {lang === 'zh-CN' ? '\u6e05\u9664' : 'Clear'}
          </button>
        </div>
      )}
      {!configLoaded ? (
        <div className="flex min-h-[calc(100vh-160px)] items-center justify-center px-6">
          <PageFallback label={t.common?.loading || 'Loading...'} />
        </div>
      ) : (
        <Suspense fallback={<PageFallback label={t.common?.loading || 'Loading...'} />}>
          {renderTabContent(activeTab)}
        </Suspense>
      )}
      <CommandCenter
        open={commandCenterOpen}
        lang={lang}
        issues={productIssues}
        commands={commandActions}
        onClose={() => setCommandCenterOpen(false)}
      />
      {proxyExitConfirmOpen ? (
        <div className="fixed inset-0 z-[120] flex items-center justify-center bg-slate-950/60 px-4 backdrop-blur-sm">
          <div className="w-full max-w-lg rounded-xl border border-slate-200 bg-white p-6 shadow-2xl dark:border-slate-800 dark:bg-slate-950">
            <div className="flex items-start justify-between gap-4">
              <div>
                <h2 className="text-xl font-semibold text-slate-950 dark:text-slate-50">
                  {lang === 'zh-CN' ? '\u5b9e\u4f8b\u8def\u7531\u6b63\u5728\u8fd0\u884c' : 'Instance routing is running'}
                </h2>
                <p className="mt-3 text-sm leading-6 text-slate-600 dark:text-slate-300">
                  {proxyExitKeepAliveEnabled
                    ? (lang === 'zh-CN'
                        ? '\u540e\u53f0\u4fdd\u6d3b\u6a21\u5f0f\u5df2\u5f00\u542f\u3002\u9009\u62e9\u4fdd\u6301\u6258\u76d8\u8fd0\u884c\u4f1a\u7ee7\u7eed\u63d0\u4f9b\u8def\u7531\uff1b\u5982\u679c\u8981\u771f\u6b63\u9000\u51fa\u4e3b\u7a0b\u5e8f\uff0c\u8bf7\u9009\u62e9\u505c\u6b62\u8def\u7531\u5e76\u9000\u51fa\u3002'
                        : 'Background keep-alive is enabled. Keeping the app in the tray will continue serving routes; to truly quit the main process, stop routing and exit.')
                    : (lang === 'zh-CN'
                        ? '\u76f4\u63a5\u9000\u51fa\u4e3b\u7a0b\u5e8f\u4f1a\u4e2d\u65ad\u7edf\u4e00 API \u5165\u53e3\u3002\u5efa\u8bae\u4fdd\u6301\u6258\u76d8\u8fd0\u884c\uff0c\u6216\u8005\u5148\u505c\u6b62\u8def\u7531\u518d\u9000\u51fa\u3002'
                        : 'Exiting the main process will interrupt the unified API endpoint. Keep it running in the tray, or stop routing before exiting.')}
                </p>
              </div>
              <button
                type="button"
                onClick={() => {
                  setProxyExitConfirmOpen(false)
                  setProxyExitKeepAliveEnabled(false)
                }}
                className="rounded-lg border border-slate-200 p-2 text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 dark:border-slate-800 dark:text-slate-400 dark:hover:bg-slate-900 dark:hover:text-slate-100"
                aria-label={lang === 'zh-CN' ? '\u5173\u95ed' : 'Close'}
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="mt-6 flex flex-col-reverse gap-3 sm:flex-row sm:justify-end">
              <Button onClick={handleKeepProxyInTray}>
                {proxyExitKeepAliveEnabled
                  ? (lang === 'zh-CN' ? '\u4fdd\u6301\u540e\u53f0\u8def\u7531\u8fd0\u884c' : 'Keep route running')
                  : (lang === 'zh-CN' ? '\u4fdd\u6301\u6258\u76d8\u8fd0\u884c' : 'Keep running in tray')}
              </Button>
              <Button
                onClick={() => {
                  setActiveTab('proxy')
                  setProxyExitConfirmOpen(false)
                  setProxyExitKeepAliveEnabled(false)
                }}
              >
                {lang === 'zh-CN' ? '\u6253\u5f00\u8def\u7531\u8bbe\u7f6e' : 'Open routing settings'}
              </Button>
              <Button variant="danger" onClick={() => void handleStopProxyAndQuit()}>
                {lang === 'zh-CN' ? '\u505c\u6b62\u8def\u7531\u5e76\u9000\u51fa' : 'Stop routing and exit'}
              </Button>
            </div>
          </div>
        </div>
      ) : null}
    </AppShell>
  )
}

function App() {
  return <I18nProvider><ErrorBoundary><AppInner /></ErrorBoundary></I18nProvider>
}

export default App
