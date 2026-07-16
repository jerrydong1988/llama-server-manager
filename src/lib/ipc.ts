import { invoke as tauriInvoke, type InvokeArgs } from '@tauri-apps/api/core'

export type AppErrorCode = 'VALIDATION' | 'CONFLICT' | 'NOT_FOUND' | 'TIMEOUT' | 'NETWORK' | 'IO' | 'INTERNAL'

export interface AppErrorPayload {
  code: AppErrorCode | string
  message: string
  retryable: boolean
  context?: Record<string, string>
}

export class AppInvokeError extends Error implements AppErrorPayload {
  code: AppErrorCode | string
  retryable: boolean
  context?: Record<string, string>

  constructor(payload: AppErrorPayload) {
    super(payload.message)
    this.name = 'AppInvokeError'
    this.code = payload.code
    this.retryable = payload.retryable
    this.context = payload.context
  }

  override toString() {
    return this.message
  }
}

function classifyLegacyError(message: string): AppErrorPayload {
  const normalized = message.toLowerCase()
  if (/already|conflict|已在|冲突/.test(normalized)) return { code: 'CONFLICT', message, retryable: false }
  if (/not found|未找到/.test(normalized)) return { code: 'NOT_FOUND', message, retryable: false }
  if (/timeout|超时/.test(normalized)) return { code: 'TIMEOUT', message, retryable: true }
  if (/connect|network|网络|连接/.test(normalized)) return { code: 'NETWORK', message, retryable: true }
  if (/invalid|required|无效|必须/.test(normalized)) return { code: 'VALIDATION', message, retryable: false }
  if (/permission|disk|file|文件|磁盘/.test(normalized)) return { code: 'IO', message, retryable: true }
  return { code: 'INTERNAL', message, retryable: false }
}

export function normalizeInvokeError(error: unknown): AppInvokeError {
  if (error instanceof AppInvokeError) return error
  if (error && typeof error === 'object') {
    const candidate = error as Partial<AppErrorPayload>
    if (typeof candidate.code === 'string' && typeof candidate.message === 'string') {
      return new AppInvokeError({ code: candidate.code, message: candidate.message, retryable: candidate.retryable === true, context: candidate.context })
    }
  }
  return new AppInvokeError(classifyLegacyError(error instanceof Error ? error.message : String(error)))
}

export async function invokeApp<T>(command: string, args?: InvokeArgs): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args)
  } catch (error) {
    throw normalizeInvokeError(error)
  }
}
