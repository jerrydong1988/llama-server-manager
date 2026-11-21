use super::{
    adapt_managed_arguments_for_capabilities, blocked_security_flags, command_for_capabilities,
    emitted_override_keys, prepare_launch_checked, stale_engine_error, trusted_engine_capabilities,
    unsupported_command_flags, uses_manual_command, validate_custom_argument_capabilities,
    validate_effective_launch_security, validate_public_bind_auth, validate_tls_configuration,
    EngineCapabilityResolution, GeneratedServerCommand,
};
use crate::error::{AppError, AppResult};
use crate::models::{AppState, EngineCapabilities, InstanceConfig};
use crate::operation_timing::OperationTiming;
use crate::vector_policy::ModelWorkload;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LaunchPurpose {
    Preview,
    CompletePreview,
    Start,
}

impl LaunchPurpose {
    fn allows_preview_cache(self) -> bool {
        self != Self::Start
    }
}

pub(super) struct LaunchPlan {
    pub(super) config: InstanceConfig,
    pub(super) workload: ModelWorkload,
    pub(super) command: Vec<String>,
    pub(super) capabilities: Option<Box<EngineCapabilities>>,
    unsupported_flags: Vec<String>,
}

impl LaunchPlan {
    fn base(config: InstanceConfig, executable: &str) -> AppResult<Self> {
        let (config, workload, command) = prepare_launch_checked(config, executable)?;
        validate_tls_configuration(&config)?;
        validate_public_bind_auth(&config)?;
        Ok(Self {
            config,
            workload,
            command,
            capabilities: None,
            unsupported_flags: Vec::new(),
        })
    }

    /// Read capability evidence once. All parameter decisions below can be exercised
    /// with a fixed snapshot, independently of the host, WebView and process launcher.
    pub(super) async fn resolve(
        state: &AppState,
        config: InstanceConfig,
        executable: &str,
        purpose: LaunchPurpose,
        timing: &mut OperationTiming,
    ) -> AppResult<Self> {
        let plan = Self::base(config, executable)?;
        timing.mark("prepare");
        if uses_manual_command(&plan.config) {
            return Ok(plan);
        }
        let evidence = trusted_engine_capabilities(
            state,
            &plan.config,
            executable,
            purpose.allows_preview_cache(),
        )
        .await;
        timing.mark("capabilities");
        plan.with_capabilities(evidence, purpose)
    }

    fn with_capabilities(
        mut self,
        evidence: EngineCapabilityResolution,
        purpose: LaunchPurpose,
    ) -> AppResult<Self> {
        if crate::gufo::is_gufo(&self.config) {
            let caps = match evidence {
                EngineCapabilityResolution::Available(caps)
                    if caps.status == "detected" && caps.version_status == "detected" =>
                {
                    caps
                }
                _ => {
                    return Err(AppError::new(
                        "GUFO_PROBE_REQUIRED",
                        "Probe the selected Gufo engine before launching",
                        false,
                    ))
                }
            };
            self.command = crate::gufo::apply_launch_capabilities(&self.command, &caps)?;
            self.capabilities = Some(caps);
            return Ok(self);
        }
        self.capabilities = match evidence {
            EngineCapabilityResolution::Available(caps) => Some(caps),
            EngineCapabilityResolution::Missing => None,
            EngineCapabilityResolution::Stale => return Err(stale_engine_error()),
        };
        let caps = self.capabilities.as_deref();
        if purpose == LaunchPurpose::CompletePreview
            && caps.is_none_or(|c| c.status != "detected")
        {
            return Err(AppError::new("ENGINE_PREVIEW_PROBE_REQUIRED",
                "A complete engine capability probe is required to preview all configured arguments.", true));
        }
        validate_custom_argument_capabilities(&self.config, caps)?;
        let adapted = adapt_managed_arguments_for_capabilities(&self.command, caps);
        self.unsupported_flags =
            unsupported_command_flags(&adapted, caps.unwrap_or(&EngineCapabilities::default()));
        if purpose == LaunchPurpose::Start && !self.unsupported_flags.is_empty() {
            return Err(AppError::new("ENGINE_PARAMETER_UNSUPPORTED",
                format!("当前 llama-server 不支持以下已启用参数：{}。请返回参数配置清除这些参数，或更换兼容的引擎版本。", self.unsupported_flags.join(", ")), false));
        }
        let blocked = blocked_security_flags(&adapted, caps);
        if !blocked.is_empty() {
            return Err(AppError::new(
                "ENGINE_SECURITY_PARAMETER_UNVERIFIED",
                format!(
                    "当前引擎尚未确认支持以下安全参数：{}。请先完成引擎能力探测。",
                    blocked.join(", ")
                ),
                false,
            ));
        }
        self.command = command_for_capabilities(&adapted, caps);
        // Start validates the final argv after adding any managed checkpoint arguments.
        if purpose != LaunchPurpose::Start {
            validate_effective_launch_security(&self.config, &self.command)?;
        }
        Ok(self)
    }

    pub(super) fn into_preview(self, executable: &str) -> GeneratedServerCommand {
        let emitted_override_keys = if crate::gufo::is_gufo(&self.config) {
            self.config.explicit_overrides.unwrap_or_default()
        } else if uses_manual_command(&self.config) {
            Vec::new()
        } else {
            emitted_override_keys(
                &self.config,
                executable,
                self.capabilities.as_deref(),
                &self.command,
            )
        };
        GeneratedServerCommand {
            command: self.command,
            unsupported_flags: self.unsupported_flags,
            emitted_override_keys,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn managed() -> LaunchPlan {
        LaunchPlan::base(
            InstanceConfig {
                model_path: "two words.gguf".into(),
                explicit_overrides: Some(vec!["temp".into()]),
                ..InstanceConfig::default()
            },
            "llama-server",
        )
        .unwrap()
    }

    #[test]
    fn complete_preview_requires_probe_but_normal_preview_can_use_conservative_arguments() {
        let plan = managed()
            .with_capabilities(EngineCapabilityResolution::Missing, LaunchPurpose::Preview)
            .unwrap();
        assert_eq!(plan.command[0], "llama-server");
        let err = managed()
            .with_capabilities(
                EngineCapabilityResolution::Missing,
                LaunchPurpose::CompletePreview,
            )
            .err()
            .unwrap();
        assert_eq!(err.code, "ENGINE_PREVIEW_PROBE_REQUIRED");
        assert!(err.retryable);
        for purpose in [LaunchPurpose::Preview, LaunchPurpose::CompletePreview] {
            let err = managed()
                .with_capabilities(EngineCapabilityResolution::Stale, purpose)
                .err()
                .unwrap();
            assert_eq!(err.code, "ENGINE_CAPABILITIES_STALE");
        }
    }

    #[test]
    fn start_requires_fresh_identity_and_rejects_flags_reported_in_preview() {
        assert!(LaunchPurpose::Preview.allows_preview_cache());
        assert!(LaunchPurpose::CompletePreview.allows_preview_cache());
        assert!(!LaunchPurpose::Start.allows_preview_cache());
        let evidence = || {
            EngineCapabilityResolution::Available(Box::new(EngineCapabilities {
                status: "detected".into(),
                supported_flags: vec!["-m".into(), "-a".into(), "--port".into(), "--host".into()],
                ..EngineCapabilities::default()
            }))
        };
        let preview = managed()
            .with_capabilities(evidence(), LaunchPurpose::CompletePreview)
            .unwrap();
        assert!(preview
            .unsupported_flags
            .iter()
            .any(|flag| flag == "--temp"));
        let err = managed()
            .with_capabilities(evidence(), LaunchPurpose::Start)
            .err()
            .unwrap();
        assert_eq!(err.code, "ENGINE_PARAMETER_UNSUPPORTED");
        assert!(err.message.contains("--temp"));
        assert_eq!(
            managed()
                .with_capabilities(EngineCapabilityResolution::Stale, LaunchPurpose::Start)
                .err()
                .unwrap()
                .code,
            "ENGINE_CAPABILITIES_STALE"
        );
    }

    #[test]
    fn manual_preview_uses_the_public_wire_shape_and_preserves_argv() {
        let plan = LaunchPlan::base(
            InstanceConfig {
                launch_mode: "manual".into(),
                port: 18123,
                manual_command: r#"--model "two words.gguf" --temp 0.5 --temp 0.75"#.into(),
                ..InstanceConfig::default()
            },
            "llama-server",
        )
        .unwrap();
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../scripts/fixtures/manual-command-preview.json"
        ))
        .unwrap();
        assert_eq!(
            serde_json::to_value(plan.into_preview("llama-server")).unwrap(),
            expected
        );
    }
}
