//! Session Report — 格式化缓存/性能报告

use std::time::Duration;

/// Session 报告数据
pub struct SessionReport {
    pub duration: Duration,
    pub turns: u64,
    pub l1_hit_rate: f64,
    pub l2_hit_rate: f64,
    pub prompt_cache_hits: u64,
    pub prompt_cache_tokens_saved: u64,
    pub estimated_cost_saved: f64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

impl SessionReport {
    /// 从 GlobalCache stats 构建
    pub fn from_cache(
        cache: &crate::core::cache::GlobalCache,
        duration: Duration,
        turns: u64,
        total_input: u64,
        total_output: u64,
    ) -> Self {
        let stats = cache.stats();
        let report = cache.report();
        // 估算节省: cache_read tokens 按 90% 折扣, prompt cache 按 90% 折扣
        let estimated_cost_saved = (stats.l1_hits as f64 * 0.001)
            + (report.prompt_cache_tokens_saved as f64 * 0.000002 * 0.9);

        Self {
            duration,
            turns,
            l1_hit_rate: stats.l1_hit_rate(),
            l2_hit_rate: stats.l2_hit_rate(),
            prompt_cache_hits: report.prompt_cache_hits,
            prompt_cache_tokens_saved: report.prompt_cache_tokens_saved(),
            estimated_cost_saved,
            total_input_tokens: total_input,
            total_output_tokens: total_output,
        }
    }
}

/// 打印人类可读的 Session 报告
pub fn print_session_report(report: &SessionReport) {
    eprintln!();
    eprintln!("┌─ Session Report ───────────────────────────┐");
    eprintln!("│ Duration:      {:>8.1}s                   │", report.duration.as_secs_f64());
    eprintln!("│ Turns:         {:>8}                       │", report.turns);
    eprintln!("│ Tokens:        {:>8} in / {:>6} out      │", report.total_input_tokens, report.total_output_tokens);
    eprintln!("│ Cache L1:      {:>6.1}% hit rate           │", report.l1_hit_rate * 100.0);
    eprintln!("│ Cache L2:      {:>6.1}% hit rate           │", report.l2_hit_rate * 100.0);
    eprintln!("│ Prompt cache:  {:>5} hits                  │", report.prompt_cache_hits);
    eprintln!("│ Tokens saved:  {:>8}                      │", report.prompt_cache_tokens_saved);
    eprintln!("│ Est. saved:    ${:.4}                      │", report.estimated_cost_saved);
    eprintln!("└────────────────────────────────────────────┘");
}
