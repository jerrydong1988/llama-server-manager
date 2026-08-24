import { useEffect, useRef, useState } from 'react'
import { invokeApp as invoke } from '../lib/ipc'

export function useOsAutoStart() {
  const [enabled, setEnabled] = useState(false)
  const mutationRef = useRef<Promise<void>>(Promise.resolve())
  const mutationGenerationRef = useRef(0)
  const confirmedRef = useRef(false)

  useEffect(() => {
    const generation = mutationGenerationRef.current
    invoke<boolean>('is_autostart_enabled')
      .then(actual => {
        if (mutationGenerationRef.current === generation) {
          confirmedRef.current = actual
          setEnabled(actual)
        }
      })
      .catch(() => {})
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
