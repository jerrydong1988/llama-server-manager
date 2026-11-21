import type { EngineKind, GeneratedServerCommand, InstanceConfig } from '../store/types'
import { invokeApp } from './ipc'

export function generateServerCommand(request: { config: InstanceConfig; engineExe: string; requireCompleteProbe: boolean }) {
  return invokeApp<GeneratedServerCommand>('generate_server_command', request)
}

export function resolveDefaultInstanceEngine(engineKind: EngineKind) {
  return invokeApp<string>('resolve_default_instance_engine', { engineKind })
}

export function startServer(request: { instanceId: string; config: InstanceConfig }) {
  return invokeApp<void>('start_server', request)
}

export function stopServer(instanceId: string) {
  return invokeApp<void>('stop_server', { instanceId })
}
