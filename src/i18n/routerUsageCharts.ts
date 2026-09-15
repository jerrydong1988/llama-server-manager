export function getRouterUsageChartLabels(lang: string) {
  const zh = lang === 'zh-CN'
  return {
    trend: zh ? '调用与用量趋势' : 'Requests and token usage',
    trendHint: zh ? '按请求结束日期归集，与上方筛选条件一致。' : 'Grouped by completion date, using the filters above.',
    tokens: zh ? 'Token 用量' : 'Token usage',
    trendMetric: zh ? '趋势指标' : 'Trend metric',
    daily: zh ? '每日 · UTC' : 'Daily · UTC',
    sevenDays: zh ? '每 7 天汇总 · UTC' : '7-day totals · UTC',
    thirtyDays: zh ? '每 30 天汇总 · UTC' : '30-day totals · UTC',
    bucketsHint: zh ? '从所选开始日期分段，最后一段可能不足完整天数。' : 'Buckets start at the selected date; the final bucket may be shorter.',
    explore: zh ? '悬停或点击查看数值，也可用左右方向键切换日期。' : 'Hover or select a bar for values; use arrow keys to move between dates.',
    noRecorded: zh ? '此时段无已记录请求' : 'No recorded requests in this period',
    unreported: zh ? '未报告' : 'Not reported',
    noTokens: zh ? '暂无已报告 Token 用量；未知用量或计数接口不按零消耗绘制。' : 'No reported token usage. Unknown usage and counting calls are not plotted as zero consumption.',
    tokenHint: zh ? '仅绘制已报告计数；缺失值以「—」标记。缓存已包含在输入中，不重复叠加。' : 'Reported counts only; missing values are marked with a dash. Cached tokens are included in input, not added again.',
    results: zh ? '请求结果分布' : 'Request outcomes',
    successRate: zh ? '请求成功率' : 'Request success rate',
    resultHint: zh ? '成功率＝成功请求／全部已记录请求，包含拒绝、取消和未完整结束。' : 'Success rate = successful / all recorded requests, including rejections, cancellations and incomplete streams.',
    ranking: zh ? '用量排行' : 'Usage ranking',
    rankBy: zh ? '排行维度' : 'Rank by',
    rankMetric: zh ? '排行指标' : 'Ranking metric',
    rankHint: zh ? '占比以当前筛选范围的总量为分母；可点击条目筛选。' : 'Shares use the filtered total. Select an entry to filter.',
    top: zh ? '展示前 6 项；完整列表见下方汇总表。' : 'Showing the top 6; see the summary table below for all entries.',
    filter: zh ? '筛选' : 'Filter',
    partialHint: zh ? '已报告用量可能只包含部分计数，请结合完整用量覆盖率判断。' : 'Reported usage may include partial counts; check complete usage coverage.',
  }
}
