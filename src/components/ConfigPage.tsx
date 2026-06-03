import { useState, useEffect } from 'react'
import { Settings, File, Image, X, FolderOpen, ChevronRight, ChevronDown } from 'lucide-react'
import { useAppStore, type InstanceConfig } from '../store'
import { useI18n } from '../i18n'
import { validateConfig, type Warning } from '../validators'
import { BasicSection, ReasoningSection, PerformanceSection, AdvancedSection } from './ConfigPage/sections'
import { getActiveParams } from './ConfigPage/activeParams'
import { normalizePath, pathBasename, pathDirname, pathJoin } from '../utils/path'

const EMBED_ARCHS = ['bge', 'gte', 'e5', 'text-embedding', 'sentence-bert', 'sentence-t5', 'instructor', 'bert', 'nomic', 'jina']

const ConfigPage = () => {
  const { instances, activeConfigInstanceId, updateInstance, saveConfig, models, modelDirs, engines, defaultEngineId } = useAppStore()
  const { t } = useI18n()
  const inst = instances.find(i => i.id === activeConfigInstanceId)
  const [local, setLocal] = useState<InstanceConfig | null>(null)
  const [saved, setSaved] = useState(false)
  const [showPicker, setShowPicker] = useState(false)
  const [pickerCollapsed, setPickerCollapsed] = useState<Set<string>>(new Set())
  const [saveWarnings, setSaveWarnings] = useState<Warning[]>([])

  useEffect(() => { if (inst) setLocal({ ...inst.config }); else setLocal(null) }, [activeConfigInstanceId, instances])

  const isEmbedding = (() => {
    if (!local?.model_path) return false
    const fname = pathBasename(local.model_path)
    if (fname.toLowerCase().includes('embed')) return true
    const model = models.find(m => m.path === local.model_path)
    if (model?.architecture && EMBED_ARCHS.some(a => model.architecture!.toLowerCase().includes(a))) return true
    return false
  })()

  useEffect(() => { if (isEmbedding && local) { if (!local.embedding) set('embedding', true); if (!local.pooling) set('pooling', 'mean') } }, [isEmbedding, local?.model_path])

  if (!local) return (
    <div className="bg-white dark:bg-gray-800 rounded-lg p-12 text-center border dark:border-gray-700">
      <Settings className="w-12 h-12 mx-auto mb-3 text-gray-400" />
      <p className="text-gray-500">{t.configPage.noInstance}</p>
    </div>
  )

  const set = (k: keyof InstanceConfig, v: any) => setLocal(l => l ? { ...l, [k]: v } : l)
  const pickModel = (modelPath: string) => {
    set('model_path', modelPath)
    const dir = pathDirname(modelPath)
    const mmproj = models.find(m => pathDirname(m.path) === dir && m.file_type === 'mmproj')
    if (mmproj) set('mmproj_path', mmproj.path); else set('mmproj_path', '')
    setShowPicker(false)
  }
  const save = () => {
    if (!local || !inst) return
    const model = models.find(m => m.path === local.model_path)
    const engine = engines.find(e => e.id === (local.engine_id || defaultEngineId || '')) || engines[0]
    const warnings = validateConfig(local, model, engine)
    updateInstance(inst.id, { config: local })
    saveConfig()
    setSaved(true)
    setSaveWarnings(warnings)
    setTimeout(() => { setSaved(false); setSaveWarnings([]) }, 6000)
  }

  const sectionProps = { local, set, t, isEmbedding, onShowPicker: () => setShowPicker(true), activeParams: local ? getActiveParams(local, isEmbedding) : new Set() as Set<keyof InstanceConfig> }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Settings className="w-5 h-5 text-blue-500" />
          <span className="text-lg font-semibold">{t.configPage.title}</span>
          <span className="text-sm text-gray-500">{'\u2014'} {inst?.name}</span>
        </div>
        <button onClick={save} disabled={!local || !inst} className="px-6 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-gray-400 text-white rounded-lg transition-colors">{saved ? t.configPage.saved : t.configPage.save}</button>
      </div>
      {saved && <div className="bg-green-50 dark:bg-green-900/20 rounded-lg p-3 text-sm text-green-600 dark:text-green-400">{t.configPage.savedMsg}{'\u300C'}{inst?.name}{'\u300D\u3002'}{t.configPage.savedHint}</div>}
      {isEmbedding && <div className="bg-blue-50 dark:bg-blue-900/20 rounded-lg p-3 text-sm text-blue-600 dark:text-blue-400">{t.configPage.embeddingBanner}</div>}
      {saveWarnings.length > 0 && (
        <div className="space-y-1.5">
          {saveWarnings.filter(w => w.severity === 'high').map((w, i) => (
            <div key={`h-${i}`} className="bg-red-50 dark:bg-red-900/20 rounded-lg px-3 py-2 text-sm text-red-600 dark:text-red-400">
              {'\u26A0'} {(t.configPage as any)[w.key] || w.key}
            </div>
          ))}
          {saveWarnings.filter(w => w.severity === 'medium').map((w, i) => (
            <div key={`m-${i}`} className="bg-yellow-50 dark:bg-yellow-900/20 rounded-lg px-3 py-2 text-sm text-yellow-700 dark:text-yellow-400">
              {(t.configPage as any)[w.key] || w.key}
            </div>
          ))}
          {saveWarnings.filter(w => w.severity === 'low').map((w, i) => (
            <div key={`l-${i}`} className="bg-sky-50 dark:bg-sky-900/20 rounded-lg px-3 py-2 text-sm text-sky-600 dark:text-sky-400">
              {(t.configPage as any)[w.key] || w.key}
            </div>
          ))}
        </div>
      )}

      <div className="space-y-3">
        <BasicSection {...sectionProps} />
        <ReasoningSection {...sectionProps} />
        <PerformanceSection {...sectionProps} />
        <AdvancedSection {...sectionProps} />
      </div>

      {showPicker && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white dark:bg-gray-800 rounded-lg w-full max-w-2xl max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between px-6 py-4 border-b dark:border-gray-700">
              <h3 className="text-lg font-semibold">{t.modelRepo.selectFromRepo}</h3>
              <button onClick={() => setShowPicker(false)} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded"><X className="w-5 h-5" /></button>
            </div>
            <div className="flex-1 overflow-y-auto p-4">
              {(function TreeRenderer() {
                interface TNode { name: string; path: string; isDir: boolean; children?: Map<string, TNode>; model?: typeof models[0] }
                function buildTree(rootDir: string): TNode {
                  const normDir = normalizePath(rootDir)
                  const root: TNode = { name: rootDir, path: normDir, isDir: true, children: new Map() }
                  const normRoot = normDir.toLowerCase()
                  for (const m of models) {
                    const normPath = normalizePath(m.path).toLowerCase()
                    if (!normPath.startsWith(normRoot)) continue
                    const rel = normalizePath(m.path.substring(rootDir.length)).replace(/^\/+/, '')
                    if (!rel) continue
                    const parts = rel.split('/')
                    let cur = root
                    for (let i = 0; i < parts.length; i++) {
                      if (i === parts.length - 1) { cur.children!.set(parts[i], { name: parts[i], path: m.path, isDir: false, model: m }) }
                      else {
                        if (!cur.children!.has(parts[i])) { cur.children!.set(parts[i], { name: parts[i], path: pathJoin(cur.path, parts[i]), isDir: true, children: new Map() }) }
                        cur = cur.children!.get(parts[i])!
                      }
                    }
                  }
                  return root
                }
                const toggleP = (k: string) => { const n = new Set(pickerCollapsed); if (n.has(k)) n.delete(k); else n.add(k); setPickerCollapsed(n) }
                function renderNode(node: TNode, depth: number): any {
                  if (node.isDir) {
                    const c = pickerCollapsed.has(node.path)
                    return (<div key={node.path}>
                      <button onClick={() => toggleP(node.path)} style={{ paddingLeft: `${depth * 12 + 4}px` }} className="w-full flex items-center gap-1.5 py-1 hover:bg-gray-100 dark:hover:bg-gray-700 rounded text-left text-xs">
                        {c ? <ChevronRight className="w-3 h-3 text-gray-400 shrink-0" /> : <ChevronDown className="w-3 h-3 text-gray-400 shrink-0" />}
                        {depth === 0 ? <FolderOpen className="w-3 h-3 text-yellow-500 shrink-0" /> : <span className="text-xs shrink-0">{'\uD83D\uDCC1'}</span>}
                        <span className="truncate font-medium text-xs">{node.name}</span>
                      </button>
                      {!c && node.children && [...node.children.values()].sort((a, b) => { if (a.isDir !== b.isDir) return a.isDir ? -1 : 1; return a.name.localeCompare(b.name) }).map(ch => renderNode(ch, depth + 1))}
                    </div>)
                  }
                  const m = node.model!
                  if (m.file_type === 'mmproj') return (<div key={node.path} style={{ paddingLeft: `${depth * 12 + 20}px` }} className="flex items-center gap-2 py-1 pr-2 text-xs text-gray-500"><Image className="w-3 h-3 text-purple-500 shrink-0" /><span className="truncate flex-1">{m.name}</span><span className="text-purple-400 shrink-0 text-xs">{t.modelRepo.typeMmprojShort}</span></div>)
                  return (<button key={node.path} onClick={() => pickModel(m.path)} style={{ paddingLeft: `${depth * 12 + 20}px` }} className="w-full flex items-center gap-2 py-1 pr-2 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded text-left text-xs">
                    <File className="w-3 h-3 text-blue-500 shrink-0" /><span className="truncate flex-1">{m.name}</span><span className="text-gray-400 shrink-0">{m.quant_type || ''}</span>
                    <span className="text-gray-400 shrink-0">{m.size > 1024 * 1024 * 1024 ? (m.size / 1024 / 1024 / 1024).toFixed(1) + ' GB' : m.size > 1024 * 1024 ? (m.size / 1024 / 1024).toFixed(1) + ' MB' : m.size > 1024 ? (m.size / 1024).toFixed(1) + ' KB' : m.size + ' B'}</span>
                  </button>)
                }
                return modelDirs.map(d => buildTree(d)).map(t => renderNode(t, 0))
              })()}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

export default ConfigPage
