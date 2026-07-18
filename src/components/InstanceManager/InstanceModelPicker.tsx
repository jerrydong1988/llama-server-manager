import { ChevronDown, ChevronRight, File, FolderOpen, Image, X } from 'lucide-react'
import type { ModelInfo } from '../../store/types'
import { useI18n } from '../../i18n'
import { isPathWithinRoot, normalizePath, pathJoin } from '../../utils/path'
import { Button, Surface } from '../ui'
import { findMatchingProjector } from '../../modelProjector'

interface PickerNode {
  name: string
  path: string
  isDir: boolean
  children?: Map<string, PickerNode>
  model?: ModelInfo
}

function buildTree(rootDir: string, models: ModelInfo[]): PickerNode {
  const normalizedRoot = normalizePath(rootDir)
  const root: PickerNode = { name: rootDir, path: normalizedRoot, isDir: true, children: new Map() }
  for (const model of models) {
    const modelPath = normalizePath(model.path)
    if (!isPathWithinRoot(modelPath, normalizedRoot)) continue
    const relative = modelPath.slice(normalizedRoot.length).replace(/^\/+/, '')
    if (!relative) continue
    const parts = relative.split('/')
    let cursor = root
    for (let index = 0; index < parts.length; index += 1) {
      const part = parts[index]
      if (index === parts.length - 1) {
        cursor.children!.set(part, { name: part, path: model.path, isDir: false, model })
      } else {
        if (!cursor.children!.has(part)) cursor.children!.set(part, { name: part, path: pathJoin(cursor.path, part), isDir: true, children: new Map() })
        cursor = cursor.children!.get(part)!
      }
    }
  }
  return root
}

export function InstanceModelPicker({ models, modelDirs, collapsed, onToggle, onPick, onClose }: {
  models: ModelInfo[]
  modelDirs: string[]
  collapsed: Set<string>
  onToggle: (path: string) => void
  onPick: (model: ModelInfo, mmprojPath: string) => void
  onClose: () => void
}) {
  const { t } = useI18n()

  const renderNode = (node: PickerNode, depth: number): JSX.Element => {
    if (node.isDir) {
      const isCollapsed = collapsed.has(node.path)
      return (
        <div key={node.path}>
          <button type="button" onClick={() => onToggle(node.path)} style={{ paddingLeft: `${depth * 12 + 4}px` }} className="flex w-full items-center gap-1.5 rounded py-1 text-left text-xs hover:bg-slate-100 dark:hover:bg-slate-800">
            {isCollapsed ? <ChevronRight className="h-3 w-3 shrink-0 text-slate-400" /> : <ChevronDown className="h-3 w-3 shrink-0 text-slate-400" />}
            <FolderOpen className="h-3 w-3 shrink-0 text-amber-500" />
            <span className="truncate font-medium">{node.name}</span>
          </button>
          {!isCollapsed && node.children && [...node.children.values()]
            .sort((left, right) => left.isDir !== right.isDir ? (left.isDir ? -1 : 1) : left.name.localeCompare(right.name))
            .map(child => renderNode(child, depth + 1))}
        </div>
      )
    }

    const model = node.model!
    if (model.file_type === 'mmproj') {
      return (
        <div key={node.path} style={{ paddingLeft: `${depth * 12 + 20}px` }} className="flex items-center gap-2 py-1 pr-2 text-xs text-slate-500">
          <Image className="h-3 w-3 shrink-0 text-purple-500" />
          <span className="flex-1 truncate">{model.name}</span>
          <span className="shrink-0 text-xs text-purple-400">{t.modelRepo.typeMmprojShort}</span>
        </div>
      )
    }

    const mmproj = findMatchingProjector(model, models)
    return (
      <button type="button" key={node.path} onClick={() => onPick(model, mmproj?.path || '')} style={{ paddingLeft: `${depth * 12 + 20}px` }} className="flex w-full items-center gap-2 rounded py-1 pr-2 text-left text-xs hover:bg-blue-50 dark:hover:bg-blue-950/30">
        <File className="h-3 w-3 shrink-0 text-blue-500" />
        <span className="flex-1 truncate">{model.name}</span>
        <span className="shrink-0 text-slate-400">{model.quant_type || ''}</span>
      </button>
    )
  }

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 p-4">
      <Surface className="flex max-h-[80vh] w-full max-w-2xl flex-col overflow-hidden">
        <div className="flex items-center justify-between border-b border-slate-200 bg-slate-50 px-6 py-4 dark:border-slate-800 dark:bg-slate-950/90">
          <h3 className="text-lg font-semibold text-slate-950 dark:text-slate-50">{t.modelRepo.selectFromRepo}</h3>
          <Button onClick={onClose} variant="subtle" size="icon" aria-label={t.instance.cancelCreate}><X className="h-5 w-5" /></Button>
        </div>
        <div className="flex-1 overflow-y-auto p-4">
          {modelDirs.map(directory => buildTree(directory, models)).map(tree => renderNode(tree, 0))}
        </div>
      </Surface>
    </div>
  )
}
