import { useLayoutEffect, useRef, useState } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { Box, Database, Image } from 'lucide-react'
import type { ModelInfo } from '../../store'
import { useI18n } from '../../i18n'
import { formatSize } from '../../utils/format'
import { pathsEqual } from '../../utils/path'
import { Badge, PathText } from '../ui'

export function ModelAssetGrid({ models, selectedPath, onSelect }: {
  models: ModelInfo[]
  selectedPath: string | null
  onSelect: (path: string) => void
}) {
  const { t } = useI18n()
  const scrollRef = useRef<HTMLDivElement>(null)
  const [columns, setColumns] = useState(1)

  useLayoutEffect(() => {
    const element = scrollRef.current
    if (!element) return
    setColumns(Math.max(1, Math.floor(element.clientWidth / 260)))
    const observer = new ResizeObserver(([entry]) => {
      setColumns(Math.max(1, Math.floor(entry.contentRect.width / 260)))
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  const virtualizer = useVirtualizer({
    count: Math.ceil(models.length / columns),
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 206,
    getItemKey: index => `${columns}:${models[index * columns]?.path ?? index}`,
    overscan: 3,
  })

  return (
    <div ref={scrollRef} className="h-[520px] overflow-y-auto" data-model-grid>
      <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map(row => (
          <div
            key={row.key}
            ref={virtualizer.measureElement}
            data-index={row.index}
            className="absolute left-0 top-0 grid w-full gap-3 pb-3"
            style={{ transform: `translateY(${row.start}px)`, gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}
          >
            {models.slice(row.index * columns, (row.index + 1) * columns).map(model => {
              const selected = Boolean(selectedPath && pathsEqual(selectedPath, model.path))
              const Icon = model.file_type === 'mmproj' ? Image : model.file_type === 'imatrix' ? Database : Box
              const kind = model.file_type === 'mmproj' ? t.modelRepo.typeMmprojShort : model.file_type === 'imatrix' ? t.modelRepo.typeImatrix : t.modelRepo.typeModelShort
              return (
                <button
                  key={model.path}
                  type="button"
                  aria-pressed={selected}
                  onClick={() => onSelect(model.path)}
                  title={model.path}
                  className={`min-w-0 rounded-xl border p-4 text-left transition ${selected ? 'border-[var(--ui-link)] bg-[var(--ui-control)]' : 'border-transparent bg-[var(--ui-soft)] hover:border-[var(--ui-line)]'}`}
                >
                  <span className="mb-3 flex items-center justify-between gap-2">
                    <span className="ui-tone-blue rounded-lg p-2"><Icon className="h-5 w-5" /></span>
                    <Badge>{kind}</Badge>
                  </span>
                  <span className="line-clamp-2 min-h-10 break-all text-[13px] font-semibold leading-5 text-[var(--ui-text)]">{model.name}</span>
                  <span className="mt-2 flex flex-wrap gap-1.5">
                    {model.quant_type && <span className="ui-chip">{model.quant_type}</span>}
                    {model.architecture && <span className="ui-chip max-w-full truncate">{model.architecture}</span>}
                    <span className="ui-chip">{formatSize(model.size)}</span>
                  </span>
                  <PathText value={model.path} className="mt-3 text-[10px] text-[var(--ui-muted)]" />
                </button>
              )
            })}
          </div>
        ))}
      </div>
      {models.length === 0 && <p className="py-16 text-center text-sm text-[var(--ui-muted)]">{t.modelRepo.noModels}</p>}
    </div>
  )
}
