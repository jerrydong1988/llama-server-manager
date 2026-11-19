use super::*;
use crate::models::InstanceConfig;

fn fixture() -> (EngineCapabilities, InstanceConfig, String) {
    let profile = profiles().iter().find(|p| p.id == "windows-rocm-b12469-qwen36-embedded-mtp").unwrap();
    let caps = EngineCapabilities {
        persistence_profile: Some(profile.id.clone()),
        persistence_revision: PERSISTENCE_REVISION,
        context_checkpoint_persistence: true,
        checkpoint_draft_types: vec!["draft-mtp".into()],
        speculative_types: vec!["draft-mtp".into(), "draft-simple".into(), "ngram-mod".into()],
        ..Default::default()
    };
    let config = InstanceConfig {
        spec_type: "draft-mtp".into(), draft_tokens: 3, parallel: 1,
        ctx_size: 8192, ctx_size_auto: false, ctx_checkpoints: 2, checkpoint_min_step: 512,
        batch_size: 512, ubatch_size: 512, kv_unified: true, kv_unified_mode: "on".into(),
        gpu_layers: 99, gpu_layers_auto: false, draft_gpu_layers: 99,
        flash_attn: "on".into(), fit_mode: "off".into(),
        ..Default::default()
    };
    (caps, config, profile.model_scopes[0].model_sha256.clone())
}

#[test]
fn b12469_capability_requires_complete_bundle_not_version_or_help_guess() {
    let profile = profiles().iter().find(|p| p.id == "windows-rocm-b12469-qwen36-embedded-mtp").unwrap();
    let help = "--slot-save-path FNAME path to save slot kv cache (default: disabled)";
    for digest in [None, Some("b12469"), Some(profile.launcher_sha256.as_str()), profile.base_artifact_sha256.as_deref()] {
        assert!(!persistence_evidence(help, digest).0);
    }
    let evidence = persistence_evidence(help, Some(&profile.artifact_sha256));
    assert_eq!(evidence.0, cfg!(windows));
    assert_eq!(evidence.1.is_some(), cfg!(windows));
    assert!(!profile.managed_lookup_cache);
}

#[test]
fn b12469_pairs_are_candidates_only_and_never_admit_four_slot_q8() {
    let (caps, mut config, model) = fixture();
    assert!(!reviewed_context_checkpoint_persistence(&caps));
    for slots in [1, 2, 3, 4] {
        for kv in ["f16", "q8_0", "q4_0"] {
            config.parallel = slots; config.ctx_size = 8192 * slots as u32;
            config.cache_type_k = kv.into(); config.cache_type_v = kv.into();
            config.cache_type_draft_k = kv.into(); config.cache_type_draft_v = kv.into();
            let expected = cfg!(windows) && matches!((slots, kv), (1 | 2, "f16" | "q8_0") | (4, "f16"));
            let models = Some((model.as_str(), ""));
            assert_eq!(scoped_embedded_mtp_candidate(&config, &caps, models), expected, "{slots} {kv}");
            assert_eq!(allows_local_checkpoint_validation(&config, &caps, models), expected);
            assert!(!reviewed_windowed_swa(&config, &caps, models));
            assert!(!reviewed_multi_slot(&config, &caps, models));
            assert!(!checkpoint_types_with_models(&config, &caps, models).contains(&"draft-mtp".into()), "bundle evidence must not bypass a local receipt");
        }
    }
}

#[test]
fn b12469_native_capability_does_not_grant_other_hybrid_architectures() {
    use crate::checkpoint::{evaluate_checkpoint_eligibility, CheckpointEligibilityContext, EngineCheckpointCapabilities, CheckpointReasonCode};
    let (caps, mut config, _) = fixture();
    config.kv_checkpoint.enabled = true;
    config.spec_type = "none".into();
    config.cache_idle_slots = true;
    config.cache_ram = 256;
    config.slots_enabled = true;
    config.mmproj_auto = false;
    for architecture in ["qwen35", "qwen35moe"] {
        let result = evaluate_checkpoint_eligibility(CheckpointEligibilityContext {
            config: &config, workload: crate::vector_policy::ModelWorkload::Inference,
            managed_local_engine: true,
            engine_capabilities: EngineCheckpointCapabilities {
                slots: true, slot_save_path: true, cache_ram: true, cache_idle_slots: true,
                context_checkpoint_persistence: reviewed_context_checkpoint_persistence(&caps),
                ..Default::default()
            },
            engine_speculative_types: &[], model_architecture: Some(architecture),
            model_artifacts_complete: true, model_has_swa: Some(false),
        });
        assert!(!result.eligible);
        assert!(result.reasons.contains(&CheckpointReasonCode::HybridRecurrentUnsupported));
    }
    let legacy = EngineCapabilities { persistence_profile: None, ..caps };
    assert!(reviewed_context_checkpoint_persistence(&legacy), "keep the existing generic help capability behavior");
}

#[test]
fn b12469_rejects_changed_model_draft_context_and_launch_shape() {
    let (caps, config, model) = fixture();
    let models = Some((model.as_str(), ""));
    for identities in [None, Some(("wrong model", "")), Some((model.as_str(), "external draft"))] {
        assert!(!allows_local_checkpoint_validation(&config, &caps, identities));
    }
    let changes: &[fn(&mut InstanceConfig)] = &[
        |c| c.draft_model_path = c.model_path.clone() + "draft.gguf",
        |c| c.ctx_size = 4096, |c| c.ctx_size = 16384,
        |c| c.batch_size = 2048, |c| c.ubatch_size = 256,
        |c| c.checkpoint_min_step = 8192, |c| c.ctx_checkpoints = 0,
        |c| c.spec_type = "ngram-mod,draft-mtp".into(),
        |c| c.spec_type = "draft-simple".into(), |c| c.draft_tokens = 4,
        |c| c.cache_type_v = "q8_0".into(), |c| c.cache_type_draft_k = "q8_0".into(),
        |c| c.gpu_layers = 1, |c| c.flash_attn = "off".into(),
        |c| c.fit_mode = "on".into(), |c| c.kv_unified = false,
        |c| c.explicit_overrides = Some(vec!["ctx_size".into()]),
    ];
    for change in changes {
        let mut changed = config.clone(); change(&mut changed);
        assert!(!allows_local_checkpoint_validation(&changed, &caps, models));
    }
    let mut old_caps = caps; old_caps.persistence_revision -= 1;
    assert!(!allows_local_checkpoint_validation(&config, &old_caps, models));
}
