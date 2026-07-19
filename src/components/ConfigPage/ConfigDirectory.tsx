import { SectionHeader, Surface } from '../ui'

export type ConfigDirectoryGroup = {
  id: string
  title: string
  children?: ConfigDirectoryGroup[]
}

export function ConfigDirectory({ title, groups }: { title: string; groups: ConfigDirectoryGroup[] }) {
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
              className="flex w-full items-center rounded-lg px-3 py-2 text-left text-sm text-slate-700 transition hover:bg-slate-100 dark:text-slate-200 dark:hover:bg-slate-800"
            >
              <span>{group.title}</span>
            </button>
            {group.children && (
              <div className="mt-1 space-y-1 border-l border-slate-200 pl-3 dark:border-slate-800">
                {group.children.map(child => (
                  <button
                    key={child.id}
                    type="button"
                    onClick={() => scrollToSection(child.id)}
                    className="flex w-full items-center rounded-lg px-2 py-1.5 text-left text-xs text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 dark:hover:bg-slate-800 dark:hover:text-slate-200"
                  >
                    <span>{child.title}</span>
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
