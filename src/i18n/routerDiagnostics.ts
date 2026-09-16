export function getRouterDiagnosticsLabels(lang: string) {
  const zh = lang === 'zh-CN'
  return {
    history: zh ? '请求检索与诊断' : 'Request history and diagnostics',
    historyHint: zh ? '每页 50 条，明细保留 90 天。以下筛选仅影响明细；刷新后可查看新记录。旧记录未采集的诊断字段显示为“—”。' : '50 requests per page, retained for 90 days. These filters affect details only. Refresh to include new records. Diagnostics missing from older records appear as “—”.',
    errorType: zh ? '错误类型' : 'Error category',
    lookup: zh ? '关联请求 ID' : 'Correlated request ID',
    lookupHint: zh ? '管理器、响应或上游请求 ID（精确匹配）' : 'Manager, response or upstream request ID (exact match)',
    startTime: zh ? '明细开始时间（UTC）' : 'Details start (UTC)',
    endTime: zh ? '明细结束时间（UTC，不含）' : 'Details end (UTC, exclusive)',
    invalidDetailRange: zh ? '请选择上方统计日期范围内有效的起止时间。' : 'Select a valid time interval within the summary date range above.',
    previous: zh ? '上一页' : 'Previous page', next: zh ? '下一页' : 'Next page',
    page: zh ? '页' : 'Page', view: zh ? '查看详情' : 'View details', close: zh ? '收起详情' : 'Close details',
    copy: zh ? '复制诊断摘要' : 'Copy diagnostic summary', copied: zh ? '已复制' : 'Copied',
    copyFailed: zh ? '复制失败，请手动选择摘要。' : 'Copy failed. Select the summary manually.',
    stage: zh ? '发生环节' : 'Failure stage', code: zh ? '错误代码' : 'Error code',
    reason: zh ? '失败原因' : 'Failure reason', advice: zh ? '处理建议' : 'Suggested action',
    responseId: zh ? '响应请求 ID' : 'Response request ID', upstreamId: zh ? '上游请求 ID' : 'Upstream request ID',
    source: zh ? '用量来源' : 'Usage source', budget: zh ? '上下文预检预算' : 'Context preflight budget',
    budgetInput: zh ? '预检输入 Token' : 'Preflight input tokens', budgetOutput: zh ? '预检预留输出 Token' : 'Reserved output tokens',
    window: zh ? '上下文窗口' : 'Context window', excess: zh ? '超出 Token' : 'Excess tokens',
    budgetHint: zh ? '预算不是实际消耗。硬额度开启时使用补齐或归一化后的输出上限；关闭且未设置上限时，预检至少预留 1 Token；未执行或未成功的输入计数显示为未知。' : 'Budgets are not consumption. With hard quotas, preflight uses the injected or normalized output limit; otherwise it reserves at least one output token when no limit is set. Skipped or unsuccessful input counts remain unknown.',
    noGeneration: zh ? '此请求未转发到生成接口，不计生成 Token 消耗。' : 'This request was not forwarded for generation and adds no generation token usage.',
    finish: zh ? '结束原因' : 'Finish reason', cacheWrite: zh ? '缓存写入 Token' : 'Cache write tokens', reasoning: zh ? '推理 Token' : 'Reasoning tokens',
    protocolCoverage: zh ? '各接口用量覆盖率' : 'Usage coverage by endpoint',
    pending: zh ? '待写入记录（最近采样）' : 'Pending records (last sample)',
    lastCommit: zh ? '最近成功写入' : 'Last successful commit', delay: zh ? '最近批次最大写入延迟' : 'Latest batch maximum write delay',
    interruptions: zh ? '异常结束的采集会话' : 'Interrupted recording sessions',
    interruptionHint: zh ? '检测到采集进程未完成正常收尾，可能存在未落盘记录或进行中的请求缺口；缺失数量未知，不能当作零。' : 'A recorder did not shut down cleanly. Uncommitted records or in-flight requests may be missing; the loss count is unknown, not zero.',
    storageScope: zh ? '写入健康信息覆盖全部调用，约每 2 秒采样；不随日期和 Key 筛选变化。历史异常提示会保留。' : 'Writer health covers all calls, sampled about every 2 seconds, independently of date and key filters. Historical warnings remain visible.',
  }
}

const failures: Record<string, [string, string, string, string]> = {
  token_quota_exceeded: ['Token 额度不足', 'Token quota exceeded', '查看已结算与保守占用，缩短请求、降低输出上限，或调整硬额度。', 'Check settled and held tokens, reduce input/output limits, or adjust the hard quota.'],
  token_quota_unavailable: ['额度账不可用', 'Quota ledger unavailable', '请求未转发。检查数据目录可写性、磁盘空间与后台服务日志。', 'Not forwarded. Check data directory permissions, free space and runtime logs.'],
  token_quota_unmetered: ['无法可靠预留额度', 'Cannot reserve tokens reliably', '请求未转发。检查引擎计数／属性接口，或拆分不支持的批量请求。', 'Not forwarded. Check engine count/props endpoints or split unsupported batches.'],
  token_quota_invalid_limit: ['输出上限格式错误', 'Invalid output limit', '检查错误响应 param 指出的字段，使用整数上限；省略或不限量写法会自动补齐默认值。', 'Check the field named by error.param and use an integer limit; missing or unbounded limits receive the configured default.'],
  token_quota_multiple_generations: ['暂不支持多份生成', 'Multiple generations unsupported', '将 n、best_of、num_return_sequences 设为 1 或省略，多份生成请拆成独立请求。', 'Set n, best_of and num_return_sequences to 1 or omit them; use separate requests for multiple generations.'],
  token_quota_unsupported_batch: ['暂不支持批量生成', 'Completion batch unsupported', '将 Completions 的多条 prompt 拆为独立请求；向量和重排序批量不受此限制。', 'Split multiple completion prompts into separate requests; embedding and reranking batches remain supported.'],
  context_length_exceeded: ['上下文超限', 'Context window exceeded', '降低客户端输出上限，或缩短对话历史和工具内容。', 'Reduce the client output limit or shorten history and tool content.'],
  authentication_failed: ['鉴权失败', 'Authentication failed', '检查客户端 API Key 是否有效、启用且与管理器一致。', 'Check that the client API key matches an enabled router key.'],
  permission_denied: ['权限不足', 'Permission denied', '检查 Key 接口权限及允许的来源设置。', 'Check the key scopes and allowed origins.'],
  rate_limited: ['频率限制', 'Rate limited', '降低请求频率，按重试时间稍后重试。', 'Reduce the request rate and follow the retry delay.'],
  queue_timeout: ['排队超时', 'Queue timeout', '减少同时请求数，或检查长时间占用槽位的请求。', 'Reduce concurrency or inspect requests holding slots.'],
  route_unavailable: ['实例不可用', 'Route unavailable', '检查模型路由、实例健康状态和检查点恢复状态。', 'Check model routing, instance health and checkpoint readiness.'],
  route_capacity: ['实例容量不足或不可用', 'Route capacity or availability', '检查实例槽位占用；不要将路由并发设得高于实例实际容量。', 'Check occupied slots and keep router concurrency within instance capacity.'],
  body_capacity: ['请求内存预算不足', 'Request memory budget reached', '减少并发或单个请求的历史与工具内容。', 'Reduce concurrency or request history and tool content.'],
  upstream_error: ['上游返回错误', 'Upstream error', '按请求 ID 查看对应实例日志；原始报错正文不写入统计库。', 'Inspect instance logs using the request ID; raw error bodies are not stored.'],
  upstream_timeout: ['上游响应超时', 'Upstream timeout', '检查实例负载和响应超时设置。', 'Check instance load and response timeouts.'],
  upstream_connection_failed: ['上游连接失败', 'Upstream connection failed', '检查实例进程、端口、网络和上游鉴权。', 'Check the instance process, port, network and upstream authentication.'],
  stream_timeout: ['流空闲超时', 'Stream idle timeout', '检查实例是否仍在推理，并协调客户端与路由的流空闲超时。', 'Check whether inference is still running and coordinate client/router idle timeouts.'],
  stream_interrupted: ['流中断', 'Stream interrupted', '检查客户端连接和实例日志；已有的部分用量仍可能产生消耗。', 'Check the client connection and instance logs; partial usage may have been consumed.'],
  client_cancelled: ['请求取消或连接关闭', 'Request cancelled or connection closed', '确认客户端是否主动停止或超时断开；已报告用量仍保留。', 'Check whether the client stopped or timed out; reported usage is retained.'],
  response_error: ['上游响应读取失败', 'Upstream response read failed', '检查实例输出格式、响应大小和连接状态。', 'Check response format, size and connection state.'],
  invalid_request: ['请求格式错误', 'Invalid request', '检查接口、请求参数与协议版本。', 'Check the endpoint, parameters and protocol version.'],
  internal_error: ['路由内部错误', 'Router internal error', '使用请求 ID 排查管理器与实例日志。', 'Use the request ID to inspect manager and instance logs.'],
}
export const failureCodes = Object.keys(failures)
export function failureText(code: string, lang: string) {
  const value = failures[code] || failures.internal_error
  return { title: value[lang === 'zh-CN' ? 0 : 1], advice: value[lang === 'zh-CN' ? 2 : 3] }
}
export function diagnosticValue(value: string, lang: string) {
  if (value === 'quota') return lang === 'zh-CN' ? '额度预留' : 'Quota reservation'
  const names: Record<string, string> = { authentication: '鉴权', admission: '请求准入', queue: '排队', routing: '实例选择', validation: '请求校验', preflight: '上下文预检', upstream: '上游响应', stream: '流传输', delivery: '客户端交付', upstream_usage: '上游报告', none: '未报告', exact: '精确计数', output_only: '仅检查输出上限', not_needed: '未触发精确计数', unavailable: '计数接口不可用' }
  return lang === 'zh-CN' ? names[value] || value : value
}
