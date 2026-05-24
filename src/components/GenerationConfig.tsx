import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import { ChevronDown, ChevronUp, Info } from 'lucide-react'
import { useState } from 'react'

const generationConfigSchema = z.object({
  nPredict: z.number().int().default(-1),
  ignoreEos: z.boolean().default(false),
  jsonSchema: z.string().optional(),
  temp: z.number().min(0).max(2).default(0.8),
  topK: z.number().int().min(0).default(40),
  topP: z.number().min(0).max(1).default(0.9),
  repeatPenalty: z.number().min(0).max(2).default(1.1),
  seed: z.number().int().default(-1),
  minP: z.number().min(0).max(1).default(0.05),
  presencePenalty: z.number().min(0).max(2).default(0),
  frequencyPenalty: z.number().min(0).max(2).default(0),
  repeatLastN: z.number().int().default(64),
  
  // 高级采样
  mirostat: z.number().int().min(0).max(2).default(0),
  mirostatLr: z.number().min(0).max(1).default(0.1),
  mirostatEnt: z.number().min(0).max(10).default(5.0),
  xtcProbability: z.number().min(0).max(1).default(0),
  xtcThreshold: z.number().min(0).max(1).default(0.1),
  dynatempRange: z.number().min(0).max(10).default(0),
  dynatempExp: z.number().min(0).max(10).default(1.0),
  typicalP: z.number().min(0).max(1).default(1.0),
  dryMultiplier: z.number().min(0).max(10).default(0),
  dryBase: z.number().min(0).max(10).default(1.75),
  dryAllowedLength: z.number().int().min(0).default(2),
  dryPenaltyLastN: z.number().int().default(-1),
  drySequenceBreaker: z.string().optional(),
})

type GenerationConfig = z.infer<typeof generationConfigSchema>

const GenerationConfig = () => {
  const [showAdvanced, setShowAdvanced] = useState(false)
  const { register, handleSubmit, formState: { errors } } = useForm<GenerationConfig>({
    resolver: zodResolver(generationConfigSchema),
    defaultValues: {
      nPredict: -1,
      ignoreEos: false,
      jsonSchema: '',
      temp: 0.8,
      topK: 40,
      topP: 0.9,
      repeatPenalty: 1.1,
      seed: -1,
      minP: 0.05,
      presencePenalty: 0,
      frequencyPenalty: 0,
      repeatLastN: 64,
      mirostat: 0,
      mirostatLr: 0.1,
      mirostatEnt: 5.0,
      xtcProbability: 0,
      xtcThreshold: 0.1,
      dynatempRange: 0,
      dynatempExp: 1.0,
      typicalP: 1.0,
      dryMultiplier: 0,
      dryBase: 1.75,
      dryAllowedLength: 2,
      dryPenaltyLastN: -1,
      drySequenceBreaker: '',
    }
  })

  const onSubmit = (data: GenerationConfig) => {
    console.log('生成配置', data)
  }

  return (
    <form onSubmit={handleSubmit(onSubmit)} className="space-y-8">
      {/* 基础生成参数 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <h3 className="text-lg font-semibold mb-4">基础生成参数</h3>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
          <div>
            <label className="block text-sm font-medium mb-2">
              生成令牌数
            </label>
            <input
              {...register('nPredict', { valueAsNumber: true })}
              type="number"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
              placeholder="-1 = 无限生成"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">最大生成的token数量，-1表示不限</p>
          </div>

          <div className="flex items-end gap-2">
            <div className="flex items-center gap-2">
              <input
                {...register('ignoreEos')}
                type="checkbox"
                id="ignoreEos"
                className="w-4 h-4 rounded border-gray-300"
              />
              <label htmlFor="ignoreEos" className="text-sm font-medium">
                忽略结束标记
              </label>
            </div>
            <Info className="w-4 h-4 text-gray-400" title="防止模型提前停止生成" />
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              JSON 约束
            </label>
            <input
              {...register('jsonSchema')}
              type="text"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
              placeholder="JSON Schema 字符串"
            />
          </div>
        </div>
      </div>

      {/* 基础采样参数 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <h3 className="text-lg font-semibold mb-4">基础采样参数</h3>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
          <div>
            <label className="block text-sm font-medium mb-2">
              温度 (temp)
            </label>
            <input
              {...register('temp', { valueAsNumber: true })}
              type="number"
              step="0.1"
              min="0"
              max="2"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">越低越确定，越高越有创造性</p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              Top-K
            </label>
            <input
              {...register('topK', { valueAsNumber: true })}
              type="number"
              min="0"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">仅保留概率最高的K个token</p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              Top-P
            </label>
            <input
              {...register('topP', { valueAsNumber: true })}
              type="number"
              step="0.1"
              min="0"
              max="1"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">核采样，累计概率和为P的token</p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              重复惩罚
            </label>
            <input
              {...register('repeatPenalty', { valueAsNumber: true })}
              type="number"
              step="0.1"
              min="0"
              max="2"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">越高越不容易出现重复内容</p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              随机种子
            </label>
            <input
              {...register('seed', { valueAsNumber: true })}
              type="number"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">-1表示随机，固定值可复现结果</p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              Min-P
            </label>
            <input
              {...register('minP', { valueAsNumber: true })}
              type="number"
              step="0.05"
              min="0"
              max="1"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">过滤概率低于此值的token</p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              存在惩罚
            </label>
            <input
              {...register('presencePenalty', { valueAsNumber: true })}
              type="number"
              step="0.1"
              min="0"
              max="2"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">降低重复讨论相同话题的概率</p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              频率惩罚
            </label>
            <input
              {...register('frequencyPenalty', { valueAsNumber: true })}
              type="number"
              step="0.1"
              min="0"
              max="2"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">降低高频词重复出现的概率</p>
          </div>

          <div>
            <label className="block text-sm font-medium mb-2">
              惩罚窗口
            </label>
            <input
              {...register('repeatLastN', { valueAsNumber: true })}
              type="number"
              className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
            />
            <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">-1表示使用整个上下文窗口</p>
          </div>
        </div>
      </div>

      {/* 高级采样参数 */}
      <div className="bg-white dark:bg-gray-800 rounded-lg p-6 shadow-sm">
        <button
          type="button"
          onClick={() => setShowAdvanced(!showAdvanced)}
          className="flex items-center justify-between w-full text-left mb-4"
        >
          <h3 className="text-lg font-semibold">高级采样参数</h3>
          {showAdvanced ? <ChevronUp className="w-5 h-5" /> : <ChevronDown className="w-5 h-5" />}
        </button>

        {showAdvanced && (
          <div className="space-y-6">
            {/* Mirostat */}
            <div>
              <h4 className="text-md font-medium mb-3 text-gray-700 dark:text-gray-300">Mirostat 自适应采样</h4>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                <div>
                  <label className="block text-sm font-medium mb-2">模式</label>
                  <select
                    {...register('mirostat', { valueAsNumber: true })}
                    className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                  >
                    <option value="0">关闭</option>
                    <option value="1">Mirostat v1</option>
                    <option value="2">Mirostat v2 (推荐)</option>
                  </select>
                </div>
                <div>
                  <label className="block text-sm font-medium mb-2">学习率</label>
                  <input
                    {...register('mirostatLr', { valueAsNumber: true })}
                    type="number"
                    step="0.01"
                    min="0"
                    max="1"
                    className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium mb-2">目标熵</label>
                  <input
                    {...register('mirostatEnt', { valueAsNumber: true })}
                    type="number"
                    step="0.1"
                    min="0"
                    max="10"
                    className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                  />
                </div>
              </div>
            </div>

            {/* XTC */}
            <div>
              <h4 className="text-md font-medium mb-3 text-gray-700 dark:text-gray-300">XTC 采样</h4>
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                <div>
                  <label className="block text-sm font-medium mb-2">概率</label>
                  <input
                    {...register('xtcProbability', { valueAsNumber: true })}
                    type="number"
                    step="0.05"
                    min="0"
                    max="1"
                    className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium mb-2">阈值</label>
                  <input
                    {...register('xtcThreshold', { valueAsNumber: true })}
                    type="number"
                    step="0.05"
                    min="0"
                    max="1"
                    className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                  />
                </div>
              </div>
            </div>

            {/* 动态温度 */}
            <div>
              <h4 className="text-md font-medium mb-3 text-gray-700 dark:text-gray-300">动态温度</h4>
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                <div>
                  <label className="block text-sm font-medium mb-2">范围</label>
                  <input
                    {...register('dynatempRange', { valueAsNumber: true })}
                    type="number"
                    step="0.1"
                    min="0"
                    max="10"
                    className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium mb-2">指数</label>
                  <input
                    {...register('dynatempExp', { valueAsNumber: true })}
                    type="number"
                    step="0.1"
                    min="0"
                    max="10"
                    className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                  />
                </div>
              </div>
            </div>

            {/* DRY采样 */}
            <div>
              <h4 className="text-md font-medium mb-3 text-gray-700 dark:text-gray-300">DRY 重复惩罚</h4>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                <div>
                  <label className="block text-sm font-medium mb-2">惩罚强度</label>
                  <input
                    {...register('dryMultiplier', { valueAsNumber: true })}
                    type="number"
                    step="0.1"
                    min="0"
                    max="10"
                    className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium mb-2">基数</label>
                  <input
                    {...register('dryBase', { valueAsNumber: true })}
                    type="number"
                    step="0.1"
                    min="0"
                    max="10"
                    className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium mb-2">允许重复长度</label>
                  <input
                    {...register('dryAllowedLength', { valueAsNumber: true })}
                    type="number"
                    min="0"
                    className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium mb-2">惩罚窗口</label>
                  <input
                    {...register('dryPenaltyLastN', { valueAsNumber: true })}
                    type="number"
                    className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium mb-2">序列分隔符</label>
                  <input
                    {...register('drySequenceBreaker')}
                    type="text"
                    className="w-full px-3 py-2 border dark:border-gray-700 rounded-lg bg-white dark:bg-gray-800"
                    placeholder="例如 \n"
                  />
                </div>
              </div>
            </div>
          </div>
        )}
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

export default GenerationConfig
