import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { Info, Cpu } from 'lucide-react'

const performanceConfigSchema = z.object({
  ctxSize: z.number().int().min(0).default(4096),
  ctxSizeAuto: z.boolean().default(false),
  gpuLayers: z.number().int().min(0).max(99).default(99),
  threads: z.number().int().min(1).default(8),
  batchSize: z.number().int().min(1).default(2048),
  ubatchSize: z.number().int().min(1).default(512),
  parallel: z.number().int().min(1).default(1),
  contBatching: z.boolean().default(false),
  cachePrompt: z.boolean().default(true),
  threadsBatch: z.number().int().min(1).default(8),
})

type PerformanceConfig = z.infer<typeof performanceConfigSchema>

const PerformanceConfig = () => {
  const { register, handleSubmit, watch, formState: { errors } } = useForm<PerformanceConfig>({
    resolver: zodResolver(performanceConfigSchema),
    defaultValues: {
      ctxSize: 4096,
      ctxSizeAuto: false,
      gpuLayers: 99,
      threads: 8,
      batchSize: 2048,
      ubatchSize: 512,
      parallel: 1,
      contBatching: false,
      cachePrompt: true,
      threadsBatch: 8,
    }
  })

  const ctxSizeAuto = watch('ctxSizeAuto')
  const cpuCount = navigator.hardwareConcurrency || 8

  const onSubmit = (data: PerformanceConfig) => {
    console.log('性能配置', data)
  }

  return (
    <form onSubmit={handleSubmit(onSubmit)} className="space-y-8">
      {/* 核心性能 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <h3 className="text-lg font-semibold mb-4">核心性能配置</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          <div>
            <label className="block text-sm font-medium mb-2">
              上下文大小 (ctx_size)
            </label>
            <div className="flex items-center gap-3">
              <input
                {...register('ctxSize', { valueAsNumber: true })}
                type="number"
                min="0"
                step="1024"
                disabled={ctxSizeAuto}
                className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800 disabled:opacity-50"
              />
              <div className="flex items-center gap-2">
                <input
                  {...register('ctxSizeAuto')}
                  type="checkbox"
                  id="ctxSizeAuto"
                  className="w-4 h-4 rounded border-gray-300"
                />
                <label htmlFor="ctxSizeAuto" className="text-sm font-medium">
                  自动适配模型
                </label>
              </div>
            </div>
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              模型的最大上下文长度，勾选后自动使用模型的原生上下文大小
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              GPU 层数量 (gpu_layers)
            </label>
            <input
              {...register('gpuLayers', { valueAsNumber: true })}
              type="number"
              min="0"
              max="99"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              卸载到GPU运行的模型层数，99表示全部卸载到GPU
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              CPU 线程数 (threads)
            </label>
            <div className="flex gap-2">
              <input
                {...register('threads', { valueAsNumber: true })}
                type="number"
                min="1"
                max={cpuCount}
                className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
              />
              <button
                type="button"
                onClick={() => register('threads').onChange({ target: { value: cpuCount } })}
                className="px-3 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg transition-colors text-sm"
              >
                最大
              </button>
            </div>
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              你的CPU有 {cpuCount} 个逻辑核心，推荐设置为物理核心数
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              批处理大小 (batch_size)
            </label>
            <input
              {...register('batchSize', { valueAsNumber: true })}
              type="number"
              min="1"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              提示处理的最大批处理大小，越大处理速度越快，但占用显存越多
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              物理批大小 (ubatch_size)
            </label>
            <input
              {...register('ubatchSize', { valueAsNumber: true })}
              type="number"
              min="1"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              实际GPU计算的批处理大小，降低此值可减少显存占用
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              并行序列数 (parallel)
            </label>
            <input
              {...register('parallel', { valueAsNumber: true })}
              type="number"
              min="1"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              同时并行处理的序列数量，多用户场景可适当提高
            </p>
          </div>
        </div>
      </div>

      {/* 高级吞吐量 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <h3 className="text-lg font-semibold mb-4">高级吞吐量配置</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          <div className="flex items-center gap-2">
            <input
              {...register('contBatching')}
              type="checkbox"
              id="contBatching"
              className="w-4 h-4 rounded border-gray-300"
            />
            <div>
              <label htmlFor="contBatching" className="text-sm font-medium">
                持续批处理 (continuous batching)
              </label>
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                提高多并发场景下的吞吐量，需要显存足够
              </p>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <input
              {...register('cachePrompt')}
              type="checkbox"
              id="cachePrompt"
              className="w-4 h-4 rounded border-gray-300"
              defaultChecked
            />
            <div>
              <label htmlFor="cachePrompt" className="text-sm font-medium">
                启用提示缓存
              </label>
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                缓存重复的提示内容，提高重复请求的处理速度
              </p>
            </div>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              批处理线程数 (threads_batch)
            </label>
            <div className="flex gap-2">
              <input
                {...register('threadsBatch', { valueAsNumber: true })}
                type="number"
                min="1"
                max={cpuCount}
                className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
              />
              <button
                type="button"
                onClick={() => register('threadsBatch').onChange({ target: { value: cpuCount } })}
                className="px-3 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg transition-colors text-sm"
              >
                最大
              </button>
            </div>
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              提示批处理时使用的线程数，默认和threads相同
            </p>
          </div>
        </div>
      </div>

      <div className="flex justify-end">
        <button
          type="submit"
          className="px-6 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-lg transition-colors"
        >
          保存配置
        </button>
      </div>
    </form>
  )
}

export default PerformanceConfig
