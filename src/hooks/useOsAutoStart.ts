import { useEffect, useRef, useState } from 'react'
import { invokeApp as invoke } from '../lib/ipc'

export function useOsAutoStart() {
  const [enabled, setEnabled] = useState(false)
  const mutationRef = useRef<Promise<void>>(Promise.resolve())
  const mutationGenerationRef = useRef(0)
  const confirmedRef = useRef(false)

  useEffect(() => {
    let disposed = false
    let readGeneration = 0
    const refresh = () => {
      const generation = mutationGenerationRef.current
      const read = ++readGeneration
      void invoke<boolean>('is_autostart_enabled')
        .then(actual => {
          if (!disposed && read === readGeneration && mutationGenerationRef.current === generation) {
            confirmedRef.current = actual
            setEnabled(actual)
          }
        })
        .catch(() => {})
    }
    const changed = (event: Event) => {
      if ((event as CustomEvent<{ operation?: string }>).detail?.operation === 'system.set_autostart') refresh()
    }
    refresh()
    window.addEventListener('lsm-management-resource-changed', changed)
    window.addEventListener('focus', refresh)
    return () => {
      disposed = true
      window.removeEventListener('lsm-management-resource-changed', changed)
      window.removeEventListener('focus', refresh)
    }
  }, [])

  const updateEnabled = (next: boolean) => {
    const generation = ++mutationGenerationRef.current
    setEnabled(next)
    mutationRef.current = mutationRef.current
      .catch(() => {})
      .then(async () => {
        if (next) await invoke('enable_autostart')
        else await invoke('disable_autostart')
        confirmedRef.current = next
      })
      .catch(async () => {
        if (mutationGenerationRef.current !== generation) return
        try {
          const actual = await invoke<boolean>('is_autostart_enabled')
          if (mutationGenerationRef.current !== generation) return
          confirmedRef.current = actual
          setEnabled(actual)
        } catch {
          if (mutationGenerationRef.current === generation) setEnabled(confirmedRef.current)
        }
      })
  }

  return { enabled, updateEnabled }
}
