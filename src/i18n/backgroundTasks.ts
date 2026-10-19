import type { Lang } from './index'

const en = {
  shortcuts: 'Quick actions', title: 'Background tasks',
  description: 'Shared CLI / MCP task history. Up to 200 retained records; refreshed automatically while open.',
  scope: 'These are management operations, not model generation or file-transfer progress. Completing a start/download command does not mean the model is ready or the download is finished.',
  search: 'Search operation or task ID', filter: 'Task status', all: 'All states',
  refresh: 'Refresh tasks', loading: 'Loading…', stale: 'Refresh failed. Previously loaded data may be out of date.',
  empty: 'No background tasks yet.', noMatches: 'No tasks match these filters.', select: 'Select a task to inspect its result.',
  id: 'Task ID', operation: 'Operation', created: 'Created', updated: 'Last updated', result: 'Execution result',
  noResult: 'No result recorded.', omitted: 'The result snapshot was too large to retain. Query the corresponding resource for its current state.',
  error: 'Error / interruption', retryable: 'Retryable', yes: 'Yes', no: 'No',
  cancel: 'Cancel queued task', cancelling: 'Cancelling…',
  cancelHint: 'Only queued tasks can be cancelled. Running operations must finish; use the resource’s stop or pause control when needed.',
  cancelled: 'Task cancelled before execution.', alreadyFinished: 'The task finished before cancellation. Its recorded final state is shown below.',
  runningRace: 'The task has started and cannot be cancelled here. Use the resource’s stop or pause control.',
  interrupted: 'The host exited before recording completion. Inspect the resource state before retrying; this operation may already have taken effect.',
  resource: 'Open resource', records: 'records',
  states: { queued: 'Queued', running: 'Running', succeeded: 'Succeeded', failed: 'Failed', cancelled: 'Cancelled', interrupted: 'Interrupted' },
}

const zh: typeof en = {
  shortcuts: '常用操作', title: '后台任务',
  description: '与 CLI / MCP 共用任务记录。最多保留 200 条，打开时自动刷新。',
  scope: '这里记录管理操作，不代表模型生成或文件传输进度。启动或添加下载命令成功，不等于模型就绪或下载完成。',
  search: '搜索操作名称或任务 ID', filter: '任务状态', all: '全部状态',
  refresh: '刷新任务', loading: '加载中…', stale: '刷新失败，已显示的数据可能已过期。',
  empty: '暂无后台任务。', noMatches: '没有符合筛选条件的任务。', select: '选择任务查看执行结果。',
  id: '任务 ID', operation: '操作', created: '创建时间', updated: '最后更新', result: '执行结果',
  noResult: '尚无结果记录。', omitted: '结果快照过大，未完整保存。请查询对应资源的当前状态。',
  error: '错误 / 中断原因', retryable: '可重试', yes: '是', no: '否',
  cancel: '取消排队任务', cancelling: '正在取消…',
  cancelHint: '仅可取消排队任务。执行中的操作需等待完成；如需停止或暂停，请使用对应资源的控制功能。',
  cancelled: '任务已在执行前取消。', alreadyFinished: '任务已先于取消请求结束，下面显示实际终态。',
  runningRace: '任务已开始执行，无法在此取消。请使用对应资源的停止或暂停功能。',
  interrupted: '宿主退出前未记录完成状态。操作可能已生效，重试前请先检查资源状态。',
  resource: '打开对应资源', records: '条记录',
  states: { queued: '排队中', running: '执行中', succeeded: '成功', failed: '失败', cancelled: '已取消', interrupted: '已中断' },
}

export const getBackgroundTaskLabels = (lang: Lang) => lang === 'zh-CN' ? zh : en
