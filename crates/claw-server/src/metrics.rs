//! Prometheus-compatible metrics endpoint for the Claw server.
//!
//! Tracks request counts, latencies, token usage, and cost.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Global metrics registry.
#[derive(Debug, Clone)]
pub struct Metrics {
    inner: Arc<MetricsInner>,
}

#[derive(Debug)]
struct MetricsInner {
    /// Total HTTP requests served.
    pub http_requests_total: AtomicU64,
    /// Total HTTP errors (4xx + 5xx).
    pub http_errors_total: AtomicU64,
    /// Total chat messages processed.
    pub chat_messages_total: AtomicU64,
    /// Total streaming chat messages processed.
    pub chat_stream_messages_total: AtomicU64,
    /// Total LLM API calls.
    pub llm_calls_total: AtomicU64,
    /// Total LLM input tokens.
    pub llm_input_tokens_total: AtomicU64,
    /// Total LLM output tokens.
    pub llm_output_tokens_total: AtomicU64,
    /// Total estimated cost in micro-dollars (USD * 1_000_000).
    pub cost_microdollars_total: AtomicU64,
    /// Total tool calls executed.
    pub tool_calls_total: AtomicU64,
    /// Total tool errors.
    pub tool_errors_total: AtomicU64,
    /// Total approvals requested.
    pub approvals_requested_total: AtomicU64,
    /// Total approvals approved.
    pub approvals_approved_total: AtomicU64,
    /// Total approvals denied.
    pub approvals_denied_total: AtomicU64,
    /// Total prompt injections detected.
    pub injection_detections_total: AtomicU64,
    /// Total rate limit rejections.
    pub rate_limit_rejections_total: AtomicU64,
    /// Server start time for uptime calculation.
    pub started_at: Instant,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(MetricsInner {
                http_requests_total: AtomicU64::new(0),
                http_errors_total: AtomicU64::new(0),
                chat_messages_total: AtomicU64::new(0),
                chat_stream_messages_total: AtomicU64::new(0),
                llm_calls_total: AtomicU64::new(0),
                llm_input_tokens_total: AtomicU64::new(0),
                llm_output_tokens_total: AtomicU64::new(0),
                cost_microdollars_total: AtomicU64::new(0),
                tool_calls_total: AtomicU64::new(0),
                tool_errors_total: AtomicU64::new(0),
                approvals_requested_total: AtomicU64::new(0),
                approvals_approved_total: AtomicU64::new(0),
                approvals_denied_total: AtomicU64::new(0),
                injection_detections_total: AtomicU64::new(0),
                rate_limit_rejections_total: AtomicU64::new(0),
                started_at: Instant::now(),
            }),
        }
    }

    pub fn inc_http_requests(&self) {
        self.inner
            .http_requests_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_http_errors(&self) {
        self.inner.http_errors_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_chat_messages(&self) {
        self.inner
            .chat_messages_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_chat_stream_messages(&self) {
        self.inner
            .chat_stream_messages_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_llm_calls(&self) {
        self.inner.llm_calls_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_llm_tokens(&self, input: u32, output: u32) {
        self.inner
            .llm_input_tokens_total
            .fetch_add(input as u64, Ordering::Relaxed);
        self.inner
            .llm_output_tokens_total
            .fetch_add(output as u64, Ordering::Relaxed);
    }

    pub fn add_cost_usd(&self, cost: f64) {
        let microdollars = (cost * 1_000_000.0) as u64;
        self.inner
            .cost_microdollars_total
            .fetch_add(microdollars, Ordering::Relaxed);
    }

    pub fn inc_tool_calls(&self) {
        self.inner.tool_calls_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_tool_errors(&self) {
        self.inner.tool_errors_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_approvals_requested(&self) {
        self.inner
            .approvals_requested_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_approvals_approved(&self) {
        self.inner
            .approvals_approved_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_approvals_denied(&self) {
        self.inner
            .approvals_denied_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_injection_detections(&self) {
        self.inner
            .injection_detections_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_rate_limit_rejections(&self) {
        self.inner
            .rate_limit_rejections_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Render metrics in Prometheus text exposition format.
    pub fn render_prometheus(&self) -> String {
        let m = &self.inner;
        let uptime = m.started_at.elapsed().as_secs();
        let cost_usd = m.cost_microdollars_total.load(Ordering::Relaxed) as f64 / 1_000_000.0;

        format!(
            r#"# HELP claw_uptime_seconds Time since the server started.
# TYPE claw_uptime_seconds gauge
claw_uptime_seconds {}

# HELP claw_http_requests_total Total HTTP requests served.
# TYPE claw_http_requests_total counter
claw_http_requests_total {}

# HELP claw_http_errors_total Total HTTP errors (4xx/5xx).
# TYPE claw_http_errors_total counter
claw_http_errors_total {}

# HELP claw_chat_messages_total Total chat messages processed.
# TYPE claw_chat_messages_total counter
claw_chat_messages_total {}

# HELP claw_chat_stream_messages_total Total streaming chat messages processed.
# TYPE claw_chat_stream_messages_total counter
claw_chat_stream_messages_total {}

# HELP claw_llm_calls_total Total LLM API calls.
# TYPE claw_llm_calls_total counter
claw_llm_calls_total {}

# HELP claw_llm_input_tokens_total Total LLM input tokens.
# TYPE claw_llm_input_tokens_total counter
claw_llm_input_tokens_total {}

# HELP claw_llm_output_tokens_total Total LLM output tokens.
# TYPE claw_llm_output_tokens_total counter
claw_llm_output_tokens_total {}

# HELP claw_cost_usd_total Total estimated cost in USD.
# TYPE claw_cost_usd_total counter
claw_cost_usd_total {:.6}

# HELP claw_tool_calls_total Total tool calls executed.
# TYPE claw_tool_calls_total counter
claw_tool_calls_total {}

# HELP claw_tool_errors_total Total tool execution errors.
# TYPE claw_tool_errors_total counter
claw_tool_errors_total {}

# HELP claw_approvals_requested_total Total approval requests created.
# TYPE claw_approvals_requested_total counter
claw_approvals_requested_total {}

# HELP claw_approvals_approved_total Total approvals approved.
# TYPE claw_approvals_approved_total counter
claw_approvals_approved_total {}

# HELP claw_approvals_denied_total Total approvals denied.
# TYPE claw_approvals_denied_total counter
claw_approvals_denied_total {}

# HELP claw_injection_detections_total Total prompt injection attempts detected.
# TYPE claw_injection_detections_total counter
claw_injection_detections_total {}

# HELP claw_rate_limit_rejections_total Total rate limit rejections (429).
# TYPE claw_rate_limit_rejections_total counter
claw_rate_limit_rejections_total {}
"#,
            uptime,
            m.http_requests_total.load(Ordering::Relaxed),
            m.http_errors_total.load(Ordering::Relaxed),
            m.chat_messages_total.load(Ordering::Relaxed),
            m.chat_stream_messages_total.load(Ordering::Relaxed),
            m.llm_calls_total.load(Ordering::Relaxed),
            m.llm_input_tokens_total.load(Ordering::Relaxed),
            m.llm_output_tokens_total.load(Ordering::Relaxed),
            cost_usd,
            m.tool_calls_total.load(Ordering::Relaxed),
            m.tool_errors_total.load(Ordering::Relaxed),
            m.approvals_requested_total.load(Ordering::Relaxed),
            m.approvals_approved_total.load(Ordering::Relaxed),
            m.approvals_denied_total.load(Ordering::Relaxed),
            m.injection_detections_total.load(Ordering::Relaxed),
            m.rate_limit_rejections_total.load(Ordering::Relaxed),
        )
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_counter_increments() {
        let m = Metrics::new();
        m.inc_http_requests();
        m.inc_http_requests();
        m.inc_chat_messages();
        let output = m.render_prometheus();
        assert!(output.contains("claw_http_requests_total 2"));
        assert!(output.contains("claw_chat_messages_total 1"));
    }

    #[test]
    fn test_metrics_tokens() {
        let m = Metrics::new();
        m.add_llm_tokens(100, 50);
        m.add_llm_tokens(200, 100);
        let output = m.render_prometheus();
        assert!(output.contains("claw_llm_input_tokens_total 300"));
        assert!(output.contains("claw_llm_output_tokens_total 150"));
    }

    #[test]
    fn test_metrics_cost() {
        let m = Metrics::new();
        m.add_cost_usd(0.005);
        m.add_cost_usd(0.003);
        let output = m.render_prometheus();
        assert!(output.contains("claw_cost_usd_total 0.008"));
    }

    #[test]
    fn test_metrics_prometheus_format() {
        let m = Metrics::new();
        let output = m.render_prometheus();
        // Verify it has proper Prometheus format
        assert!(output.contains("# HELP claw_uptime_seconds"));
        assert!(output.contains("# TYPE claw_uptime_seconds gauge"));
        assert!(output.contains("# TYPE claw_http_requests_total counter"));
    }
}
