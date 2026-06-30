import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { message } from '@tauri-apps/plugin-dialog'
import { pathBasename } from './utils/path'
import { startTransition } from 'react'

// ── 启动性能打点 — 模块级数组，Dashboard 直接读取 ──
export const _startupTimings: { name: string; ms: number }[] = []

// Re-exports from split modules
export type { ModelInfo, EngineInfo, InstanceConfig, Instance, LogEntry, MsFileEntry, DownloadProgress, AppState, WorkerInfo, WorkerDevice, WorkerStatus, Usb4Adapter } from './store/types'
export { defaultInstanceConfig } from './store/defaults'
import type { AppState, ModelInfo, EngineInfo, InstanceConfig, Instance, MsFileEntry, PersistedQueueEntry } from './store/types'

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
  activeTab: (() => { try { return localStorage.getItem('lastTab') || 'dashboard' } catch { return 'dashboard' } })(),
  workers: [],
  clusterScanning: false,
  downloadProgress: {},
  downloadTasks: {},
  downloadQueue: [],
  setDownloadTasks: (tasks) => set({ downloadTasks: tasks }),
  addToDownloadQueue: (entry) => {
    const s = get()
    const id = crypto.randomUUID()
    const fileNames = new Set(entry.files.map(f => f.name))
    
    // 去重：仅当新条目的所有文件都已在队列/下载中时才跳过
    const alreadyInQueue = s.downloadQueue.some(q =>
      q.repoId === entry.repoId && q.source === entry.source &&
      entry.files.every(f => fileNames.has(f.name))) ||
      entry.files.every(f => s.downloadTasks[f.name] && s.downloadTasks[f.name].status !== 'cancelled' && s.downloadTasks[f.name].status !== 'error' && s.downloadTasks[f.name].status !== 'paused')
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
    get().persistQueue()
    get().processDownloadQueue()
  },
  removeFromDownloadQueue: (id) => {
    set(s => ({ downloadQueue: s.downloadQueue.filter(e => e.id !== id) }))
  },
  processDownloadQueue: () => {
    const { downloadQueue, downloadTasks } = get()
    const activeCount = Object.values(downloadTasks).filter(t => t.status === 'active').length
    const MAX = 3
    if (activeCount >= MAX || downloadQueue.length === 0) return

    const next = downloadQueue[0]
    set(s => ({ downloadQueue: s.downloadQueue.filter(e => e.id !== next.id) }))

    // Mark as active
    const tasks = { ...get().downloadTasks }
    for (const f of next.files) {
      tasks[f.name] = { fileName: f.name, repoId: next.repoId, source: next.source, downloaded: 0, total: f.size, speed: 0, status: 'active' }
    }
    set({ downloadTasks: tasks })

    // Fire-and-forget — don't block, allows up to MAX concurrent downloads
    ;(async () => {
      try {
        if (next.source === 'modelscope') {
          await invoke('download_modelscope_files', { repoId: next.repoId, files: next.files, saveDir: next.saveDir })
        } else {
          await invoke('download_huggingface_files', { repoId: next.repoId, files: next.files, saveDir: next.saveDir })
        }
      } catch { /* events handle error state */ } finally {
        get().processDownloadQueue()
      }
    })()

    // Try to fill remaining concurrency slots immediately
    get().processDownloadQueue()
  },
  setModels: (models) => set({ models }),
  setEngines: (engines) => set({ engines }),
  setModelDirs: (dirs) => { set({ modelDirs: dirs }); get().saveConfig() },
  setEngineDirs: (dirs) => { set({ engineDirs: dirs }); get().saveConfig() },
  setDefaultEngineId: (id) => { set({ defaultEngineId: id }); get().saveConfig() },
  setActiveConfigInstanceId: (id) => set({ activeConfigInstanceId: id }),
  setActiveTab: (tab) => { set({ activeTab: tab }); try { localStorage.setItem('lastTab', tab) } catch {} },
  setDarkMode: (dm) => { set({ darkMode: dm }); document.documentElement.classList.toggle('dark', dm); get().saveConfig() },

  addInstance: (instance) => { set((s) => ({ instances: [...s.instances, instance] })); get().saveConfig() },
  updateInstance: (id, partial) => set((s) => ({
    instances: s.instances.map((i) => (i.id === id ? { ...i, ...partial } : i)),
  })),
  deleteInstance: (id) => set((s) => ({
    instances: s.instances.filter((i) => i.id !== id),
  })),

  moveInstance: (id, direction) => {
    const s = get()
    const idx = s.instances.findIndex(i => i.id === id)
    if (idx < 0) return
    const target = direction === 'up' ? idx - 1 : idx + 1
    if (target < 0 || target >= s.instances.length) return
    const arr = [...s.instances];
    [arr[idx], arr[target]] = [arr[target], arr[idx]]
    set({ instances: arr })
    get().saveConfig()
  },

  renameInstance: (id, name) => {
    const s = get()
    const inst = s.instances.find(i => i.id === id)
    if (!inst) return
    const newConfig = { ...inst.config, name }
    const updated = s.instances.map(i => i.id === id ? { ...i, name, config: newConfig } : i)
    set({ instances: updated })
    get().saveConfig()
  },

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
    const t0 = performance.now()
    set({ isLoading: true })
    try {
      // Native init timing: elapsed from Rust main() start to loadConfig call
      invoke<number>('get_startup_elapsed').then(ms => {
        _startupTimings.push({ name: 'native-init', ms })
      }).catch(() => {})

      // 快速路径：initialization_script 注入的配置（绕过 IPC 冷启动）
      // 慢速路径：IPC fallback（后续重载或热重载时用）
      type GlobalConfigShape = {
        instances: Record<string, InstanceConfig>
        model_dirs: string[]
        engine_dirs: string[]
        default_engine_id: string
        running: Record<string, { instance_id: string; pid: number; port: number; host: string; start_time?: number }>
        instance_order: string[]
        last_tab: string
        dark_mode: boolean
      }
      const injected = (window as any).__INITIAL_CONFIG__ as GlobalConfigShape | undefined
      if (injected) {
        ;(window as any).__INITIAL_CONFIG__ = null
        _startupTimings.push({ name: 'config-source', ms: 0 })
        processConfig(injected)
      } else {
        const t_ipc = performance.now()
        const global = await invoke<GlobalConfigShape>('load_config')
        _startupTimings.push({ name: 'config-source', ms: Math.round(performance.now() - t_ipc) })
        processConfig(global)
      }

      function processConfig(global: GlobalConfigShape) {
        const runningIds = new Set(Object.keys(global.running || {}))
        const order = global.instance_order || Object.keys(global.instances)
        let list: Instance[] = Object.entries(global.instances).map(([id, config]) => {
          const st = global.running?.[id]?.start_time ?? 0
          return {
          id, name: config.name || '未命名实例', status: runningIds.has(id) ? 'running' as const : 'stopped' as const,
          model: pathBasename(config.model_path),
          port: config.port,         healthCheck: 'pending' as const, config,
          startTime: st > 0 ? st * 1000 : undefined,
        }})
        list.sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id))
        startTransition(() => {
          set({
            instances: list,
            modelDirs: global.model_dirs || [],
            engineDirs: global.engine_dirs || [],
            defaultEngineId: global.default_engine_id || null,
          })
        })
        if (global.dark_mode !== undefined) {
          document.documentElement.classList.toggle('dark', global.dark_mode)
          startTransition(() => { set({ darkMode: !!global.dark_mode }) })
        }
        console.log('[loadConfig] model_dirs:', JSON.stringify(global.model_dirs), 'engine_dirs:', JSON.stringify(global.engine_dirs))
        invoke<[ModelInfo[], EngineInfo[], PersistedQueueEntry[]]>('load_app_data', { paths: global.model_dirs || [], enginePaths: global.engine_dirs || [] })
          .then(([models, engines, queue]) => {
            startTransition(() => { set({ models, engines }) })
            if (queue?.length > 0) startTransition(() => { get().restoreDownloadQueue(queue) })
          })
          .catch(() => {
            invoke<EngineInfo[]>('scan_engines', { paths: global.engine_dirs || [] })
              .then(engines => startTransition(() => { set({ engines }) }))
              .catch(() => {})
          })
      }
    } catch (e) { console.error('load_config error:', e) }
    set({ isLoading: false })
    _startupTimings.push({ name: 'loadConfig', ms: Math.round(performance.now() - t0) })
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

  // ── 下载队列持久化 ──────────────────────────────────────────
  restoreDownloadQueue: (entries: PersistedQueueEntry[]) => {
    const s = get()
    const now = Date.now()
    let hasPending = false
    for (const entry of entries) {
      // 跳过已完成的
      const allDone = entry.files.every(f => s.downloadTasks[f.name]?.status === 'completed')
      if (allDone) continue

      const id = entry.id || crypto.randomUUID()
      const status = (entry.status === 'active' ? 'queued' : entry.status || 'queued') as 'queued' | 'active' | 'paused' | 'completed' | 'cancelled' | 'error'
      set(st => ({
        downloadQueue: [...st.downloadQueue, {
          id, repoId: entry.repo_id,
          source: entry.source as 'modelscope' | 'huggingface',
          files: entry.files,
          saveDir: entry.save_dir,
          addedAt: entry.added_at || now,
        }]
      }))
      const tasks = { ...get().downloadTasks }
      for (const f of entry.files) {
        tasks[f.name] = { fileName: f.name, repoId: entry.repo_id, source: entry.source, downloaded: 0, total: f.size, speed: 0, status }
      }
      set({ downloadTasks: tasks })
      hasPending = true
    }
    if (hasPending) get().processDownloadQueue()
  },

  persistQueue: () => {
    const { downloadQueue, downloadTasks } = get()
    const entries = downloadQueue.map(q => ({
          id: q.id, repo_id: q.repoId, source: q.source as string,
      files: q.files, save_dir: q.saveDir, added_at: q.addedAt,
      status: downloadTasks[q.files[0]?.name]?.status || 'queued',
    }))
    // Also include active tasks not in queue
    const active = Object.values(get().downloadTasks).filter(t => t.status === 'active')
    if (active.length > 0 && !downloadQueue.some(q => q.files.some(f => active.some(a => a.fileName === f.name)))) {
      const activeByRepo = new Map<string, typeof active>()
      for (const t of active) {
        const key = `${t.source}:${t.repoId}`
        if (!activeByRepo.has(key)) activeByRepo.set(key, [])
        activeByRepo.get(key)!.push(t)
      }
      for (const [, files] of activeByRepo) {
        entries.push({
          id: crypto.randomUUID(), repo_id: files[0].repoId, source: files[0].source,
          files: files.map(f => ({ name: f.fileName, path: '', size: f.total, file_type: 'model' })),
          save_dir: '', added_at: Date.now(), status: 'active',
        })
      }
    }
    invoke('persist_download_queue', { queue: entries }).catch(() => {})
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
  const pathFlags = new Set(['-m', '-md', '--mmproj', '--lora', '--lora-init-without-apply'])
  const shortParts: string[] = []
  for (let i = 1; i < tokens.length; i++) {
    const s = tokens[i].replace(/^"|"$/g, '')
    if (pathFlags.has(s) && i + 1 < tokens.length) {
      const val = tokens[i + 1].replace(/^"|"$/g, '')
      shortParts.push(s + ' "' + (val.split(/[\\/]/).pop() || val) + '"')
      i++ // skip value token
    } else {
      shortParts.push(tokens[i])
    }
  }
  const shortCmd = shortParts.join(' ')
  lines.push('\u2502')
  lines.push(`\u2502 \u5B8C\u6574: ${shortCmd}`)

  return lines.join('\n')
}

declare global {
  interface Window { __lsm_listeners_registered?: boolean }
}

const _g = window as Window & typeof globalThis
if (!_g.__lsm_listeners_registered) {
_g.__lsm_listeners_registered = true

// ── 启动计时事件 (Rust 侧 emit) ──
listen<{ name: string; ms: number }>('startup-timing', (event) => {
  _startupTimings.push({ name: event.payload.name, ms: event.payload.ms })
}).catch(() => {})

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
let _lastProgressUpdate = 0
listen<{ fileName: string; repoId: string; source: string; downloaded: number; total: number; speed: number }>('download-progress', (e) => {
  const now = Date.now()
  if (now - _lastProgressUpdate < 200) return
  _lastProgressUpdate = now
  const s = useAppStore.getState()
  const { fileName, repoId, source, downloaded, total, speed } = e.payload
  s.setDownloadTasks({ ...s.downloadTasks, [fileName]: { fileName, repoId, source, downloaded, total, speed, status: 'active' } })
}).catch(() => {})

listen<{ fileName: string; repoId: string; source: string; path: string }>('download-complete', (e) => {
  const s = useAppStore.getState()
  const { fileName, repoId, source, path } = e.payload
  const prev = s.downloadTasks[fileName]
  s.setDownloadTasks({ ...s.downloadTasks, [fileName]: { ...(prev || { fileName, repoId, source, downloaded: 0, total: 0, speed: 0 }), status: 'completed', path } })
  s.persistQueue()
  useAppStore.getState().processDownloadQueue!()
  useAppStore.getState().scanModels(useAppStore.getState().modelDirs)
}).catch(() => {})

listen<{ fileName: string; repoId: string; source: string }>('download-cancelled', (e) => {
  const s = useAppStore.getState()
  const prev = s.downloadTasks[e.payload.fileName]
  s.setDownloadTasks({ ...s.downloadTasks, [e.payload.fileName]: { ...(prev || { fileName: e.payload.fileName, repoId: e.payload.repoId, source: e.payload.source, downloaded: 0, total: 0, speed: 0 }), status: 'cancelled' } })
  s.persistQueue()
  useAppStore.getState().processDownloadQueue()
}).catch(() => {})

listen<{ fileName: string; repoId: string; source: string; error: string }>('download-error', (e) => {
  const s = useAppStore.getState()
  const prev = s.downloadTasks[e.payload.fileName]
  s.setDownloadTasks({ ...s.downloadTasks, [e.payload.fileName]: { ...(prev || { fileName: e.payload.fileName, repoId: e.payload.repoId, source: e.payload.source, downloaded: 0, total: 0, speed: 0 }), status: 'error', error: e.payload.error } })
  s.persistQueue()
  useAppStore.getState().processDownloadQueue()
}).catch(() => {})


listen<{ fileName: string }>('download-removed', (e) => {
  const s = useAppStore.getState()
  const tasks = { ...s.downloadTasks }
  delete tasks[e.payload.fileName]
  s.setDownloadTasks(tasks)
  s.persistQueue()
}).catch(() => {})

} // HMR-safe guard
