import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { message } from '@tauri-apps/plugin-dialog'
import { pathBasename } from './utils/path'

// Re-exports from split modules
export type { ModelInfo, EngineInfo, InstanceConfig, Instance, LogEntry, MsFileEntry, DownloadProgress, AppState, WorkerInfo, WorkerDevice, WorkerStatus, Usb4Adapter } from './store/types'
export { defaultInstanceConfig } from './store/defaults'
import type { AppState, ModelInfo, EngineInfo, InstanceConfig, Instance, MsFileEntry } from './store/types'

// ── Store 状态 ─────────────────────────────────────────────────
// AppState interface is defined in ./store/types.ts
const MAX_LOG_ENTRIES = 1000

export const useAppStore = create<AppState>((set, get) => ({
  models: [],
  engines: [],
  instances: [],
  logs: {},
  isLoading: false,
  darkMode: true,
  defaultEngineId: null,
  modelDirs: [],
  engineDirs: [],
  activeConfigInstanceId: null,
  activeTab: 'model-repo',
  workers: [],
  clusterScanning: false,
  downloadProgress: {},
  downloadTasks: {},
  downloadQueue: [],
  processingQueue: false,
  setDownloadTasks: (tasks) => set({ downloadTasks: tasks }),
  addToDownloadQueue: (entry) => {
    const s = get()
    const id = crypto.randomUUID()
    const fileNames = new Set(entry.files.map(f => f.name))
    
    // 去重：已在队列或正在下载/已完成的不重复入队
    const alreadyInQueue = s.downloadQueue.some(q => 
      q.repoId === entry.repoId && q.source === entry.source && 
      q.files.some(f => fileNames.has(f.name)))
    if (alreadyInQueue) return

    // 更新已存在条目的状态为 queued
    const updated = { ...s.downloadTasks }
    for (const f of entry.files) {
      if (updated[f.name]) {
        updated[f.name] = { ...updated[f.name], status: 'queued' as const }
      }
    }
    if (Object.keys(updated).length > 0) set({ downloadTasks: updated })

    set({ downloadQueue: [...s.downloadQueue, { ...entry, id, addedAt: Date.now() }] })
    // 尝试立即处理队列
    get().processDownloadQueue!()
  },
  removeFromDownloadQueue: (id) => {
    set(s => ({ downloadQueue: s.downloadQueue.filter(e => e.id !== id) }))
  },
  processDownloadQueue: async () => {
    const s = get()
    if (s.processingQueue) return
    set({ processingQueue: true })
    
    try {
      const { downloadQueue, downloadTasks } = get()
      const activeCount = Object.values(downloadTasks).filter(t => t.status === 'active').length
      const MAX = 3
      
      if (activeCount >= MAX || downloadQueue.length === 0) return

      const next = downloadQueue[0]
      set(s => ({ downloadQueue: s.downloadQueue.filter(e => e.id !== next.id) }))
      
      // 标记为 active
      const tasks = { ...get().downloadTasks }
      for (const f of next.files) {
        tasks[f.name] = { fileName: f.name, repoId: next.repoId, source: next.source, downloaded: 0, total: f.size, speed: 0, status: 'active' }
      }
      set({ downloadTasks: tasks })

      try {
        if (next.source === 'modelscope') {
          await invoke('download_modelscope_files', { repoId: next.repoId, files: next.files, saveDir: next.saveDir })
        } else {
          await invoke('download_huggingface_files', { repoId: next.repoId, files: next.files, saveDir: next.saveDir })
        }
      } finally {
        set({ processingQueue: false })
        await get().processDownloadQueue!()
      }
    } catch {
      set({ processingQueue: false })
    }
  },
  setModels: (models) => set({ models }),
  setEngines: (engines) => set({ engines }),
  setModelDirs: (dirs) => { set({ modelDirs: dirs }); get().saveConfig() },
  setEngineDirs: (dirs) => { set({ engineDirs: dirs }); get().saveConfig() },
  setDefaultEngineId: (id) => { set({ defaultEngineId: id }); get().saveConfig() },
  setActiveConfigInstanceId: (id) => set({ activeConfigInstanceId: id }),
  setActiveTab: (tab) => { set({ activeTab: tab }); get().saveConfig() },
  setDarkMode: (dm) => { set({ darkMode: dm }); document.documentElement.classList.toggle('dark', dm); get().saveConfig() },

  addInstance: (instance) => { set((s) => ({ instances: [...s.instances, instance] })); get().saveConfig() },
  updateInstance: (id, partial) => set((s) => ({
    instances: s.instances.map((i) => (i.id === id ? { ...i, ...partial } : i)),
  })),
  deleteInstance: (id) => set((s) => ({
    instances: s.instances.filter((i) => i.id !== id),
  })),

  moveInstance: (id, direction) => set((s) => {
    const idx = s.instances.findIndex(i => i.id === id)
    if (idx < 0) return s
    const target = direction === 'up' ? idx - 1 : idx + 1
    if (target < 0 || target >= s.instances.length) return s
    const arr = [...s.instances];
    [arr[idx], arr[target]] = [arr[target], arr[idx]]
    get().saveConfig()
    return { instances: arr }
  }),

  renameInstance: (id, name) => set((s) => {
    const inst = s.instances.find(i => i.id === id)
    if (!inst) return s
    const newConfig = { ...inst.config, name }
    const updated = s.instances.map(i => i.id === id ? { ...i, name, config: newConfig } : i)
    get().saveConfig()
    return { instances: updated }
  }),

  addLog: (entry) => set((s) => {
    const existing = s.logs[entry.instanceId] || []
    const updated = [...existing.slice(-(MAX_LOG_ENTRIES - 1)), { ...entry, timestamp: Date.now() }]
    return { logs: { ...s.logs, [entry.instanceId]: updated } }
  }),
  clearLogs: (instanceId) => set((s) => ({
    logs: { ...s.logs, [instanceId]: [] },
  })),

  // ── 初始化 ──────────────────────────────────────────────────
  loadInitialData: async () => {
    set({ isLoading: true })
    try {
      const [models, engines] = await Promise.all([
        invoke<ModelInfo[]>('get_models').catch(() => [] as ModelInfo[]),
        invoke<EngineInfo[]>('get_engines').catch(() => [] as EngineInfo[]),
      ])
      set({ models, engines, isLoading: false })
    } catch { set({ isLoading: false }) }
  },

  // ── 模型仓库 ──────────────────────────────────────────────────
  scanModels: async (paths: string[]) => {
    set({ isLoading: true })
    try {
      const models = await invoke<ModelInfo[]>('scan_models', { paths })
      set({ models, modelDirs: paths, isLoading: false })
      return null
    } catch (e: any) {
      console.error('scan_models error:', e)
      set({ isLoading: false })
      return e?.message || e?.toString() || '扫描失败'
    }
  },

  deleteModelFile: async (path: string) => {
    await invoke('delete_model_file', { path })
    set((s) => ({ models: s.models.filter((m) => m.path !== path) }))
  },

  openModelFolder: async (path: string) => {
    await invoke('open_model_folder', { path })
  },

  readGgufMetadata: async (path: string) => {
    return await invoke<[string | null, number | null, string | null]>('read_gguf_metadata', { path })
  },

  // ── 引擎管理 ──────────────────────────────────────────────────
  scanEngines: async (paths: string[]) => {
    set({ isLoading: true })
    try {
      const engines = await invoke<EngineInfo[]>('scan_engines', { paths })
      set({ engines, engineDirs: paths, isLoading: false })
    } catch (e) { console.error('scan_engines error:', e); set({ isLoading: false }) }
  },

  deleteEngine: async (id: string) => {
    await invoke('delete_engine', { id })
    set((s) => ({ engines: s.engines.filter((e) => e.id !== id) }))
  },

  renameEngine: (id, name) => {
    invoke('rename_engine', { id, name })
    set((s) => ({
      engines: s.engines.map(e => e.id === id ? { ...e, name, custom_name: name } : e)
    }))
  },

  openEngineFolder: async (dir: string) => {
    await invoke('open_engine_folder', { dir })
  },

  // ── 服务器控制 ────────────────────────────────────────────────
  generateCommand: async (config: InstanceConfig, engineExe: string) => {
    return await invoke<string[]>('generate_server_command', { config, engineExe })
  },

  startInstance: async (id: string) => {
    try {
      const { instances, engines, defaultEngineId } = get()
      const inst = instances.find(i => i.id === id)
      if (!inst) { message('Instance not found.', { title: 'Error', kind: 'error' }); return }
      // 实例指定引擎 > 默认引擎 > 第一个引擎
      const engine = engines.find(e => e.id === inst.config.engine_id)
        || engines.find(e => e.id === defaultEngineId)
        || engines[0]
      if (!engine) { message('No llama-server engine available.\n\nPlease scan engines first.', { title: 'Error', kind: 'error' }); return }

      await invoke('start_server', { instanceId: id, config: inst.config, engineExe: engine.exe, engineBackend: engine.backend })
      get().updateInstance(id, { status: 'running', healthCheck: 'pending' })
    } catch (e) {
      console.error('start_server error:', e)
    }
  },

  stopInstance: async (id: string) => {
    try {
      await invoke('stop_server', { instanceId: id })
      get().updateInstance(id, { status: 'stopped', healthCheck: 'pending' })
    } catch (e) {
      console.error('stop_server error:', e)
    }
  },

  openBrowser: async (host: string, port: number) => {
    await invoke('open_browser', { host, port })
  },

  // ── 配置持久化 ────────────────────────────────────────────────
  saveConfig: async () => {
    const { instances, modelDirs, engineDirs, defaultEngineId, activeTab, darkMode } = get()
    const map: Record<string, InstanceConfig> = {}
    const order: string[] = []
    instances.forEach((i) => { map[i.id] = i.config; order.push(i.id) })
    await invoke('save_config', { instances: map, modelDirs, engineDirs, defaultEngineId: defaultEngineId || '', instanceOrder: order, lastTab: activeTab, darkMode })
  },

  loadConfig: async () => {
    try {
      const global = await invoke<{
        instances: Record<string, InstanceConfig>
        model_dirs: string[]
        engine_dirs: string[]
        default_engine_id: string
        running: Record<string, { instance_id: string; pid: number; port: number; host: string; start_time?: number }>
        instance_order: string[]
        last_tab: string
        dark_mode: boolean
      }>('load_config')

      const runningIds = new Set(Object.keys(global.running || {}))
      const order = global.instance_order || Object.keys(global.instances)
      let list: Instance[] = Object.entries(global.instances).map(([id, config]) => {
        const st = global.running?.[id]?.start_time ?? 0
        return {
        id, name: config.name || '未命名实例', status: runningIds.has(id) ? 'running' as const : 'stopped' as const,
        model: pathBasename(config.model_path),
        port: config.port, healthCheck: runningIds.has(id) ? 'pending' as const : 'pending' as const, config,
        startTime: st > 0 ? st * 1000 : undefined,
      }})
      // 按保存的顺序排列
      list.sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id))
        set({
          instances: list,
          modelDirs: global.model_dirs || [],
          engineDirs: global.engine_dirs || [],
          defaultEngineId: global.default_engine_id || null,
          activeTab: global.last_tab || 'model-repo',
        })
        if (global.dark_mode !== undefined) {
          document.documentElement.classList.toggle('dark', global.dark_mode)
          set({ darkMode: !!global.dark_mode })
        }
      // #4: succeed-first — 扫描失败或返回空时不覆盖已有数据
      invoke('scan_models', { paths: global.model_dirs || [] }).then((models) => {
        const arr = models as any[]
        if (arr && arr.length > 0) set({ models: arr })
      }).catch(() => {})
      invoke('scan_engines', { paths: global.engine_dirs || [] }).then((engines) => {
        const arr = engines as any[]
        if (arr && arr.length > 0) set({ engines: arr })
      }).catch(() => {})
    } catch (e) { console.error('load_config error:', e) }
  },

  // ── ModelScope ──────────────────────────────────────────────────
  browseModelscope: async (repoId: string) => {
    return await invoke<MsFileEntry[]>('browse_modelscope', { repoId })
  },

  downloadModelscopeFiles: async (repoId: string, files: MsFileEntry[], saveDir: string) => {
    await invoke('download_modelscope_files', { repoId, files, saveDir })
  },

  // ── HuggingFace ──────────────────────────────────────────────────
  browseHuggingface: async (repoId: string) => {
    return await invoke<MsFileEntry[]>('browse_huggingface', { repoId })
  },

  downloadHuggingfaceFiles: async (repoId: string, files: MsFileEntry[], saveDir: string) => {
    await invoke('download_huggingface_files', { repoId, files, saveDir })
  },

  cancelFileDownload: async (fileName: string) => {
    try { await invoke('cancel_file_download', { fileName }) } catch (e) { console.error(e) }
  },

  pauseFileDownload: async (fileName: string) => {
    try { await invoke('pause_file_download', { fileName }) } catch (e) { console.error(e) }
  },

  cancelAndCleanupDownload: async (fileName: string, filePath: string) => {
    try { await invoke('cancel_and_cleanup_download', { fileName, filePath }) } catch (e) { console.error(e) }
  },

  // ── Cluster ──
  setWorkers: (workers) => set({ workers }),
  addWorker: (worker) => set(s => ({ workers: [...s.workers, worker] })),
  removeWorker: (id) => set(s => ({ workers: s.workers.filter(w => w.id !== id) })),
  updateWorker: (id, partial) => set(s => ({
    workers: s.workers.map(w => w.id === id ? { ...w, ...partial } : w)
  })),
  setClusterScanning: (scanning) => set({ clusterScanning: scanning }),
}))

// ── 事件监听 ─────────────────────────────────────────────────

// 格式化启动命令为分组可读格式
export function formatStartupCommand(cmdStr: string): string {
  const tokens = cmdStr.match(/(?:[^\s"]+|"[^"]*")+/g) || []
  const exeName = (tokens[0] || '').split(/[\\/]/).pop() || tokens[0] || ''

  // 分组规则: 按 flag 前缀归类
  const groups: Record<string, string[]> = {
    '\u6A21\u578B': [],  // 模型
    '\u63A8\u7406': [],  // 推理
    '\u6027\u80FD': [],  // 性能
    '\u7F13\u5B58': [],  // 缓存
    '\u5185\u5B58': [],  // 内存
    '\u91C7\u6837': [],  // 采样
    '\u63A8\u6D4B': [],  // 推测
    '\u89C6\u89C9': [],  // 视觉
    '\u7F51\u7EDC': [],  // 网络
    '\u5176\u4ED6': [],  // 其他
  }

  const classify = (flag: string): string => {
    if (/^-m$|^-a$|^--alias|^--mmproj|^--lora|^--chat-template|^--chat-template-file|^--grammar\b|^--skip-chat|^--jinja|^--models-dir|^--models-preset|^--models-max|^--models-autoload|^--tools/.test(flag)) return '\u6A21\u578B'
    if (/^--reasoning|^--reasoning-budget/.test(flag)) return '\u63A8\u7406'
    if (/^-ngl|^-t$|^-tb$|^-b$|^-ub$|^-np|^-cb|^--threads|^--batch/.test(flag)) return '\u6027\u80FD'
    if (/^-c$|^--ctx|^--keep|^-cram|^--cache-ram|^--cache-reuse|^--cache-idle|^--kv-unified|^--warmup|^--no-cache|^--override-kv|^--rope-scaling|^--rope-scale|^--rope-freq-base|^--rope-freq-scale|^--yarn-ext-factor|^--yarn-attn-factor|^--yarn-beta|^--no-context-shift|^--swa/.test(flag)) return '\u7F13\u5B58'
    if (/^-fa|^--mlock|^--no-mmap|^--numa|^--check-tensors|^--fit/.test(flag)) return '\u5185\u5B58'
    if (/^-n$|^--temp$|^--top-k|^--top-p|^--top-n-sigma|^--min-p|^--repeat|^-s$|^--seed|^--presence|^--frequency|^--ignore-eos|^--json-schema|^--mirostat|^--xtc|^--dynatemp|^--typical|^--dry|^--adaptive|^--logit-bias|^--samplers\b|^--sampler-seq|^-bs|^--backend-sampling|^-sp$|^--special|^--reverse-prompt|^--spm-infill/.test(flag)) return '\u91C7\u6837'
    if (/^--spec|^-md$|^-ngld|-lcs|-lcd|^--lookup|^--draft/.test(flag)) return '\u63A8\u6D4B'
    if (/^--image|^--mmproj-url|^--mmproj-auto|^--embedding|^--pooling|^--reranking|^--embd-normalize|^--tags|^--media/.test(flag)) return '\u89C6\u89C9'
    if (/^--host|^--port|^--api-key|^--ssl|^--path|^--api-prefix|^--no-ui|^--threads-http|^--metrics|^--props|^--slot|^--ui-config|^--sleep-idle|^--verbose/.test(flag)) return '\u7F51\u7EDC'
    return '\u5176\u4ED6'
  }

  for (let i = 1; i < tokens.length; i++) {
    const t = tokens[i].replace(/^"|"$/g, '')
    if (t.startsWith('-')) {
      const cat = classify(t)
      const nextIsValue = i + 1 < tokens.length && (
        !tokens[i + 1].startsWith('-') || /^-\d+(\.\d+)?$/.test(tokens[i + 1])
      )
      if (nextIsValue) {
        const val = tokens[i + 1].replace(/^"|"$/g, '')
        // 路径缩短: 只显示文件名
        const shortVal = (t === '-m' || t === '-md' || t === '--mmproj' || t === '--lora')
          ? val.split(/[\\/]/).pop() || val : val
        groups[cat].push(`${t} ${shortVal}`)
        i++
      } else {
        groups[cat].push(t)
      }
    }
  }

  const lines: string[] = []
  lines.push('\u250C\u2500\u2500 \u542F\u52A8\u547D\u4EE4 (EXE: ' + exeName + ')')  // ┌── 启动命令 (EXE: ...)

  for (const [label, args] of Object.entries(groups)) {
    if (args.length > 0) {
      lines.push(`\u2502 [${label}]  ${args.join('  ')}`)
    }
  }

  // 底部短路径: 完整命令(一行, 路径截断为文件名)
  const shortCmd = tokens.map((t, i) => {
    const s = t.replace(/^"|"$/g, '')
    if (i > 0 && (s === '-m' || s === '-md' || s === '--mmproj' || s === '--lora' || s === '--lora-init-without-apply')) {
      const next = tokens[i + 1]?.replace(/^"|"$/g, '')
      return s + ' "' + ((next || '').split(/[\\/]/).pop() || next) + '"'
    }
    return t
  }).slice(1).join(' ')
  lines.push('\u2502')
  lines.push(`\u2502 \u5B8C\u6574: ${shortCmd}`)

  return lines.join('\n')
}

listen<{ instanceId: string; text: string }>('server-log', (event) => {
  useAppStore.getState().addLog({
    instanceId: event.payload.instanceId,
    text: event.payload.text,
    timestamp: Date.now(),
  })
}).catch(() => {})

listen<{ instanceId: string; pid: number; port: number; command: string }>('server-started', (event) => {
  const state = useAppStore.getState()
  state.updateInstance(event.payload.instanceId, {
    status: 'running',
    healthCheck: 'pending',
    startTime: Date.now(),
  })
  state.addLog({
    instanceId: event.payload.instanceId,
    text: formatStartupCommand(event.payload.command) + '\n\u2514\u2500\u2500 PID: ' + event.payload.pid + ' | \u7AEF\u53E3: ' + event.payload.port,
    timestamp: Date.now(),
  })
  state.saveConfig()
}).catch(() => {})

listen<{ instanceId: string }>('server-stopped', (event) => {
  const state = useAppStore.getState()
  const inst = state.instances.find(i => i.id === event.payload.instanceId)
  if (inst) {
    const isError = inst.status === 'running' && inst.healthCheck !== 'ok'
    state.updateInstance(event.payload.instanceId, {
      status: isError ? 'error' : 'stopped',
      healthCheck: isError ? 'fail' : 'pending',
    })
  }
  state.saveConfig()
}).catch(() => {})

listen<{ instanceId: string; error: string }>('server-error', (event) => {
  const state = useAppStore.getState()
  state.updateInstance(event.payload.instanceId, {
    status: 'error',
    healthCheck: 'fail',
  })
  state.saveConfig()
}).catch(() => {})

listen<{ instanceId: string; status: string }>('health-status', (event) => {
  useAppStore.getState().updateInstance(event.payload.instanceId, {
    healthCheck: event.payload.status === 'ok' ? 'ok' : 'fail',
  })
}).catch(() => {})

// ── 下载任务全局追踪 ──
listen<{ fileName: string; repoId: string; source: string; downloaded: number; total: number; speed: number }>('download-progress', (e) => {
  const s = useAppStore.getState()
  const { fileName, repoId, source, downloaded, total, speed } = e.payload
  s.setDownloadTasks({ ...s.downloadTasks, [fileName]: { fileName, repoId, source, downloaded, total, speed, status: 'active' } })
}).catch(() => {})

listen<{ fileName: string; repoId: string; source: string; path: string }>('download-complete', (e) => {
  const s = useAppStore.getState()
  const { fileName, repoId, source, path } = e.payload
  const prev = s.downloadTasks[fileName]
  s.setDownloadTasks({ ...s.downloadTasks, [fileName]: { ...(prev || { fileName, repoId, source, downloaded: 0, total: 0, speed: 0 }), status: 'completed', path } })
  useAppStore.getState().processDownloadQueue!()
}).catch(() => {})

listen<{ fileName: string; repoId: string; source: string }>('download-cancelled', (e) => {
  const s = useAppStore.getState()
  const prev = s.downloadTasks[e.payload.fileName]
  s.setDownloadTasks({ ...s.downloadTasks, [e.payload.fileName]: { ...(prev || { fileName: e.payload.fileName, repoId: e.payload.repoId, source: e.payload.source, downloaded: 0, total: 0, speed: 0 }), status: 'cancelled' } })
  useAppStore.getState().processDownloadQueue!()
}).catch(() => {})

listen<{ fileName: string; repoId: string; source: string; error: string }>('download-error', (e) => {
  const s = useAppStore.getState()
  const prev = s.downloadTasks[e.payload.fileName]
  s.setDownloadTasks({ ...s.downloadTasks, [e.payload.fileName]: { ...(prev || { fileName: e.payload.fileName, repoId: e.payload.repoId, source: e.payload.source, downloaded: 0, total: 0, speed: 0 }), status: 'error', error: e.payload.error } })
  useAppStore.getState().processDownloadQueue!()
}).catch(() => {})
