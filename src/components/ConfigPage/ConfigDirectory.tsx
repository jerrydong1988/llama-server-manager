import { SelectInput } from '../ui'

export type ConfigDirectoryGroup = {
  id: string
  title: string
  children?: ConfigDirectoryGroup[]
}

export function ConfigDirectory({ title, groups }: { title: string; groups: ConfigDirectoryGroup[] }) {
  function jump(id: string) {
    if (!id) return
    const parent = groups.find(group => group.children?.some(child => child.id === id))
    const expand = (sectionId: string) => {
      const section = document.getElementById(sectionId)
      section?.querySelector<HTMLButtonElement>(':scope > button[aria-expanded="false"], :scope > div > button[aria-expanded="false"]')?.click()
      return section
    }
    if (parent) expand(parent.id)
    requestAnimationFrame(() => {
      const section = expand(id)
      section?.scrollIntoView({ block: 'start' })
      section?.querySelector<HTMLElement>('button, input, select')?.focus({ preventScroll: true })
    })
  }
  return <SelectInput aria-label={title} value="" onChange={event => jump(event.target.value)} className="config-jump">
    <option value="">{title}</option>
    {groups.map(group => group.children ? <optgroup key={group.id} label={group.title}>
      <option value={group.id}>{group.title}</option>
      {group.children.map(child => <option key={child.id} value={child.id}>{child.title}</option>)}
    </optgroup> : <option key={group.id} value={group.id}>{group.title}</option>)}
  </SelectInput>
}
