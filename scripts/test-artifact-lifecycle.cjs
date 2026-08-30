const assert = require('node:assert')
const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const read = relative => fs.readFileSync(path.join(root, relative), 'utf8')

const modelRepo = read('src/components/ModelRepo.tsx')
const deleteHandler = modelRepo.slice(
  modelRepo.indexOf('const handleDeleteFile'),
  modelRepo.indexOf('const renderNode'),
)
assert.ok(deleteHandler.includes("invoke<ModelDeletionPreview>('preview_model_deletion'"), 'model deletion must preview its physical artifact set')
assert.ok(deleteHandler.indexOf('preview_model_deletion') < deleteHandler.indexOf('confirm('), 'artifact preview must precede user confirmation')
assert.ok(deleteHandler.indexOf('confirm(') < deleteHandler.indexOf('deleteModelFile(path)'), 'deletion must follow explicit confirmation')
assert.ok(deleteHandler.includes('preview.artifactCount') && deleteHandler.includes('preview.totalBytes'), 'confirmation must disclose file count and total bytes')

const coreSlice = read('src/store/coreSlice.ts')
const storeDelete = coreSlice.slice(
  coreSlice.indexOf('deleteModelFile: async'),
  coreSlice.indexOf('openModelFolder: async'),
)
assert.ok(storeDelete.includes("invoke<ModelDeletionResult>('delete_model_file'"), 'store must consume the backend deletion result')
assert.ok(storeDelete.includes('result.artifactPaths.some'), 'store must remove every physical shard returned by the backend')

const scanner = read('src-tauri/src/commands/scanner.rs')
assert.ok(scanner.includes('resolve_model_deletion_artifacts'), 'backend must resolve a complete artifact set')
assert.ok(scanner.includes('ModelArtifactError::Incomplete'), 'incomplete shard groups must fail closed')
assert.ok(scanner.includes('Rebuild the complete artifact set at execution time'), 'execution must not trust a stale preview')

const security = read('src-tauri/src/security.rs')
const revoke = security.slice(
  security.indexOf('pub async fn revoke_authorized_directory'),
  security.indexOf('pub fn initialize_path_authority'),
)
assert.ok(revoke.indexOf('persist_authority(&updated)?') < revoke.indexOf('*authority = updated'), 'authorization revocation must persist before publishing memory state')

const external = read('src-tauri/src/external_artifacts.rs')
assert.ok(external.includes('ownership: "operator".to_string()'), 'external paths must remain operator-owned')
assert.ok(!external.includes('std::fs::remove_file') && !external.includes('std::fs::remove_dir'), 'external artifact inventory must stay read-only')

const storage = read('src-tauri/src/storage_maintenance.rs')
assert.ok(storage.includes('Unknown fixed storage-maintenance group'), 'storage cleanup must reject arbitrary group identifiers')
assert.ok(storage.includes('validated_path_size(path, root)?'), 'storage cleanup must revalidate containment and links immediately before deletion')
assert.ok(storage.includes('WebView cache cleanup must be scheduled for the next restart'), 'live WebView cleanup must be rejected')
const webviewAllowlist = storage.slice(
  storage.indexOf('const WEBVIEW_CACHE_RELATIVES'),
  storage.indexOf('pub struct StorageArtifactItem'),
)
for (const forbidden of ['Local Storage', 'IndexedDB', 'Cookies']) {
  assert.ok(!webviewAllowlist.includes(forbidden), `WebView cleanup allowlist must exclude ${forbidden}`)
}

const telemetry = read('src-tauri/src/commands/telemetry.rs')
assert.ok(telemetry.includes('TelemetryWrite::Vacuum'), 'telemetry VACUUM must use the serialized writer control queue')
assert.ok(telemetry.includes('requires all instances to be stopped'), 'telemetry VACUUM must reject active instances')

const main = read('src-tauri/src/main.rs')
const singleInstancePlugin = main.indexOf('tauri_plugin_single_instance::init')
const appSetup = main.indexOf('.setup(|app|')
const webviewCleanup = main.indexOf('process_scheduled_webview_cleanup()')
const webviewCreation = main.indexOf('tauri::WebviewWindowBuilder::new')
assert.ok(singleInstancePlugin < appSetup && appSetup < webviewCleanup && webviewCleanup < webviewCreation, 'WebView cache cleanup must run only in the primary app setup before WebView creation')

console.log('Artifact lifecycle regression checks passed.')
