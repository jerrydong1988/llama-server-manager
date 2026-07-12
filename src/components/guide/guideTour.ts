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
    id: 'dashboard',
    tab: 'dashboard',
    selector: '[data-guide="dashboard"]',
    zh: { title: '系统总览', description: '查看系统资源、实例状态与整体运行健康度。' },
    en: { title: 'Dashboard', description: 'Review system resources, instance state, and overall health.' },
  },
  {
    id: 'models',
    tab: 'model-repo',
    selector: '[data-guide="model-search"]',
    zh: { title: '模型仓库', description: '扫描和管理本地 GGUF 模型与投影器。' },
    en: { title: 'Model Repository', description: 'Scan and manage local GGUF models and projectors.' },
  },
  {
    id: 'downloads',
    tab: 'downloads',
    selector: '[data-guide="download-source"]',
    zh: { title: '下载管理', description: '浏览远程仓库并管理下载队列与恢复策略。' },
    en: { title: 'Downloads', description: 'Browse repositories and manage queues and resume policy.' },
  },
  {
    id: 'engines',
    tab: 'engine',
    selector: '[data-guide="engine-scan"]',
    zh: { title: '引擎管理', description: '扫描 llama-server 并选择默认运行引擎。' },
    en: { title: 'Engines', description: 'Scan llama-server binaries and choose the default runtime.' },
  },
  {
    id: 'instances',
    tab: 'instances',
    selector: '[data-guide="instance-create"]',
    zh: { title: '实例管理', description: '创建、启动和维护独立服务实例。' },
    en: { title: 'Instances', description: 'Create, start, and maintain server instances.' },
  },
  {
    id: 'config',
    tab: 'config',
    selector: '[data-guide="config-save"]',
    zh: { title: '参数配置', description: '按实例调整参数并查看分级校验提示。' },
    en: { title: 'Configuration', description: 'Tune instance parameters and review validation findings.' },
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
    zh: { title: '性能监控', description: '查看资源信号、吞吐、槽位与请求分析。' },
    en: { title: 'Performance', description: 'Inspect resources, throughput, slots, and request analysis.' },
  },
  {
    id: 'bigscreen',
    tab: 'bigscreen',
    selector: '[data-guide="monitoring-wall"]',
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

export function getGuideTourSteps(lang: string): LocalizedGuideTourStep[] {
  return GUIDE_TOUR_STEPS.map((step) => {
    const copy = lang === 'zh-CN' ? step.zh : step.en
    return {
      id: step.id,
      tab: step.tab,
      selector: step.selector,
      title: copy.title,
      description: copy.description,
    }
  })
}
