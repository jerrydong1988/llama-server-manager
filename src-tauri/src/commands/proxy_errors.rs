//! Local routing failures: stable codes shared by HTTP responses and the usage ledger.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RoutingFailure {
    ModelRequired,
    ModelNotFound,
    EndpointMismatch,
    Unavailable,
    Capacity,
}

impl RoutingFailure {
    pub(super) fn response(self, format: ProxyApiFormat) -> Response {
        let (status, code, message, param) = match self {
            Self::ModelRequired => (
                StatusCode::BAD_REQUEST,
                "model_required",
                "model is required when no default or unambiguous implicit target is available",
                Some("model"),
            ),
            Self::ModelNotFound => (
                StatusCode::NOT_FOUND,
                "model_not_found",
                "no public route matches the requested model",
                Some("model"),
            ),
            Self::EndpointMismatch => (
                StatusCode::BAD_REQUEST,
                "model_endpoint_mismatch",
                "the requested model does not support this endpoint workload",
                Some("model"),
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "route_unavailable",
                "matching routes are stopped, loading or unhealthy",
                None,
            ),
            Self::Capacity => (
                StatusCode::SERVICE_UNAVAILABLE,
                "route_capacity",
                "all ready matching routes are at capacity; retry with bounded backoff",
                None,
            ),
        };
        let mut response = super::super::proxy_protocol::coded_error_response(
            format, status, "routing", code, message, param,
        );
        if matches!(self, Self::Capacity | Self::Unavailable) {
            // A retry hint, not a promise that a model finishes loading/generating in one second.
            response
                .headers_mut()
                .insert("retry-after", HeaderValue::from_static("1"));
        }
        response
    }
}

pub(super) fn unresolved_model_failure(
    config: &ProxyConfig,
    instances: &HashMap<String, InstanceConfig>,
    running: &HashMap<String, crate::models::RunningInstance>,
    requested_model: Option<&str>,
    workload: Option<ModelWorkload>,
) -> RoutingFailure {
    if workload.is_some()
        && !resolve_proxy_candidates_from(config, instances, running, requested_model, None)
            .is_empty()
    {
        return RoutingFailure::EndpointMismatch;
    }
    let Some(model) = requested_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
    else {
        return if config.default_instance_id.trim().is_empty() {
            RoutingFailure::ModelRequired
        } else {
            RoutingFailure::Unavailable
        };
    };
    let routes = config
        .routes
        .iter()
        .filter(|route| route_is_configured(route))
        .collect::<Vec<_>>();
    if routes.iter().any(|route| route.model_alias.trim() == model) {
        return RoutingFailure::Unavailable;
    }
    if config.strict_model_routing && !routes.is_empty() {
        return RoutingFailure::ModelNotFound;
    }
    // Inspect only public selectors (or explicitly enabled legacy selectors).
    // Never leak internal IDs through error distinctions in strict mode.
    let known = instances.iter().any(|(id, stored)| {
        let instance = running
            .get(id)
            .and_then(|item| item.launch_config.as_ref())
            .unwrap_or(stored);
        !routes
            .iter()
            .any(|route| route.target_instance_id.trim() == id)
            && (public_model_id(instance) == model
                || (!config.strict_model_routing && (id == model || instance.name.trim() == model)))
    });
    if known || (!config.strict_model_routing && !config.default_instance_id.trim().is_empty()) {
        RoutingFailure::Unavailable
    } else {
        RoutingFailure::ModelNotFound
    }
}
