import { ChevronDown, ChevronRight, File, FolderOpen, Image, X } from 'lucide-react'
import { useI18n } from '../../i18n'
import type { ModelInfo } from '../../store'
import { Button, PathText } from '../ui'
import { buildPickerTree, type PickerNode } from './configWorkspace'

export type ModelAssetPickerTarget = 'model' | 'draft' | 'mmproj'

interface Props {
  target: ModelAssetPickerTarget
  models: ModelInfo[]
  modelDirs: string[]
  collapsed: Set<string>
  description: string
  emptyLabel: string
  onToggle: (path: string) => void
  onPick: (path: string) => void
  onClose: () => void
}

export function ModelAssetPicker({
  target,
  models,
  modelDirs,
  collapsed,
  description,
  emptyLabel,
  onToggle,
  onPick,
  onClose,
}: Props) {
  const { t } = useI18n()
  const visibleModels = target === 'mmproj'
    ? models.filter(model => model.file_type === 'mmproj')
    : models
  const trees = modelDirs
    .map(directory => buildPickerTree(directory, visibleModels))
    .filter(tree => (tree.children?.size ?? 0) > 0)

  const renderNode = (node: PickerNode, depth: number): JSX.Element => {
    if (node.isDir) {
      const isCollapsed = collapsed.has(node.path)
      return (
        <div key={node.path}>
          <button
            type="button"
            onClick={() => onToggle(node.path)}
            style={{ paddingLeft: `${depth * 14 + 8}px` }}
            className="flex w-full items-center gap-2 rounded-lg py-2 pr-3 text-left text-sm text-slate-200 transition hover:bg-slate-800/80"
          >
            {isCollapsed ? <ChevronRight className="h-3.5 w-3.5 shrink-0 text-slate-500" /> : <ChevronDown className="h-3.5 w-3.5 shrink-0 text-slate-500" />}
            <FolderOpen className="h-4 w-4 shrink-0 text-amber-400" />
            {depth === 0 ? (
              <PathText value={node.path} maxLength={78} className="flex-1 text-slate-200" />
            ) : (
              <span className="min-w-0 flex-1 truncate" title={node.name}>{node.name}</span>
            )}
          </button>
          {!isCollapsed && node.children && [...node.children.values()]
            .sort((left, right) => {
              if (left.isDir !== right.isDir) return left.isDir ? -1 : 1
              return left.name.localeCompare(right.name)
            })
            .map(child => renderNode(child, depth + 1))}
        </div>
      )
    }

    const model = node.model!
    if (model.file_type === 'mmproj') {
      if (target === 'mmproj') {
        return (
          <button
            type="button"
            key={node.path}
            onClick={() => onPick(model.path)}
            style={{ paddingLeft: `${depth * 14 + 32}px` }}
            className="flex w-full items-center gap-2 rounded-lg py-2 pr-3 text-left text-sm text-slate-100 transition hover:bg-fuchsia-500/10"
          >
            <Image className="h-4 w-4 shrink-0 text-fuchsia-400" />
            <span className="min-w-0 flex-1 truncate" title={model.path}>{model.name}</span>
            <span className="shrink-0 text-xs text-fuchsia-300">{t.modelRepo.typeMmprojShort}</span>
          </button>
        )
      }

      return (
        <div
          key={node.path}
          style={{ paddingLeft: `${depth * 14 + 32}px` }}
          className="flex items-center gap-2 py-2 pr-3 text-sm text-slate-500"
        >
          <Image className="h-4 w-4 shrink-0 text-fuchsia-400" />
          <span className="min-w-0 flex-1 truncate" title={model.path}>{model.name}</span>
          <span className="shrink-0 text-xs text-fuchsia-300">{t.modelRepo.typeMmprojShort}</span>
        </div>
      )
    }

    return (
      <button
        type="button"
        key={node.path}
        onClick={() => onPick(model.path)}
        style={{ paddingLeft: `${depth * 14 + 32}px` }}
        className="flex w-full items-center gap-2 rounded-lg py-2 pr-3 text-left text-sm text-slate-100 transition hover:bg-blue-500/10"
      >
        <File className="h-4 w-4 shrink-0 text-sky-400" />
        <span className="min-w-0 flex-1 truncate" title={model.path}>{model.name}</span>
        <span className="shrink-0 text-xs text-slate-500">{model.quant_type || ''}</span>
      </button>
    )
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm">
      <div className="flex max-h-[85vh] w-full max-w-3xl flex-col overflow-hidden rounded-lg border border-slate-800 bg-slate-900 shadow-[0_30px_80px_rgba(2,6,23,0.7)]">
        <div className="flex items-center justify-between gap-4 border-b border-slate-800 bg-slate-950/90 px-5 py-4">
          <div className="min-w-0">
            <h3 className="text-lg font-semibold text-slate-50">{t.modelRepo.selectFromRepo}</h3>
            <p className="mt-1 truncate text-sm text-slate-400">{description}</p>
          </div>
          <Button onClick={onClose} variant="subtle" size="icon" aria-label="Close">
            <X className="h-5 w-5" />
          </Button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          {trees.length > 0 ? (
            <div className="min-w-0 space-y-2 rounded-lg border border-slate-800 bg-slate-950/40 p-3">
              {trees.map(tree => renderNode(tree, 0))}
            </div>
          ) : (
            <div className="rounded-lg border border-dashed border-slate-700 bg-slate-950/40 px-4 py-10 text-center text-sm text-slate-400">
              {emptyLabel}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
