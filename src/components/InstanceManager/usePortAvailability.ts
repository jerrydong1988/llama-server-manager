import { useEffect, useRef, useState } from 'react'
import { useAppStore } from '../../store'
import { invokeApp as invoke } from '../../lib/ipc'

type PortAvailabilityLabels = {
  checkingPort: string
  portAvailable: string
  portInUse: string
  portCheckFailed: string
}

export function usePortAvailability(labels: PortAvailabilityLabels) {
  const [portStatus, setPortStatus] = useState('')
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const generationRef = useRef(0)
  const mountedRef = useRef(true)

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
      generationRef.current += 1
      if (timerRef.current) clearTimeout(timerRef.current)
    }
  }, [])

  const schedulePortCheck = (port: number) => {
    if (timerRef.current) clearTimeout(timerRef.current)
    const generation = ++generationRef.current
    setPortStatus(labels.checkingPort)
    timerRef.current = setTimeout(() => {
      timerRef.current = null
      invoke<boolean>('check_port', { port, host: '127.0.0.1' })
        .then(free => {
          if (!mountedRef.current || generation !== generationRef.current) return
          setPortStatus(free ? labels.portAvailable : labels.portInUse)
        })
        .catch(error => {
          if (!mountedRef.current || generation !== generationRef.current) return
          setPortStatus(labels.portCheckFailed)
          useAppStore.getState().addRuntimeWarning(`${labels.portCheckFailed}: ${String(error)}`)
        })
    }, 300)
  }

  return { portStatus, setPortStatus, schedulePortCheck }
}
