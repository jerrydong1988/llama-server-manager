const fs = require('fs')
const path = require('path')

const source = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'store', 'instanceSlice.ts'),
  'utf8',
)

if (!source.includes('let configSaveQueue: Promise<void> = Promise.resolve()')) {
  throw new Error('configuration saves must share a serialization queue')
}
if (!source.includes('configSaveQueue.catch(() => {}).then(')) {
  throw new Error('each configuration save must wait for the previous save')
}
if (!source.includes('configSaveQueue = operation')) {
  throw new Error('the latest configuration save must become the queue tail')
}

console.log('config save sequencing regression passed')
