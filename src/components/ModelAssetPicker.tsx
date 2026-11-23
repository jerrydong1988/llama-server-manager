import { useLayoutEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from 'react'
import { Search, X } from 'lucide-react'
import { useI18n } from '../i18n'
import { buildModelTrees } from '../modelRepositoryTree'
import type { ModelInfo } from '../store/types'
import { ModelRepositoryTree } from './ModelRepositoryTree'
import { Button, TextInput } from './ui'

export type ModelAssetPickerTarget = 'model' | 'draft' | 'mmproj'

interface Props {
  target: ModelAssetPickerTarget
  models: ModelInfo[]
  modelDirs: string[]
  selectedPath: string
  collapsed: Set<string>
  onCollapsedChange: Dispatch<SetStateAction<Set<string>>>
  includeEmptyRoots?: boolean
  description?: string
  emptyLabel?: string
  onPick: (model: ModelInfo) => void
  onClose: () => void
}

export function ModelAssetPicker({
  target, models, modelDirs, selectedPath, collapsed, onCollapsedChange, includeEmptyRoots,
  description, emptyLabel, onPick, onClose,
}: Props) {
  const { t } = useI18n()
  const [query, setQuery] = useState('')
  const dialog = useRef<HTMLDialogElement>(null)
  const roots = useMemo(() => {
    const visibleModels = target === 'mmproj' ? models.filter(model => model.file_type === 'mmproj') : models
    const trees = buildModelTrees(modelDirs, visibleModels)
    return includeEmptyRoots ? trees : trees.filter(tree => tree.orderedChildren?.length)
  }, [target, models, modelDirs, includeEmptyRoots])

  useLayoutEffect(() => {
    const element = dialog.current
    element?.showModal()
    return () => element?.close()
  }, [])

  return (
    <dialog ref={dialog} aria-modal="true" aria-label={t.modelRepo.selectFromRepo}
      className="model-picker-dialog console-surface console-popover"
      onCancel={event => { event.preventDefault(); onClose() }}>
      <header className="grid shrink-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-5 py-4 sm:grid-cols-[minmax(0,1fr)_minmax(12rem,18rem)_auto]">
        <div className="min-w-0">
          <h3 className="text-lg font-semibold text-ui-ink">{t.modelRepo.selectFromRepo}</h3>
          {description && <p className="mt-1 text-sm text-ui-muted">{description}</p>}
        </div>
        <TextInput type="search" value={query} onChange={event => setQuery(event.target.value)}
          aria-label={t.modelRepo.searchPlaceholder} placeholder={t.modelRepo.searchPlaceholder}
          leadingIcon={<Search size={16} />} className="col-span-2 row-start-2 sm:col-span-1 sm:col-start-2 sm:row-start-1" />
        <Button onClick={onClose} variant="subtle" size="icon" aria-label={t.appShell.close}
          className="col-start-2 row-start-1 sm:col-start-3"><X size={18} /></Button>
      </header>
      <ModelRepositoryTree roots={roots} query={query} collapsed={collapsed} onCollapsedChange={onCollapsedChange}
        selectedPath={selectedPath} revealSelectedOnOpen emptyLabel={emptyLabel ?? t.modelRepo.noModels}
        canSelect={model => target === 'mmproj' || model.file_type !== 'mmproj'} onSelect={onPick} />
    </dialog>
  )
}
