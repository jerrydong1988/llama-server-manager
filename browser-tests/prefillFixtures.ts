import type {
  CompletedPrefill,
  InferenceRequestSummary,
  InstancePrefillInterval,
  RequestPrefillWindow,
} from '../src/store/types'

// Numeric timings only, from the short-prompt session that originally showed no live Prefill points.
const captured = [
  { id: 570, ago: 120085, tokens: 506, ms: 3337.09, output: 1550, decode: 29.62 },
  { id: 1326, ago: 65558, tokens: 310, ms: 1672.64, output: 1147, decode: 32.26 },
  { id: 1835, ago: 14327, tokens: 159, ms: 1430.94, output: 984, decode: 33.76 },
]

export function replayRequests(now: number, session: string): InferenceRequestSummary[] {
  return [...captured].reverse().map((r) => ({
    session_id: session,
    task_id: r.id,
    slot_id: 0,
    completed_at: now - r.ago,
    source: 'log',
    api_format: null,
    model: null,
    target_instance_id: null,
    http_status: null,
    error_text: null,
    prompt_tokens: r.tokens,
    prompt_time_ms: r.ms,
    prompt_tps: (r.tokens * 1000) / r.ms,
    generated_tokens: r.output,
    generation_time_ms: null,
    generation_tps: r.decode,
    total_tokens: null,
    total_time_ms: null,
    spec_accept_rate: null,
    spec_accepted: null,
    spec_generated: null,
    spec_gen_time_ms: null,
  }))
}

export function fixturePrefill(now: number, mode: string | null): RequestPrefillWindow {
  let points: CompletedPrefill[]
  if (mode === 'gufo') {
    points = [
      {
        ts: now - 6000,
        request: 'r1292',
        tokens: 1320,
        elapsedMs: (1320 * 1000) / 1069.5,
        count: 1,
        estimated: true,
      },
    ]
  } else if (mode === 'replay') {
    points = captured.map((r) => ({
      ts: now - r.ago,
      request: '#' + r.id,
      tokens: r.tokens,
      elapsedMs: r.ms,
      count: 1,
      estimated: false,
    }))
  } else {
    points = Array.from({ length: 5 }, (_, i) => ({
      ts: now - (i + 1) * 18000,
      request: '#' + (784 - i),
      tokens: 1800 + i * 64,
      elapsedMs: ((1800 + i * 64) * 1000) / (510 + i * 5),
      count: 1,
      estimated: false,
    })).reverse()
  }
  return { asOf: now, points, latest: points[points.length - 1], bucketed: false }
}

export function fixtureIntervals(ts: number, mode: string | null, index: number): InstancePrefillInterval[] {
  if (mode === 'gufo')
    return [
      {
        ts,
        wallMs: 1000,
        tokens: index % 5 === 0 ? 1536 : 1024,
        rate: index % 5 === 0 ? 1536 : 1024,
        basis: 'wall_time',
      },
    ]
  if (mode === 'llama-window')
    return [{ ts, wallMs: 5000, tokens: 159, rate: 159 / 1.43094, basis: 'engine_time' }]
  return []
}
