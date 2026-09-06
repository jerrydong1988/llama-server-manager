import { useCallback, useEffect, useRef, useState } from 'react'
import type { MouseEvent, RefObject } from 'react'
import { defaultRangeExtractor } from '@tanstack/react-virtual'
import type { Range } from '@tanstack/react-virtual'
import type { LogEntry } from '../store/types'

type SelectionSession = {
  entries: LogEntry[]
  start: number
  end: number
}

function selectedRow(node: Node | null, container: HTMLElement): number | null {
  const element = node instanceof Element ? node : node?.parentElement
  const row = element?.closest<HTMLElement>('[data-log-row]')
  if (!row || !container.contains(row)) return null
  const index = Number(row.dataset.logRow)
  return Number.isInteger(index) ? index : null
}

/** Keep native text-selection endpoints mounted, even outside the viewport. */
export function useLogSelection(entries: LogEntry[], containerRef: RefObject<HTMLDivElement>) {
  const [session, setSession] = useState<SelectionSession | null>(null)
  const draggingRef = useRef(false)

  const clearSelection = useCallback(() => {
    draggingRef.current = false
    const selection = window.getSelection()
    const container = containerRef.current
    if (container && selection && (
      container.contains(selection.anchorNode) || container.contains(selection.focusNode)
    )) selection.removeAllRanges()
    setSession(null)
  }, [containerRef])

  const beginSelection = useCallback((event: MouseEvent<HTMLDivElement>) => {
    if (event.button !== 0 || !(event.target instanceof Node)) return false
    const row = selectedRow(event.target, event.currentTarget)
    if (row === null) return false
    draggingRef.current = true
    setSession(previous => event.shiftKey && previous ? previous : {
      entries: previous?.entries ?? entries,
      start: row,
      end: row,
    })
    return true
  }, [entries])

  useEffect(() => {
    const synchronizeSelection = () => {
      const container = containerRef.current
      const selection = window.getSelection()
      if (!container || !selection || selection.isCollapsed) {
        if (!draggingRef.current) setSession(null)
        return
      }
      const anchor = selectedRow(selection.anchorNode, container)
      const focus = selectedRow(selection.focusNode, container)
      if (anchor === null && focus === null) {
        if (!draggingRef.current) setSession(null)
        return
      }
      setSession(previous => {
        const start = Math.min(anchor ?? focus!, focus ?? anchor!)
        const end = Math.max(anchor ?? focus!, focus ?? anchor!)
        if (previous?.start === start && previous.end === end) return previous
        return { entries: previous?.entries ?? entries, start, end }
      })
    }
    const finishDrag = () => {
      draggingRef.current = false
      synchronizeSelection()
    }
    document.addEventListener('selectionchange', synchronizeSelection)
    window.addEventListener('mouseup', finishDrag)
    window.addEventListener('blur', finishDrag)
    return () => {
      document.removeEventListener('selectionchange', synchronizeSelection)
      window.removeEventListener('mouseup', finishDrag)
      window.removeEventListener('blur', finishDrag)
    }
  }, [containerRef, entries])

  const rangeExtractor = useCallback((range: Range) => {
    const visible = defaultRangeExtractor(range)
    if (!session || visible.length === 0) return visible
    // Render the entire interval: native copy must include every intermediate
    // row, and both selection endpoints must survive dragging in either direction.
    const start = Math.min(visible[0], session.start)
    const end = Math.min(range.count - 1, Math.max(visible[visible.length - 1], session.end))
    return Array.from({ length: end - start + 1 }, (_, index) => start + index)
  }, [session])

  return {
    selectedLogs: session?.entries ?? entries,
    selectionActive: session !== null,
    beginSelection,
    clearSelection,
    rangeExtractor,
  }
}
