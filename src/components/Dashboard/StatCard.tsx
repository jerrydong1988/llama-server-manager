interface Props {
  icon: React.ReactNode
  label: string
  value: number | string
  color: 'emerald' | 'blue' | 'purple' | 'slate'
}

const colorMap = {
  emerald: 'bg-emerald-50 dark:bg-emerald-900/20 text-emerald-600 dark:text-emerald-400',
  blue: 'bg-blue-50 dark:bg-blue-900/20 text-blue-600 dark:text-blue-400',
  purple: 'bg-purple-50 dark:bg-purple-900/20 text-purple-600 dark:text-purple-400',
  slate: 'bg-ui-inset text-ui-muted',
}

export default function StatCard({ icon, label, value, color }: Props) {
  return (
    <div className="bg-ui-surface rounded-xl border border-ui-line p-5 hover:shadow-md transition-shadow">
      <div className="flex items-center justify-between">
        <div>
          <p className="text-sm text-ui-muted">{label}</p>
          <p className="text-3xl font-bold text-ui-ink mt-1">{value}</p>
        </div>
        <div className={`p-3 rounded-lg ${colorMap[color]}`}>{icon}</div>
      </div>
    </div>
  )
}
