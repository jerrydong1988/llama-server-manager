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

console.log('Artifact lifecycle regression checks passed.')
