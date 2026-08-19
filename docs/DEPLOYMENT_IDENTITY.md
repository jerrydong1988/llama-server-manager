# Versioned Deployment Identity

LSM creates a versioned, local identity chain before it starts or recovers an instance. The chain binds the exact engine artifact, primary model artifact, immutable configuration revision, and sealed qualification evidence that were accepted by preflight.

LSM 在启动或恢复实例前创建版本化的本地身份链。该身份链绑定预检实际接受的引擎制品、主模型制品、不可变配置修订和已封存的资格证据。

## Identity chain

| Identity | Schema | Source | Invalidated when |
| --- | --- | --- | --- |
| Engine artifact | `urn:lsm:engine:v1:sha256:*` | bounded samples of the executable | sampled bytes or file size change |
| Model artifact | `urn:lsm:model:v1:sha256:*` | bounded samples of the selected primary GGUF | sampled bytes or file size change |
| Configuration | `urn:lsm:configuration:v1:sha256:*` | canonical deployment-affecting configuration | a deployment-affecting field changes |
| Qualification evidence | `urn:lsm:qualification:v2:sha256:*` | sealed terminal qualification report | report content, engine identity, or representative model identity changes |
| Deployment | `urn:lsm:deployment:v1:sha256:*` | the four identities above plus the immutable configuration revision ID | any component identity changes |

The artifact algorithm is `sha256-sampled-v1`: at most five 64 KiB regions are read at deterministic offsets. Paths and timestamps are deliberately excluded, so moving an unchanged file preserves its identity. This is a bounded change detector for local operations, not a claim that the complete file was cryptographically hashed, signed, or authenticated. File authorization remains a separate security boundary.

制品算法为 `sha256-sampled-v1`：在确定性偏移位置最多读取五段、每段 64 KiB。路径和时间戳不参与计算，因此仅移动未修改文件不会改变身份。它是面向本地运维的有界变化检测器，不代表已经完整哈希、签名或认证整个文件；文件访问授权仍是独立的安全边界。

## Migration and fail-closed rules

- Inventory schema 6 persists artifact identity. Schema 5 rows remain visible during refresh but are marked unverified until the file is sampled again.
- Configuration revision schema 2 adds the stable configuration identity to event integrity. A valid schema 1 history is upgraded without changing revision IDs, parent links, known-good pointers, or audit events. Corrupt legacy events remain visible and invalid; migration never blesses them.
- Qualification schema 2 seals engine and representative-model artifact identities into a deterministic evidence ID. Legacy, malformed, incomplete, or stale evidence cannot pass the launch gate.
- Runtime state schema 3 persists the composite identity. Legacy snapshots can be read for diagnostics, but automatic recovery fails closed until a new verified launch snapshot exists.

## Operator workflow

1. Refresh model and engine inventories after installing, replacing, or moving artifacts.
2. Re-run engine qualification when its artifact or representative model changes.
3. Save the instance configuration so it has a current immutable revision.
4. Check **Configuration → Deployment Identity**. “Verified and ready” means every component is current.
5. Start the instance. Both initial launch and background recovery re-sample the engine and primary model and re-check the configuration and composite IDs.

The status panel exposes only stable IDs and error codes. It does not expose filesystem paths, credentials, certificate material, manual commands, or private configuration values.

## Deliberate Phase 1 limits

This foundation identifies one concrete local launch. It does not introduce deployment objects, resource planning, canary promotion, automatic rollback, routing policy, remote registries, artifact signing, or auxiliary-model identities. Those are governed by later roadmap phases. Auxiliary paths remain protected by the configuration revision until their dedicated artifact semantics are designed.
