const fs = require('fs')
const path = require('path')

const source = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'store', 'downloadSlice.ts'),
  'utf8',
)

const start = source.indexOf('redownloadFile:')
const end = source.indexOf('moveQueueEntry:', start)
if (start < 0 || end < 0) {
  throw new Error('Unable to locate redownloadFile implementation')
}

const implementation = source.slice(start, end)
const awaitIndex = implementation.indexOf("await invoke('reset_download_for_redownload'")
const deleteIndex = implementation.indexOf('delete tasks[taskId]')
const queueIndex = implementation.indexOf('get().addToDownloadQueue')

if (!implementation.startsWith('redownloadFile: async')) {
  throw new Error('redownloadFile must be asynchronous')
}
if (awaitIndex < 0) {
  throw new Error('redownloadFile must await backend cleanup')
}
if (!implementation.includes('repoId: task.repoId')) {
  throw new Error('redownloadFile must pass repoId to backend cleanup')
}
if (deleteIndex < awaitIndex || queueIndex < awaitIndex) {
  throw new Error('task removal and requeue must happen after cleanup succeeds')
}

console.log('redownload sequencing regression passed')
