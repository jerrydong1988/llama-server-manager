# Versioned Deployment Identity

LSM creates a versioned, local identity chain before it starts or recovers an instance. The chain binds the exact engine artifact, primary model artifact, immutable configuration revision, and qualification or advisory evidence recorded by preflight.

LSM 在启动或恢复实例前创建版本化的本地身份链。该身份链绑定预检实际接受的引擎制品、主模型制品、不可变配置修订，以及资格认证或告警状态证据。

## Identity chain

| Identity | Schema | Source | Invalidated when |
| --- | --- | --- | --- |
| Engine artifact | `urn:lsm:engine:v1:sha256:*` | complete executable plus the recursive DLL bundle on Windows; complete executable elsewhere | any bound byte or Windows DLL membership changes |
| Model artifact | `urn:lsm:model:v1:sha256:*` | complete selected primary GGUF | any file byte changes |
| Configuration | `urn:lsm:configuration:v1:sha256:*` | canonical deployment-affecting configuration | a deployment-affecting field changes |
| Qualification evidence | `urn:lsm:qualification:v2:sha256:*` | sealed passed report, or deterministic advisory admission material | report/advisory status, profile, or engine identity changes |
| Deployment | `urn:lsm:deployment:v1:sha256:*` | the four identities above plus the immutable configuration revision ID | any component identity changes |

Models and non-Windows engines use `sha256-full-v1`. Windows engines use `sha256-engine-bundle-v1`, which seals the complete executable and every recursively discovered DLL into one deterministic identity. Paths and timestamps are excluded from the artifact ID, so moving unchanged content preserves its identity. These identities prove content equality within the bound artifact set; they do not prove publisher authenticity or replace code signing.

模型和非 Windows 引擎使用 `sha256-full-v1`；Windows 引擎使用 `sha256-engine-bundle-v1`，把完整可执行文件以及递归发现的全部 DLL 封装为一个确定性身份。路径和时间戳不参与制品 ID，因此仅移动未修改内容不会改变身份。这些身份用于证明已绑定制品集合的内容一致性，但不证明发布者身份，也不能替代代码签名。

## External Windows engine directories

An authorized scan root may remain anywhere the operator chooses. If its Windows ACL already prevents other principals from writing the engine, LSM probes and runs it directly. Otherwise LSM automatically copies the executable and its recursive DLL bundle through retained, non-replaceable source handles into an owner-and-SYSTEM-only, content-addressed directory under `engine-snapshots/v1`. It verifies the copied bundle before atomically publishing and executing that private snapshot. The source directory is not moved, modified, or granted new permissions.

The source path and source fingerprint remain the evidence boundary. Replacing the source invalidates the probe and qualification even when an older private snapshot still exists. A snapshot removes the writable-directory race for unattended execution; it does not certify that user-selected source content was benign.

已授权的 Windows 扫描根目录可以保留在用户选择的任意位置。如果其 ACL 已阻止其他主体写入，引擎会直接探测和运行；否则，LSM 会通过不可替换的已保留源句柄，把可执行文件及递归 DLL 集合复制到 `engine-snapshots/v1` 下仅当前所有者和 SYSTEM 可访问的内容寻址目录。副本完成校验并原子发布后才会用于探测、认证、启动和后台恢复，原目录不会被移动、修改或重设权限。

扫描证据仍绑定原始路径和原始指纹。原始内容被替换后，即使旧快照仍存在，探测与认证也会失效。私有快照解决的是可写目录中的竞态替换风险，并不证明用户所选源内容本身可信。

## External Windows model directories

External GGUF models remain in their operator-authorized directories. When the model tree already satisfies the strict ACL policy, LSM uses the normal protected binding. When another Windows principal has write-like ACL rights, LSM does not copy the potentially very large model. It instead verifies the complete content identity, retains a no-write/no-delete-sharing handle to the selected file, and binds ancestor directories from the authorized root to the model before qualification or launch. These handles remain live for the qualification run or managed server lifetime, and the model is re-hashed after process creation. A conflicting writer, replacement, deletion, identity mismatch, or post-launch change fails closed.

This fallback authorizes read-only execution, not destructive management. Model deletion continues to require the strict owner-protected ACL policy. LSM does not rewrite the source ACL or claim that user-selected model content is trustworthy.

Windows 外部 GGUF 模型会继续保留在操作者已授权的目录中。目录满足严格 ACL 时使用常规保护绑定；如果其他 Windows 主体拥有写入类权限，LSM 不会复制可能非常庞大的模型，而是在认证或启动前校验完整内容身份，保留拒绝写入和删除共享的文件句柄，并从授权根目录到模型逐级绑定祖先目录。这些句柄会覆盖整个资格认证或托管服务生命周期，并在进程创建后重新哈希模型。若存在冲突写入、替换、删除、身份不一致或启动期间变更，操作会安全失败。

该降级路径只授权只读运行，不授权破坏性管理。删除模型仍要求严格的所有者保护 ACL。LSM 不会修改源目录 ACL，也不声称用户所选模型内容本身可信。

## Migration and fail-closed rules

- Inventory schema 7 persists complete artifact identities. Compatible schema 5 and 6 rows remain visible during refresh but are excluded from incremental reuse until each file is completely hashed again.
- Configuration revision schema 2 adds the stable configuration identity to event integrity. A valid schema 1 history is upgraded without changing revision IDs, parent links, known-good pointers, or audit events. Corrupt legacy events remain visible and invalid; migration never blesses them.
- Qualification schema 2/profile 3 seals a passed GPU-offload or CPU-backend smoke report. Missing, failed, incomplete, cancelled, or stale qualification creates deterministic advisory admission evidence instead of blocking launch; exact artifact, configuration, security, and recovery checks remain fail-closed.
- Runtime state schema 3 persists the composite identity. Legacy snapshots can be read for diagnostics, but automatic recovery fails closed until a new verified launch snapshot exists.

## Operator workflow

1. Refresh model and engine inventories after installing, replacing, or moving artifacts.
2. Re-run engine qualification when its artifact or representative model changes and current compatibility evidence is useful.
3. Save the instance configuration so it has a current immutable revision.
4. Check **Configuration → Deployment Identity**. “Verified and ready” means the hard artifact/configuration bindings are current; qualification may still carry an advisory warning.
5. Start the instance. Both initial launch and background recovery re-hash the engine and primary model and re-check the configuration and composite IDs.

The status panel exposes only stable IDs and error codes. It does not expose filesystem paths, credentials, certificate material, manual commands, or private configuration values.

## Deliberate Phase 1 limits

This foundation identifies one concrete local launch. It does not introduce deployment objects, resource planning, canary promotion, automatic rollback, routing policy, remote registries, artifact signing, or auxiliary-model identities. Those are governed by later roadmap phases. Auxiliary paths remain protected by the configuration revision until their dedicated artifact semantics are designed.
