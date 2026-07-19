import { Badge, SectionHeader, Surface } from '../ui'

export type ConfigDirectoryGroup = {
  id: string
  title: string
  changedCount: number
  emittedCount: number
  children?: ConfigDirectoryGroup[]
}

export function ConfigDirectory({ title, groups, changedLabel, emittedLabel }: { title: string; groups: ConfigDirectoryGroup[]; changedLabel: string; emittedLabel: string }) {
  const scrollToSection = (id: string) => {
    document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }
  return (
    <Surface as="aside" className="h-fit p-4 xl:sticky xl:top-4">
      <SectionHeader title={title} />
      <nav className="mt-4 space-y-1">
        {groups.map(group => (
          <div key={group.id}>
            <button
              type="button"
              onClick={() => scrollToSection(group.id)}
              className="flex w-full items-center justify-between gap-3 rounded-lg px-3 py-2 text-left text-sm text-slate-700 transition hover:bg-slate-100 dark:text-slate-200 dark:hover:bg-slate-800"
            >
              <span className="min-w-0 truncate">{group.title}</span>
              <span className="flex shrink-0 items-center gap-1">
                {group.changedCount > 0 && <Badge tone="slate" className="px-1.5 py-0.5 text-[10px]">{changedLabel} {group.changedCount}</Badge>}
                {group.emittedCount > 0 && <Badge tone="blue" className="px-1.5 py-0.5 text-[10px]">{emittedLabel} {group.emittedCount}</Badge>}
              </span>
            </button>
            {group.children && (
              <div className="mt-1 space-y-1 border-l border-slate-200 pl-3 dark:border-slate-800">
                {group.children.map(child => (
                  <button
                    key={child.id}
                    type="button"
                    onClick={() => scrollToSection(child.id)}
                    className="flex w-full items-center justify-between gap-2 rounded-lg px-2 py-1.5 text-left text-xs text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 dark:hover:bg-slate-800 dark:hover:text-slate-200"
                  >
                    <span className="min-w-0 truncate">{child.title}</span>
                    <span className="flex shrink-0 items-center gap-1 text-[10px] text-slate-500">
                      {child.changedCount > 0 && <span>{changedLabel} {child.changedCount}</span>}
                      {child.emittedCount > 0 && <span>{emittedLabel} {child.emittedCount}</span>}
                    </span>
                  </button>
                ))}
              </div>
            )}
          </div>
        ))}
      </nav>
    </Surface>
  )
}
