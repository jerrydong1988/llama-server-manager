mod multi_slot {
    use super::*;

    #[test]
    fn reviewed_multi_slot_requires_retaining_native_idle_slots() {
        let mut config = eligible_config();
        config.parallel = 4;
        let evaluate_multi = |config: &InstanceConfig| {
            evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
                config,
                workload: ModelWorkload::Inference,
                managed_local_engine: true,
                engine_capabilities: EngineCheckpointCapabilities {
                    slots: true,
                    slot_save_path: true,
                    cache_ram: true,
                    cache_idle_slots: true,
                    swa_full: true,
                    context_checkpoint_persistence: true,
                    reviewed_swa_checkpoint_reuse: false,
                    reviewed_multi_slot: true,
                },
                engine_speculative_types: &[],
                model_architecture: Some("llama"),
                model_artifacts_complete: true,
                model_has_swa: Some(false),
            })
        };
        assert!(evaluate_multi(&config)
            .reasons
            .contains(&CheckpointReasonCode::MultiSlotIdleCacheUnsupported));
        config.cache_idle_slots = false;
        config.cache_ram = 0;
        let result = evaluate_multi(&config);
        assert!(result.eligible, "{result:?}");
        assert_eq!(result.slot_count, 4);
        // Single-slot qualification keeps its original RAM retention contract.
        config.parallel = 1;
        assert!(evaluate_multi(&config)
            .reasons
            .contains(&CheckpointReasonCode::PromptCacheRetentionRequired));
    }

    struct MultiBackend {
        slots: Vec<FakeSlotBackend>,
        corrupt_first_after_last: bool,
    }

    impl MultiBackend {
        fn new(store: &CheckpointStore, count: u32) -> Self {
            let slots = (0..count)
                .map(|id| {
                    let backend = FakeSlotBackend::new(store, "instance-1");
                    backend.configure(|s| {
                        s.prompt_tokens = 512 + u64::from(id);
                        s.restore_tokens = s.prompt_tokens;
                        s.payload = format!("independent slot {id} state").into_bytes();
                    });
                    backend
                })
                .collect();
            Self {
                slots,
                corrupt_first_after_last: false,
            }
        }
    }

    impl SlotBackend for MultiBackend {
        fn health(&self) -> StoreResult<()> {
            Ok(())
        }
        fn slots(&self) -> StoreResult<Vec<SlotSnapshot>> {
            self.slots
                .iter()
                .enumerate()
                .map(|(id, b)| {
                    let mut s = b.slots()?.remove(0);
                    s.id = id as u32;
                    Ok(s)
                })
                .collect()
        }
        fn save(&self, f: &str) -> StoreResult<SlotSaveResult> {
            self.save_slot(0, f)
        }
        fn restore(&self, f: &str) -> StoreResult<SlotRestoreResult> {
            self.restore_slot(0, f)
        }
        fn erase(&self) -> StoreResult<SlotEraseResult> {
            panic!("a failed restore must replace the process, not erase")
        }
        fn save_slot(&self, id: u32, f: &str) -> StoreResult<SlotSaveResult> {
            let mut r = self.slots[id as usize].save(f)?;
            r.id_slot = id;
            Ok(r)
        }
        fn restore_slot(&self, id: u32, f: &str) -> StoreResult<SlotRestoreResult> {
            let mut r = self.slots[id as usize].restore(f)?;
            r.id_slot = id;
            if self.corrupt_first_after_last && id as usize == self.slots.len() - 1 {
                self.slots[0].configure(|s| s.payload = b"damaged shared state".to_vec());
            }
            Ok(r)
        }
    }

    fn coordinator(
        store: &CheckpointStore,
        count: u32,
        pid: u32,
    ) -> (CheckpointCoordinator, InstanceConfig, CheckpointFingerprint) {
        let c = CheckpointCoordinator::new(store.clone());
        let mut cfg = eligible_config();
        cfg.parallel = count as i32;
        let fp = test_fingerprint(&cfg);
        let mut eligible = CheckpointEligibility::from_reasons(Vec::new());
        eligible.slot_count = count;
        c.register_start_with_context(
            "instance-1",
            pid,
            &eligible,
            Some(fp.clone()),
            cfg.kv_checkpoint.clone(),
        )
        .unwrap();
        c.on_engine_healthy("instance-1", pid).unwrap();
        (c, cfg, fp)
    }

    fn saved(store: &CheckpointStore, count: u32) -> (MultiBackend, LoadedGeneration) {
        let b = MultiBackend::new(store, count);
        let (c, cfg, fp) = coordinator(store, count, 101);
        c.restore_or_cold("instance-1", 101, &fp, false, &b)
            .unwrap();
        c.begin_draining("instance-1", 101).unwrap();
        let result = c
            .save_before_stop("instance-1", 101, &fp, &cfg.kv_checkpoint, &b)
            .unwrap();
        assert_eq!(result.last_outcome, CheckpointOutcome::Success);
        (
            b,
            store
                .load_latest("instance-1", &fp.digest)
                .unwrap()
                .unwrap(),
        )
    }

    #[test]
    fn all_slots_restore_before_roundtrip_and_routing() {
        let sandbox = TestSandbox::new("multi-roundtrip");
        let store = sandbox.store();
        let (b, old) = saved(&store, 4);
        assert_eq!(old.manifest.schema_version, 2);
        assert_eq!(old.manifest.slots.len(), 4);
        let (c, _, fp) = coordinator(&store, 4, 102);
        for slot in &b.slots {
            slot.configure(|s| {
                s.prompt_tokens = 0;
                s.payload.clear();
            });
        }
        assert!(!c.gate_allows_routing("instance-1"));
        let result = c.restore_or_cold("instance-1", 102, &fp, true, &b).unwrap();
        assert_eq!(result.phase, CheckpointPhase::Ready);
        assert_eq!(result.prompt_tokens, Some(2054));
        assert_eq!(result.bytes, Some(old.manifest.payload_bytes()));
        assert!(c.gate_allows_routing("instance-1"));
        for (i, slot) in b.slots.iter().enumerate() {
            assert_eq!(
                slot.state.lock().unwrap().payload,
                format!("independent slot {i} state").as_bytes()
            );
        }
    }

    #[test]
    fn partial_restore_and_shared_state_damage_never_become_routable() {
        for damage in [false, true] {
            let sandbox = TestSandbox::new("multi-poison");
            let store = sandbox.store();
            let (mut b, old) = saved(&store, 2);
            if damage {
                b.corrupt_first_after_last = true;
            } else {
                b.slots[1].configure(|s| s.restore_claim_bytes = Some(1));
            }
            let (c, _, fp) = coordinator(&store, 2, 202);
            let result = c.restore_or_cold("instance-1", 202, &fp, true, &b).unwrap();
            assert_eq!(result.phase, CheckpointPhase::RestartRequired);
            assert!(!c.gate_allows_routing("instance-1"));
            assert!(old.generation_dir().join("restore-rejected.json").exists());
            // The first slot really was restored before failure, but the whole
            // process still needs replacement, not successful-slot publication.
            assert!(b.slots[0].state.lock().unwrap().prompt_tokens > 0);
            assert!(c.restore_restart_required("instance-1", 202));
        }
    }

    #[test]
    fn second_slot_save_failure_keeps_old_generation_and_cleans_scratch() {
        let sandbox = TestSandbox::new("multi-save-fault");
        let store = sandbox.store();
        let (b, old) = saved(&store, 2);
        b.slots[1]
            .configure(|s| s.save_error = Some(CheckpointStoreError::io("injected write denial")));
        let (c, cfg, fp) = coordinator(&store, 2, 303);
        c.restore_or_cold("instance-1", 303, &fp, false, &b)
            .unwrap();
        c.begin_draining("instance-1", 303).unwrap();
        let result = c
            .save_before_stop("instance-1", 303, &fp, &cfg.kv_checkpoint, &b)
            .unwrap();
        assert_eq!(result.last_outcome, CheckpointOutcome::Failed);
        assert_eq!(
            store
                .load_latest("instance-1", &fp.digest)
                .unwrap()
                .unwrap()
                .manifest,
            old.manifest
        );
        assert_eq!(
            fs::read_dir(store.prepare_instance("instance-1").unwrap())
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn partial_commit_fault_retains_latest_and_aggregate_budget_is_enforced() {
        let sandbox = TestSandbox::new("multi-commit-fault");
        let store = sandbox.store();
        let (_, old) = saved(&store, 2);
        for budget_failure in [true, false] {
            let mut manifest = old.manifest.clone();
            manifest.generation_id = uuid::Uuid::new_v4().to_string();
            let paths: Vec<_> = manifest
                .slots
                .iter()
                .map(|slot| {
                    store
                        .copy_slot_to_scratch("instance-1", 404, &old, slot)
                        .unwrap()
                })
                .collect();
            let moves = std::cell::Cell::new(0);
            let result = store.commit_slots_with_fault(
                &manifest,
                &paths,
                if budget_failure { 1 } else { 1024 * 1024 },
                |point| {
                    if point == StoreFaultPoint::AfterPayloadMove {
                        moves.set(moves.get() + 1);
                        if moves.get() == 2 {
                            return Err(CheckpointStoreError::io("second payload move failure"));
                        }
                    }
                    Ok(())
                },
            );
            assert!(result.is_err());
            assert_eq!(
                store
                    .load_latest("instance-1", &manifest.fingerprint.digest)
                    .unwrap()
                    .unwrap()
                    .manifest,
                old.manifest
            );
            for p in &paths {
                store.remove_scratch_payload("instance-1", p).unwrap();
            }
        }
    }

    #[test]
    fn per_slot_longer_retention_does_not_block_other_slot_progress() {
        let sandbox = TestSandbox::new("multi-longer");
        let store = sandbox.store();
        let (b, old) = saved(&store, 2);
        b.slots[0].configure(|s| {
            s.prompt_tokens = 5;
            s.payload = b"short title".to_vec();
        });
        b.slots[1].configure(|s| {
            s.prompt_tokens = 2048;
            s.payload = b"new long conversation".to_vec();
        });
        let (c, mut cfg, fp) = coordinator(&store, 2, 505);
        cfg.kv_checkpoint.preserve_longer = true;
        c.restore_or_cold("instance-1", 505, &fp, false, &b)
            .unwrap();
        c.begin_draining("instance-1", 505).unwrap();
        assert_eq!(
            c.save_before_stop("instance-1", 505, &fp, &cfg.kv_checkpoint, &b)
                .unwrap()
                .last_outcome,
            CheckpointOutcome::Success
        );
        let new = store
            .load_latest("instance-1", &fp.digest)
            .unwrap()
            .unwrap();
        assert_eq!(new.manifest.slots[0], old.manifest.slots[0]);
        assert_eq!(new.manifest.slots[1].prompt_tokens, 2048);
        assert_eq!(
            fs::read(old.generation_dir().join("slot-1.bin")).unwrap(),
            b"independent slot 1 state"
        );
    }

    #[test]
    fn manifest_rejects_duplicate_ids_paths_schema_and_any_corrupt_payload() {
        let sandbox = TestSandbox::new("multi-manifest");
        let store = sandbox.store();
        let (_, old) = saved(&store, 2);
        for edit in [
            |m: &mut CheckpointManifestV1| m.slots[1].id = 0,
            |m: &mut CheckpointManifestV1| m.slots[1].filename = "../slot-1.bin".into(),
            |m: &mut CheckpointManifestV1| m.slot_count = 1,
            |m: &mut CheckpointManifestV1| m.schema_version = 1,
        ] {
            let mut m = old.manifest.clone();
            edit(&mut m);
            assert!(m.validate("instance-1", &m.fingerprint.digest).is_err());
        }
        fs::write(old.generation_dir().join("slot-1.bin"), b"corrupt").unwrap();
        assert!(store
            .load_generation(
                "instance-1",
                &old.manifest.fingerprint.digest,
                old.generation_dir().parent().unwrap(),
                &old.manifest.generation_id,
                None
            )
            .is_err());
        assert!(store
            .load_latest("instance-1", &old.manifest.fingerprint.digest)
            .is_err());
    }
    #[test]
    fn idle_save_tracks_nonzero_slots_and_requires_a_complete_quiet_set() {
        let sandbox = TestSandbox::new("multi-idle");
        let store = sandbox.store();
        let (c, _, fp) = coordinator(&store, 4, 800);
        let b = MultiBackend::new(&store, 4);
        c.restore_or_cold("instance-1", 800, &fp, false, &b)
            .unwrap();
        c.entries
            .lock()
            .unwrap()
            .get_mut("instance-1")
            .unwrap()
            .policy
            .idle_save_seconds = 5;
        let mut slots: Vec<_> = (0..4)
            .map(|id| crate::commands::telemetry::SlotSnapshotRecord {
                slot_id: id,
                is_processing: false,
                n_ctx: 4096,
                n_past: None,
                task_id: Some(10 + u64::from(id)),
                cached_tokens: Some(512),
                processed_tokens: Some(1),
            })
            .collect();
        let age = || {
            let mut entries = c.entries.lock().unwrap();
            let e = entries.get_mut("instance-1").unwrap();
            e.idle.quiet_since = Some(Instant::now() - Duration::from_secs(6));
        };
        assert!(!c.idle_save_due("instance-1", 800, &slots, 0));
        age();
        slots[3].is_processing = true;
        assert!(!c.idle_save_due("instance-1", 800, &slots, 0));
        slots[3].is_processing = false;
        assert!(!c.idle_save_due("instance-1", 800, &slots, 0));
        age();
        assert!(c.idle_save_due("instance-1", 800, &slots, 0));
        age();
        slots[3].task_id = Some(100);
        assert!(!c.idle_save_due("instance-1", 800, &slots, 0));
        age();
        assert!(!c.idle_save_due("instance-1", 800, &slots, 1));
        age();
        assert!(!c.idle_save_due("instance-1", 800, &slots[..3], 0));
        age();
        slots[3].slot_id = 2;
        assert!(!c.idle_save_due("instance-1", 800, &slots, 0));
    }

    #[test]
    fn sparse_generation_restores_nonzero_slots_and_reports_the_measured_slot() {
        let sandbox = TestSandbox::new("multi-sparse");
        let store = sandbox.store();
        let b = MultiBackend::new(&store, 4);
        for i in 0..3 {
            b.slots[i].configure(|s| s.prompt_tokens = 0);
        }
        let (c, cfg, fp) = coordinator(&store, 4, 900);
        c.restore_or_cold("instance-1", 900, &fp, false, &b)
            .unwrap();
        c.begin_draining("instance-1", 900).unwrap();
        assert_eq!(
            c.save_before_stop("instance-1", 900, &fp, &cfg.kv_checkpoint, &b)
                .unwrap()
                .last_outcome,
            CheckpointOutcome::Success
        );
        let g = store
            .load_latest("instance-1", &fp.digest)
            .unwrap()
            .unwrap();
        assert_eq!(g.manifest.slots.len(), 1);
        assert_eq!(g.manifest.slots[0].id, 3);
        let (fresh, _, _) = coordinator(&store, 4, 901);
        assert_eq!(
            fresh
                .restore_or_cold("instance-1", 901, &fp, true, &b)
                .unwrap()
                .phase,
            CheckpointPhase::Ready
        );
        fresh.observe_restored_request(
            "instance-1",
            901,
            &crate::commands::telemetry::SlotSnapshotRecord {
                slot_id: 3,
                is_processing: true,
                n_ctx: 4096,
                n_past: None,
                task_id: Some(99),
                cached_tokens: Some(515),
                processed_tokens: Some(1),
            },
        );
        assert_eq!(
            fresh
                .status("instance-1")
                .unwrap()
                .reuse_observation
                .unwrap()
                .slot_id,
            3
        );
    }
}
