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
  const dragAnchor = useRef<{ node: Node; offset: number } | null>(null)

  const clearSelection = useCallback(() => {
    draggingRef.current = false
    dragAnchor.current = null
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
    const existing = window.getSelection()
    const caret = document.caretRangeFromPoint?.(event.clientX, event.clientY)
    dragAnchor.current = event.shiftKey && existing?.anchorNode && event.currentTarget.contains(existing.anchorNode)
      ? { node: existing.anchorNode, offset: existing.anchorOffset }
      : caret ? { node: caret.startContainer, offset: caret.startOffset } : null
    setSession(previous => event.shiftKey && previous ? previous : {
      entries: previous?.entries ?? entries,
      start: row,
      end: row,
    })
    return true
  }, [entries])

  useEffect(() => {
    let pointer: { x: number; y: number } | null = null
    let frame = 0
    const container = containerRef.current
    // When the console ends at the window edge, native autoscroll can move the
    // selection into the surrounding workspace. Keep its end in the visible log.
    const extendAtEdge = () => {
      frame = 0
      const anchor = dragAnchor.current
      if (!draggingRef.current || !pointer || !container || !anchor || !container.contains(anchor.node)) return
      const rect = container.getBoundingClientRect()
      const top = Math.max(0, rect.top), bottom = Math.min(window.innerHeight, rect.bottom)
      if (bottom <= top || (pointer.y >= top && pointer.y < bottom)) return
      const down = pointer.y >= bottom
      const x = Math.max(rect.left + 1, Math.min(pointer.x, rect.right - 1))
      const row = document.elementFromPoint(x, down ? bottom - 2 : top + 2)?.closest('[data-log-row]')
      const text = row?.querySelector('span:last-child')?.firstChild
      const selection = window.getSelection()
      if (!selection || !text || !container.contains(text)) return
      const offset = down ? text.textContent?.length ?? 0 : 0
      if (selection.focusNode !== text || selection.focusOffset !== offset) {
        selection.setBaseAndExtent(anchor.node, anchor.offset, text, offset)
      }
    }
    const scheduleEdge = () => {
      if (draggingRef.current && pointer && !frame) frame = requestAnimationFrame(extendAtEdge)
    }
    const trackPointer = (event: globalThis.MouseEvent) => {
      if (!draggingRef.current) return
      pointer = { x: event.clientX, y: event.clientY }
      scheduleEdge()
    }
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
      dragAnchor.current = null
      pointer = null
      synchronizeSelection()
    }
    container?.addEventListener('scroll', scheduleEdge, { passive: true })
    window.addEventListener('mousemove', trackPointer)
    document.addEventListener('selectionchange', synchronizeSelection)
    window.addEventListener('mouseup', finishDrag)
    window.addEventListener('blur', finishDrag)
    return () => {
      cancelAnimationFrame(frame)
      container?.removeEventListener('scroll', scheduleEdge)
      window.removeEventListener('mousemove', trackPointer)
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
