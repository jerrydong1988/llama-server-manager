import { selectLocalizedCopy } from '../../i18n'

export type GuideTourMode = 'setup' | 'advanced'
export type GuideTourStep = {
  id: string
  tab: string
  selector: string
  zh: { title: string; description: string }
  en: { title: string; description: string }
}

export type LocalizedGuideTourStep = {
  id: string
  tab: string
  selector: string
  title: string
  description: string
}

export const GUIDE_TOUR_STEPS: GuideTourStep[] = [
  {
    id: 'models', tab: 'model-repo', selector: '[data-guide="model-directories"]',
    zh: { title: '准备模型', description: '添加存放 GGUF 的目录并扫描。已有模型可以直接复用；尚未下载时，可从下方入口前往下载，再返回继续。' },
    en: { title: 'Prepare a model', description: 'Add the directory containing your GGUF files and scan it. Reuse existing models, or open downloads below and return when ready.' },
  },
  {
    id: 'engines', tab: 'engine', selector: '[data-guide="engine-scan"]',
    zh: { title: '确认运行引擎', description: '添加 llama-server 所在目录并扫描，确认所需后端已识别。已有引擎可直接使用。' },
    en: { title: 'Prepare an engine', description: 'Add the llama-server directory and scan it. Check that the backend you need is recognized, or reuse an existing engine.' },
  },
  {
    id: 'instances', tab: 'instances', selector: '[data-guide="instance-create"]',
    zh: { title: '创建或选择实例', description: '点击创建实例，选择模型、引擎和端口。也可以在下方选择已有实例；后续步骤会一直跟随该实例。' },
    en: { title: 'Create or choose an instance', description: 'Create an instance with a model, engine and port, or select an existing one below. The remaining steps stay with that instance.' },
  },
  {
    id: 'config', tab: 'config', selector: '[data-guide="config-save"]',
    zh: { title: '检查必要参数', description: '确认实例名称、模型、端口和硬件参数，按需调整。修改后请保存并处理校验提示，再继续启动。' },
    en: { title: 'Review configuration', description: 'Check the instance, model, port and hardware settings. Save any changes and resolve validation messages before continuing.' },
  },
  {
    id: 'start', tab: 'instances', selector: '[data-guide="instance-runtime"]',
    zh: { title: '启动实例', description: '确认页面中当前操作的实例，然后点击启动。引导会等待运行状态；已经运行的实例可以直接继续。' },
    en: { title: 'Start the instance', description: 'Confirm the selected instance, then click Start. Wait until it is running; an already running instance can continue immediately.' },
  },
  {
    id: 'verify', tab: 'instances', selector: '[data-guide="instance-connection"]',
    zh: { title: '验证连接', description: '点击测试连接并确认成功。之后可在浏览器中打开实例，或继续探索实例路由与监控。' },
    en: { title: 'Verify the connection', description: 'Click Test Connection and confirm success. Then open the instance in your browser, or explore routing and monitoring.' },
  },
]

const ADVANCED_TOUR_STEPS: GuideTourStep[] = [
  {
    id: 'dashboard',
    tab: 'dashboard',
    selector: '[data-guide="dashboard-overview"]',
    zh: { title: '系统总览', description: '查看系统资源、实例状态与整体运行健康度。' },
    en: { title: 'Dashboard', description: 'Review system resources, instance state, and overall health.' },
  },
  {
    id: 'downloads',
    tab: 'downloads',
    selector: '[data-guide="download-source"]',
    zh: { title: '下载管理', description: '浏览远程仓库并管理下载队列与恢复策略。' },
    en: { title: 'Downloads', description: 'Browse repositories and manage queues and resume policy.' },
  },
  {
    id: 'cluster',
    tab: 'cluster',
    selector: '[data-guide="cluster-scan"]',
    zh: { title: '集群管理', description: '发现或启动本地与远程 RPC Worker。' },
    en: { title: 'Cluster', description: 'Discover or launch local and remote RPC workers.' },
  },
  {
    id: 'proxy',
    tab: 'proxy',
    selector: '[data-guide="proxy-overview"]',
    zh: { title: '实例路由', description: '把多个实例统一到 OpenAI 兼容入口，并用代理密钥保护全部端点。' },
    en: { title: 'Instance Routing', description: 'Expose one OpenAI-compatible endpoint with key protection across all routes.' },
  },
  {
    id: 'performance',
    tab: 'perf',
    selector: '[data-guide="perf-select"]',
    zh: { title: '性能监控', description: '按生成或 Embedding / Reranker 工作负载查看资源、吞吐、任务日志与代理请求。' },
    en: { title: 'Performance', description: 'Inspect workload-aware resources and throughput from task logs and proxied requests.' },
  },
  {
    id: 'bigscreen',
    tab: 'bigscreen',
    selector: '[data-guide="monitoring-header"]',
    zh: { title: '监控大屏', description: '集中观察服务健康、吞吐、压力和告警。' },
    en: { title: 'Monitoring Wall', description: 'Watch service health, throughput, pressure, and alerts.' },
  },
  {
    id: 'logs',
    tab: 'logs',
    selector: '[data-guide="logs-clear"]',
    zh: { title: '服务器日志', description: '筛选实时日志并定位启动与健康检查问题。' },
    en: { title: 'Logs', description: 'Filter live logs and diagnose startup or health failures.' },
  },
]

export function getGuideTourSteps(lang: string, mode: GuideTourMode = 'setup'): LocalizedGuideTourStep[] {
  return (mode === 'setup' ? GUIDE_TOUR_STEPS : ADVANCED_TOUR_STEPS).map((step) => {
    const copy = selectLocalizedCopy(lang, step.zh, step.en)
    return {
      id: step.id,
      tab: step.tab,
      selector: step.selector,
      title: copy.title,
      description: copy.description,
    }
  })
}
