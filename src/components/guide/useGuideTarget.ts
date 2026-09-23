import { useEffect, useState } from 'react'

// A bounded wait handles lazy pages without polling for the whole tour. There is
// no overlay: normal controls, native dialogs and nested model pickers stay usable.
export function useGuideTarget(selector: string, enabled: boolean, revision: string) {
  const [status, setStatus] = useState<'loading' | 'ready' | 'missing'>('loading')
  useEffect(() => {
    if (!enabled) return
    let disposed = false
    let frame = 0
    let target: HTMLElement | null = null
    let previous = ''
    let stableFrames = 0
    const deadline = performance.now() + 5000
    setStatus('loading')
    const check = () => {
      if (disposed) return
      const element = document.querySelector<HTMLElement>(selector)
      const rect = element?.getBoundingClientRect()
      if (element && rect && rect.width > 0 && rect.height > 0) {
        const next = [rect.x, rect.y, rect.width, rect.height].map(value => Math.round(value)).join(',')
        stableFrames = next === previous ? stableFrames + 1 : 0
        previous = next
        if (stableFrames >= 2) {
          target = element
          element.classList.add('guide-tour-target')
          element.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'instant' })
          setStatus('ready')
          return
        }
      } else {
        stableFrames = 0
        previous = ''
      }
      if (performance.now() >= deadline) {
        setStatus('missing')
        return
      }
      frame = requestAnimationFrame(check)
    }
    frame = requestAnimationFrame(check)
    return () => {
      disposed = true
      cancelAnimationFrame(frame)
      target?.classList.remove('guide-tour-target')
    }
  }, [selector, enabled, revision])
  return status
}
