use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

use crate::provider::{LlmProvider, LlmRequest, LlmResponse, StreamChunk};
use claw_core::Result;

/// Maximum retry attempts for transient errors (429, 500, 502, 503).
const MAX_RETRIES: u32 = 3;
/// Base delay for exponential backoff (doubles each retry).
const BASE_DELAY_MS: u64 = 1000;

// ── Circuit Breaker ────────────────────────────────────────────

/// Number of consecutive failures before opening the circuit.
const CIRCUIT_FAILURE_THRESHOLD: u32 = 5;
/// How long the circuit stays open before allowing a probe request.
const CIRCUIT_OPEN_DURATION: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CircuitState {
    /// Normal operation — requests flow through.
    Closed,
    /// Provider is failing — reject requests immediately.
    Open { since: Instant },
    /// Allow a single probe request to test if provider recovered.
    HalfOpen,
}

#[derive(Debug)]
struct CircuitBreaker {
    state: CircuitState,
    consecutive_failures: u32,
    total_failures: u64,
    total_successes: u64,
    last_failure_time: Option<Instant>,
}

impl CircuitBreaker {
    fn new() -> Self {
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            total_failures: 0,
            total_successes: 0,
            last_failure_time: None,
        }
    }

    /// Check whether a request should be allowed.
    fn allow_request(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open { since } => {
                if since.elapsed() >= CIRCUIT_OPEN_DURATION {
                    // Transition to half-open: allow one probe
                    self.state = CircuitState::HalfOpen;
                    true
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => {
                // Already probing — block additional concurrent requests
                false
            }
        }
    }

    /// Record a successful call.
    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.total_successes += 1;
        self.state = CircuitState::Closed;
    }

    /// Record a failed call.
    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        self.total_failures += 1;
        self.last_failure_time = Some(Instant::now());

        if self.consecutive_failures >= CIRCUIT_FAILURE_THRESHOLD {
            self.state = CircuitState::Open {
                since: Instant::now(),
            };
        }
    }

    fn is_open(&self) -> bool {
        matches!(self.state, CircuitState::Open { .. })
    }
}

/// Routes model requests to the correct provider, with automatic failover.
#[derive(Clone)]
pub struct ModelRouter {
    providers: Vec<Arc<dyn LlmProvider>>,
    /// Circuit breakers keyed by provider name.
    breakers: Arc<Mutex<HashMap<String, CircuitBreaker>>>,
}

/// Check if an error is transient and worth retrying.
fn is_retryable(err: &claw_core::ClawError) -> bool {
    match err {
        claw_core::ClawError::RateLimited { .. } => true,
        claw_core::ClawError::LlmProvider(msg) => {
            // Match HTTP status codes that are transient
            msg.starts_with("HTTP 429")
                || msg.starts_with("HTTP 500")
                || msg.starts_with("HTTP 502")
                || msg.starts_with("HTTP 503")
                || msg.starts_with("HTTP 529")
                || msg.contains("timed out")
                || msg.contains("connection reset")
                || msg.contains("connection closed")
                || msg.contains("overloaded")
        }
        _ => false,
    }
}

/// Extract retry-after hint from a RateLimited error (in seconds).
fn retry_after_hint(err: &claw_core::ClawError) -> Option<u64> {
    if let claw_core::ClawError::RateLimited { retry_after_secs } = err {
        Some(*retry_after_secs)
    } else {
        None
    }
}

impl Default for ModelRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelRouter {
    pub fn new() -> Self {
        Self {
            providers: vec![],
            breakers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a provider.
    pub fn add_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        let name = provider.name().to_string();
        info!(provider = %name, "registered LLM provider");
        self.breakers
            .lock()
            .entry(name)
            .or_insert_with(CircuitBreaker::new);
        self.providers.push(provider);
    }

    /// Check if a provider's circuit is currently open (tripped).
    fn is_available(&self, provider_name: &str) -> bool {
        let mut breakers = self.breakers.lock();
        if let Some(cb) = breakers.get_mut(provider_name) {
            cb.allow_request()
        } else {
            true
        }
    }

    /// Record a success for a provider.
    fn record_success(&self, provider_name: &str) {
        let mut breakers = self.breakers.lock();
        if let Some(cb) = breakers.get_mut(provider_name) {
            cb.record_success();
        }
    }

    /// Record a failure for a provider.
    fn record_failure(&self, provider_name: &str) {
        let mut breakers = self.breakers.lock();
        if let Some(cb) = breakers.get_mut(provider_name) {
            let was_open = cb.is_open();
            cb.record_failure();
            if !was_open && cb.is_open() {
                warn!(
                    provider = provider_name,
                    failures = cb.consecutive_failures,
                    "circuit breaker OPEN — provider disabled for {}s",
                    CIRCUIT_OPEN_DURATION.as_secs()
                );
            }
        }
    }

    /// Find the right provider for a model string like "anthropic/claude-opus-4-6".
    fn resolve(&self, model: &str) -> Option<(Arc<dyn LlmProvider>, String)> {
        // Format: "provider/model-name" or just "model-name" (try all providers)
        if let Some((prefix, model_name)) = model.split_once('/') {
            for p in &self.providers {
                if p.name().to_lowercase() == prefix.to_lowercase() {
                    return Some((Arc::clone(p), model_name.to_string()));
                }
            }
        }
        // Fallback: try each provider's model list
        for p in &self.providers {
            if p.models().iter().any(|m| m == model) {
                return Some((Arc::clone(p), model.to_string()));
            }
        }
        None
    }

    /// Complete a request, with retry on transient errors and failover to alternative providers.
    pub async fn complete(
        &self,
        request: &LlmRequest,
        fallback_model: Option<&str>,
    ) -> Result<LlmResponse> {
        // Try primary with retries (if circuit is closed)
        if let Some((provider, model_name)) = self.resolve(&request.model) {
            if self.is_available(provider.name()) {
                let mut req = request.clone();
                req.model = model_name;

                match self.complete_with_retry(&*provider, &req).await {
                    Ok(resp) => {
                        self.record_success(provider.name());
                        return Ok(resp);
                    }
                    Err(e) => {
                        self.record_failure(provider.name());
                        warn!(
                            provider = provider.name(),
                            error = %e,
                            "primary provider failed after retries, attempting failover"
                        );
                    }
                }
            } else {
                warn!(
                    provider = provider.name(),
                    "circuit breaker is OPEN — skipping to fallback"
                );
            }
        }

        // Try fallback with retries
        if let Some(fallback) = fallback_model
            && let Some((provider, model_name)) = self.resolve(fallback)
            && self.is_available(provider.name())
        {
            let mut req = request.clone();
            req.model = model_name;
            match self.complete_with_retry(&*provider, &req).await {
                Ok(resp) => {
                    self.record_success(provider.name());
                    return Ok(resp);
                }
                Err(e) => {
                    self.record_failure(provider.name());
                    return Err(e);
                }
            }
        }

        Err(claw_core::ClawError::ModelNotFound(request.model.clone()))
    }

    /// Stream a request with retry on transient errors and failover.
    pub async fn stream(
        &self,
        request: &LlmRequest,
        fallback_model: Option<&str>,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>> {
        // Try primary with retries (if circuit is closed)
        if let Some((provider, model_name)) = self.resolve(&request.model) {
            if self.is_available(provider.name()) {
                let mut req = request.clone();
                req.model = model_name;

                match self.stream_with_retry(&*provider, &req).await {
                    Ok(rx) => {
                        self.record_success(provider.name());
                        return Ok(rx);
                    }
                    Err(e) => {
                        self.record_failure(provider.name());
                        warn!(
                            provider = provider.name(),
                            error = %e,
                            "primary provider stream failed after retries, attempting failover"
                        );
                    }
                }
            } else {
                warn!(
                    provider = provider.name(),
                    "circuit breaker is OPEN — skipping stream to fallback"
                );
            }
        }

        // Try fallback with retries
        if let Some(fallback) = fallback_model
            && let Some((provider, model_name)) = self.resolve(fallback)
            && self.is_available(provider.name())
        {
            let mut req = request.clone();
            req.model = model_name;
            match self.stream_with_retry(&*provider, &req).await {
                Ok(rx) => {
                    self.record_success(provider.name());
                    return Ok(rx);
                }
                Err(e) => {
                    self.record_failure(provider.name());
                    return Err(e);
                }
            }
        }

        Err(claw_core::ClawError::ModelNotFound(request.model.clone()))
    }

    /// Retry a complete() call with exponential backoff on transient errors.
    async fn complete_with_retry(
        &self,
        provider: &dyn LlmProvider,
        request: &LlmRequest,
    ) -> Result<LlmResponse> {
        let mut last_err = None;

        for attempt in 0..=MAX_RETRIES {
            match provider.complete(request).await {
                Ok(resp) => return Ok(resp),
                Err(e) if is_retryable(&e) && attempt < MAX_RETRIES => {
                    let delay = retry_after_hint(&e)
                        .map(|s| s * 1000)
                        .unwrap_or(BASE_DELAY_MS * 2u64.pow(attempt));
                    warn!(
                        provider = provider.name(),
                        attempt = attempt + 1,
                        max = MAX_RETRIES,
                        delay_ms = delay,
                        error = %e,
                        "retrying after transient error"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }

        Err(last_err.unwrap())
    }

    /// Retry a stream() call with exponential backoff on transient errors.
    async fn stream_with_retry(
        &self,
        provider: &dyn LlmProvider,
        request: &LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>> {
        let mut last_err = None;

        for attempt in 0..=MAX_RETRIES {
            match provider.stream(request).await {
                Ok(rx) => return Ok(rx),
                Err(e) if is_retryable(&e) && attempt < MAX_RETRIES => {
                    let delay = retry_after_hint(&e)
                        .map(|s| s * 1000)
                        .unwrap_or(BASE_DELAY_MS * 2u64.pow(attempt));
                    warn!(
                        provider = provider.name(),
                        attempt = attempt + 1,
                        max = MAX_RETRIES,
                        delay_ms = delay,
                        error = %e,
                        "retrying stream after transient error"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }

        Err(last_err.unwrap())
    }
}
