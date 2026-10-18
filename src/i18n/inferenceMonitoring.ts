import type { InferenceRate } from '../store/types'

export function getInferenceLabels(lang: string) {
  const zh = lang === 'zh-CN'
  return {
    prefill: zh ? 'Prefill 输入速度' : 'Prefill input speed',
    decode: zh ? 'Decode 生成速度' : 'Decode output speed',
    noTrend: zh ? '暂无近期速度采样；阶段平均不计入此图' : 'No recent rate samples; phase averages are excluded',
    noTasks: zh ? '暂无可观测的运行中请求' : 'No observable active requests',
    unknownPhase: zh ? '等待阶段数据' : 'Waiting for phase data',
    processed: zh ? '已处理' : 'Processed',
    cached: zh ? '缓存复用' : 'Cached',
    generated: zh ? '已生成' : 'Generated',
    progress: zh ? '输入进度' : 'Input progress',
    phaseAverage: zh ? '阶段平均' : 'Phase average',
    prefillAverage: zh ? 'Prefill 阶段平均' : 'Prefill phase average',
    decodeAverage: zh ? 'Decode 阶段平均' : 'Decode phase average',
    sessionDecodeAverage: zh ? '会话平均 Decode' : 'Session average Decode',
    prefillTime: zh ? '最近请求 Prefill 平均耗时' : 'Recent requests: avg prefill time',
  }
}

export function inferenceRateDetail(rate: InferenceRate, lang: string): string {
  const zh = lang === 'zh-CN'
  if (rate.state === 'idle') return zh ? '当前无此阶段计算' : 'No computation in this phase'
  if (rate.state === 'sampling') return zh ? '采样中，等待连续观测' : 'Collecting consecutive samples'
  if (rate.state === 'stale') return zh ? '等待更新，旧数据已过期' : 'Waiting for fresh data'
  if (rate.state === 'unavailable') return zh ? '数据不可用' : 'Data unavailable'
  const source = rate.source === 'log' ? (zh ? '任务日志' : 'Task log') : rate.source === 'slots' ? '/slots' : (zh ? '多来源' : 'Multiple sources')
  const window = ((rate.windowMs || 3000) / 1000).toFixed(1)
  return rate.state === 'partial'
    ? (zh ? `部分数据 · ${rate.covered}/${rate.expected} 完整覆盖` : `Partial data · ${rate.covered}/${rate.expected} complete`)
    : (zh ? `近期 ${window}s 观测 · ${source}` : `Recent ${window}s observation · ${source}`)
}
