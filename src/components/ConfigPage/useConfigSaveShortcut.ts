import { useEffect, useRef } from 'react'

export function useConfigSaveShortcut() {
  const saveRef = useRef<() => Promise<void>>(async () => {})

  useEffect(() => {
    const handleSaveShortcut = (event: KeyboardEvent) => {
      if (!event.ctrlKey || (event.key !== 's' && event.key !== 'S') || event.isComposing) return
      event.preventDefault()
      void saveRef.current()
    }

    window.addEventListener('keydown', handleSaveShortcut)
    return () => window.removeEventListener('keydown', handleSaveShortcut)
  }, [])

  return saveRef
}
