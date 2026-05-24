import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { Info } from 'lucide-react'

const advancedConfigSchema = z.object({
  flashAttn: z.enum(['on', 'off', 'auto']).default('auto'),
  moeCpuLayers: z.number().int().min(0).default(0),
  mlock: z.boolean().default(false),
  noMmap: z.boolean().default(false),
  numa: z.boolean().default(false),
  cacheTypeK: z.string().default(''),
  cacheTypeV: z.string().default(''),
  
  // 推测解码
  draftModelPath: z.string().optional(),
  draftGpuLayers: z.number().int().min(0).max(99).default(99),
  draftTokens: z.number().int().min(1).default(16),
  specDraftNMin: z.number().int().min(0).default(0),
  specType: z.string().default(''),
  
  // 服务器可靠性
  timeout: z.number().int().min(1).default(600),
  sleepIdle: z.number().int().default(-1),
  contextShift: z.boolean().default(false),
})

type AdvancedConfig = z.infer<typeof advancedConfigSchema>

const cacheTypes = [
  { value: '', label: '默认 (f16)' },
  { value: 'f32', label: 'f32' },
  { value: 'f16', label: 'f16' },
  { value: 'bf16', label: 'bf16' },
  { value: 'q8_0', label: 'q8_0' },
  { value: 'q4_0', label: 'q4_0' },
  { value: 'q4_1', label: 'q4_1' },
  { value: 'iq4_nl', label: 'iq4_nl' },
  { value: 'q5_0', label: 'q5_0' },
  { value: 'q5_1', label: 'q5_1' },
]

const specTypes = [
  { value: '', label: '自动' },
  { value: 'none', label: '关闭' },
  { value: 'draft-mtp', label: '草稿模型 MTP' },
  { value: 'draft-simple', label: '草稿模型 Simple' },
  { value: 'draft-eagle3', label: '草稿模型 Eagle3' },
  { value: 'ngram-cache', label: 'Ngram 缓存' },
  { value: 'ngram-simple', label: 'Ngram Simple' },
  { value: 'ngram-map-k', label: 'Ngram Map-K' },
  { value: 'ngram-map-k4v', label: 'Ngram Map-K4V' },
  { value: 'ngram-mod', label: 'Ngram Mod' },
]

const AdvancedConfig = () => {
  const { register, handleSubmit, formState: { errors } } = useForm<AdvancedConfig>({
    resolver: zodResolver(advancedConfigSchema),
    defaultValues: {
      flashAttn: 'auto',
      moeCpuLayers: 0,
      mlock: false,
      noMmap: false,
      numa: false,
      cacheTypeK: '',
      cacheTypeV: '',
      draftModelPath: '',
      draftGpuLayers: 99,
      draftTokens: 16,
      specDraftNMin: 0,
      specType: '',
      timeout: 600,
      sleepIdle: -1,
      contextShift: false,
    }
  })

  const onSubmit = (data: AdvancedConfig) => {
    console.log('高级配置', data)
  }

  return (
    <form onSubmit={handleSubmit(onSubmit)} className="space-y-8">
      {/* 内存与优化 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <h3 className="text-lg font-semibold mb-4">内存与优化配置</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          <div>
            <label className="block text-sm font-medium mb-2">
              Flash Attention
            </label>
            <select
              {...register('flashAttn')}
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            >
              <option value="auto">自动</option>
              <option value="on">开启</option>
              <option value="off">关闭</option>
            </select>
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              启用FlashAttention加速，需要GPU支持
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              MoE CPU 层数量
            </label>
            <input
              {...register('moeCpuLayers', { valueAsNumber: true })}
              type="number"
              min="0"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              显存不足时，将N层MoE层放在CPU运行
            </p>
          </div>

          <div className="flex items-start gap-2">
            <input
              {...register('mlock')}
              type="checkbox"
              id="mlock"
              className="w-4 h-4 mt-1 rounded border-gray-300"
            />
            <div>
              <label htmlFor="mlock" className="text-sm font-medium">
                内存锁定 (mlock)
              </label>
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                将模型锁定在内存中，防止被系统交换到磁盘
              </p>
            </div>
          </div>

          <div className="flex items-start gap-2">
            <input
              {...register('noMmap')}
              type="checkbox"
              id="noMmap"
              className="w-4 h-4 mt-1 rounded border-gray-300"
            />
            <div>
              <label htmlFor="noMmap" className="text-sm font-medium">
                禁用内存映射 (no-mmap)
              </label>
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                不使用内存映射加载模型，加载会变慢但更稳定
              </p>
            </div>
          </div>

          <div className="flex items-start gap-2">
            <input
              {...register('numa')}
              type="checkbox"
              id="numa"
              className="w-4 h-4 mt-1 rounded border-gray-300"
            />
            <div>
              <label htmlFor="numa" className="text-sm font-medium">
                NUMA 优化
              </label>
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                仅多路CPU服务器需要，普通PC开启无效果
              </p>
            </div>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              KV缓存类型 (K)
            </label>
            <select
              {...register('cacheTypeK')}
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            >
              {cacheTypes.map(type => (
                <option key={type.value} value={type.value}>{type.label}</option>
              ))}
            </select>
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              KV缓存的量化类型，越低占用显存越少，精度略有损失
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              KV缓存类型 (V)
            </label>
            <select
              {...register('cacheTypeV')}
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            >
              {cacheTypes.map(type => (
                <option key={type.value} value={type.value}>{type.label}</option>
              ))}
            </select>
          </div>
        </div>
      </div>

      {/* 推测解码 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <h3 className="text-lg font-semibold mb-4">推测解码配置</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          <div>
            <label className="block text-sm font-medium mb-2">
              草稿模型路径
            </label>
            <div className="flex gap-2">
              <input
                {...register('draftModelPath')}
                type="text"
                className="flex-1 px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                placeholder="选择草稿模型GGUF文件"
              />
              <button
                type="button"
                className="px-3 py-2 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg transition-colors"
              >
                <Info className="w-5 h-5" />
              </button>
            </div>
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              小模型作为草稿，可以大幅提升生成速度
            </p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              草稿模型 GPU 层数量
            </label>
            <input
              {...register('draftGpuLayers', { valueAsNumber: true })}
              type="number"
              min="0"
              max="99"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              最大草稿令牌数
            </label>
            <input
              {...register('draftTokens', { valueAsNumber: true })}
              type="number"
              min="1"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              最小草稿令牌数
            </label>
            <input
              {...register('specDraftNMin', { valueAsNumber: true })}
              type="number"
              min="0"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              推测解码类型
            </label>
            <select
              {...register('specType')}
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            >
              {specTypes.map(type => (
                <option key={type.value} value={type.value}>{type.label}</option>
              ))}
            </select>
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              选择推测解码算法，多个可以用逗号分隔
            </p>
          </div>
        </div>
      </div>

      {/* 服务器可靠性 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <h3 className="text-lg font-semibold mb-4">服务器可靠性配置</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          <div>
            <label className="block text-sm font-medium mb-2">
              请求超时时间 (秒)
            </label>
            <input
              {...register('timeout', { valueAsNumber: true })}
              type="number"
              min="1"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              空闲休眠时间 (秒)
            </label>
            <input
              {...register('sleepIdle', { valueAsNumber: true })}
              type="number"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
              空闲N秒后自动卸载模型释放显存，-1表示禁用
            </p>
          </div>

          <div className="flex items-start gap-2">
            <input
              {...register('contextShift')}
              type="checkbox"
              id="contextShift"
              className="w-4 h-4 mt-1 rounded border-gray-300"
            />
            <div>
              <label htmlFor="contextShift" className="text-sm font-medium">
                上下文偏移 (context shift)
              </label>
              <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                上下文满了之后自动滚动，实现无限生成
              </p>
            </div>
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

export default AdvancedConfig
