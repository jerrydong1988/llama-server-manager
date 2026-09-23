import { useState } from 'react'
import { useAppStore } from '../../store'
import { useI18n } from '../../i18n'
import { getGuideTourCopy } from '../../i18n/guideTourCopy'
import { Button, SelectInput } from '../ui'
import { useGuideTourStore } from './guideTourStore'

export function GuideInstancePicker({ alwaysExpanded }: { alwaysExpanded: boolean }) {
  const { lang } = useI18n()
  const copy = getGuideTourCopy(lang)
  const { session, configPending, selectInstance } = useGuideTourStore()
  const instances = useAppStore(state => state.instances)
  const instance = instances.find(item => item.id === session?.instanceId)
  const [switching, setSwitching] = useState(false)
  if (!alwaysExpanded && instance && !switching) {
    return (
      <div className="flex min-w-0 items-center gap-2 text-xs" data-guide-instance>
        <span className="min-w-0 truncate text-slate-600 dark:text-slate-300" title={instance.name}>{copy.selectInstance}: {instance.name}</span>
        <Button size="sm" variant="subtle" disabled={configPending} onClick={() => setSwitching(true)}>{copy.switchInstance}</Button>
      </div>
    )
  }
  return (
    <div className="flex min-w-0 items-center gap-2">
      <label className="min-w-0 flex-1 text-xs text-slate-500 dark:text-slate-400">
        <span className="sr-only">{copy.selectInstance}</span>
        <SelectInput autoFocus={switching} className="w-full min-w-0" disabled={configPending} value={instance?.id ?? ''} onChange={event => { selectInstance(event.target.value); setSwitching(false) }}>
          <option value="" disabled>{copy.chooseInstance}</option>
          {instances.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
        </SelectInput>
      </label>
      {switching && <Button size="sm" variant="subtle" onClick={() => setSwitching(false)}>{copy.cancelSwitch}</Button>}
    </div>
  )
}
