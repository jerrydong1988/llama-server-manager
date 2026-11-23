import { useEffect, useMemo, useRef, useState, type Dispatch, type ReactNode, type SetStateAction } from 'react'
import { ChevronRight, File, FolderTree, Image } from 'lucide-react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { useI18n } from '../i18n'
import { matchesModelNode, visibleModelRows, type ModelTreeNode } from '../modelRepositoryTree'
import type { ModelInfo } from '../store/types'
import { formatSize } from '../utils/format'
import { formatPathForDisplay, pathsEqual } from '../utils/path'

interface Props {
  roots: ModelTreeNode[]
  query: string
  collapsed: Set<string>
  onCollapsedChange: Dispatch<SetStateAction<Set<string>>>
  selectedPath?: string | null
  revealSelectedOnOpen?: boolean
  onSelect: (model: ModelInfo) => void
  canSelect?: (model: ModelInfo) => boolean
  emptyLabel?: string
}

const highlightText = (text: string, query: string): ReactNode => {
  if (!query) {
    return text
  }

  const index = text.toLowerCase().indexOf(query.toLowerCase())
  if (index < 0) {
    return text
  }

  return (
    <>
      {text.slice(0, index)}
      <mark className="rounded bg-ui-warn-bg px-0.5 text-ui-warn">
        {text.slice(index, index + query.length)}
      </mark>
      {text.slice(index + query.length)}
    </>
  )
}

export function ModelRepositoryTree({
  roots, query, collapsed, onCollapsedChange, selectedPath, revealSelectedOnOpen, onSelect, canSelect, emptyLabel,
}: Props) {
  const { t } = useI18n()
  const filter = query.trim().toLowerCase()
  // Searching reveals matching ancestors. Collapsing a search result must not
  // change the branches the user left open in the unfiltered tree.
  const [searchCollapsed, setSearchCollapsed] = useState<Set<string>>(new Set())
  useEffect(() => { setSearchCollapsed(new Set()) }, [filter])
  const folded = filter ? searchCollapsed : collapsed
  const rows = useMemo(() => visibleModelRows(roots, filter, folded), [roots, filter, folded])
  const scrollRef = useRef<HTMLDivElement>(null)
  const initialSelection = useRef(revealSelectedOnOpen ? selectedPath : null)
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 39,
    overscan: 14,
    getItemKey: index => rows[index].key,
  })
  const viewportHeight = virtualizer.scrollRect?.height
  useEffect(() => {
    const path = initialSelection.current
    if (!path || !viewportHeight || !rows.length) return
    initialSelection.current = null
    if (filter) return
    const index = rows.findIndex(({ node }) => node.model && pathsEqual(node.model.path, path))
    if (index >= 0) virtualizer.scrollToIndex(index, { align: 'auto' })
  }, [filter, rows, viewportHeight, virtualizer])

  const toggle = (key: string) => {
    const update = filter ? setSearchCollapsed : onCollapsedChange
    update(previous => {
      const next = new Set(previous)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }

  return (
    <div ref={scrollRef} role="region" aria-label={t.modelRepoWorkspace.explorer} className="model-tree-viewport">
      {!rows.length && <p className="px-5 py-8 text-center text-sm text-ui-muted">
        {filter ? t.modelRepoWorkspace.noSearchResults : (emptyLabel ?? t.modelRepoWorkspace.noSearchResults)}
      </p>}
      <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map(item => {
          const { node, branches, key } = rows[item.index]
          const { model, stats } = node
          const expanded = node.isDir ? !folded.has(key) : undefined
          const selectable = !model || !canSelect || canSelect(model)
          const selected = model && selectable ? Boolean(selectedPath && pathsEqual(selectedPath, model.path)) : undefined
          const muted = !selectable || model?.is_shard || (node.isDir && filter && !matchesModelNode(node, filter))
          const label = model?.name ?? formatPathForDisplay(node.name)
          const kind = model?.file_type === 'mmproj' ? t.modelRepo.typeMmprojShort
            : model?.file_type === 'imatrix' ? t.modelRepo.typeImatrix : t.modelRepo.typeModelShort
          const directoryCounts = node.isDir ? [
            stats.models > 0 && `${stats.models} ${t.modelRepo.typeModelShort}`,
            stats.mmproj > 0 && `${stats.mmproj} ${t.modelRepo.mmprojCount}`,
            stats.imatrix > 0 && `${stats.imatrix} ${t.modelRepo.typeImatrix}`,
          ].filter(Boolean).join(' · ') : undefined
          const Row = selectable ? 'button' : 'div'
          const activate = () => {
            if (model) onSelect(model)
            else toggle(key)
          }

          return (
            <div key={item.key} ref={virtualizer.measureElement} data-index={item.index}
              className="absolute left-0 top-0 w-full" style={{ transform: `translateY(${item.start}px)` }}>
              <Row type={selectable ? 'button' : undefined}
                onClick={selectable ? activate : undefined}
                aria-label={node.isDir ? label : undefined} aria-description={directoryCounts || undefined}
                aria-expanded={expanded} aria-pressed={selected} aria-disabled={selectable ? undefined : true}
                data-selected={selected} className="console-list-row model-tree-row">
                <span className="model-tree-prefix" aria-hidden="true">
                  <span className="model-tree-branches">
                    {branches.map((continues, level) => <span key={level} data-continues={continues} />)}
                  </span>
                  <span className="model-tree-disclosure">
                    {node.isDir && <ChevronRight size={14} className={expanded ? 'rotate-90' : undefined} />}
                  </span>
                </span>
                {node.isDir ? <FolderTree aria-hidden="true" className="h-4 w-4 shrink-0 text-ui-warn" />
                  : model?.file_type === 'mmproj' ? <Image aria-hidden="true" className="h-4 w-4 shrink-0 text-fuchsia-700 dark:text-fuchsia-300" />
                  : <File aria-hidden="true" className="h-4 w-4 shrink-0 text-ui-info" />}
                <span className={`min-w-0 flex-1 truncate text-sm ${muted ? 'text-ui-muted' : 'text-ui-ink'}`}
                  title={formatPathForDisplay(node.path)}>{highlightText(label, filter)}</span>
                {model ? <>
                  <span className="model-tree-quant truncate text-right text-xs text-ui-muted" title={model.quant_type ?? undefined}>{model.quant_type}</span>
                  <span className="model-tree-kind justify-end">
                    <span className="truncate rounded-full border border-ui-line px-2 py-0.5 text-[11px] text-ui-body">{kind}</span>
                  </span>
                  <span className="model-tree-size text-xs text-ui-muted">{formatSize(model.size)}</span>
                </> : (
                  <span className="max-w-[48%] shrink-0 truncate text-xs text-ui-muted" title={directoryCounts || undefined}>{directoryCounts}</span>
                )}
              </Row>
            </div>
          )
        })}
      </div>
    </div>
  )
}
