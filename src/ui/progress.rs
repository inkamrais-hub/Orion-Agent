//! 进度指示器
//!
//! 终端旋转进度条 + 状态消息

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// 终端进度指示器
pub struct ProgressBar {
    message: String,
    spinner_idx: usize,
    start_time: std::time::Instant,
    finished: AtomicBool,
}

impl ProgressBar {
    pub fn new(msg: &str) -> Self {
        let pb = Self {
            message: msg.to_string(),
            spinner_idx: 0,
            start_time: std::time::Instant::now(),
            finished: AtomicBool::new(false),
        };
        pb.render();
        pb
    }

    fn render(&self) {
        let frame = SPINNER_FRAMES[self.spinner_idx % SPINNER_FRAMES.len()];
        let elapsed = self.start_time.elapsed().as_secs();
        eprint!("\r{} {} ({}s)  ", frame, self.message, elapsed);
        let _ = io::stderr().flush();
    }

    /// 旋转动画
    pub fn tick(&mut self) {
        self.spinner_idx = self.spinner_idx.wrapping_add(1);
        self.render();
    }

    /// 更新消息
    pub fn set_message(&mut self, msg: &str) {
        self.message = msg.to_string();
        self.render();
    }

    /// 完成
    pub fn finish(&self) {
        if self.finished.swap(true, Ordering::SeqCst) {
            return; // 已经 finish 过了
        }
        let elapsed = self.start_time.elapsed().as_millis();
        eprint!("\r✓ {} ({}ms)\n", self.message, elapsed);
        let _ = io::stderr().flush();
    }

    /// 完成并标记失败
    pub fn finish_with_error(&self, err: &str) {
        if self.finished.swap(true, Ordering::SeqCst) {
            return; // 已经 finish 过了
        }
        let elapsed = self.start_time.elapsed().as_millis();
        eprint!("\r✗ {} — {} ({}ms)\n", self.message, err, elapsed);
        let _ = io::stderr().flush();
    }
}

impl Drop for ProgressBar {
    fn drop(&mut self) {
        // finish() 内部的 AtomicBool::swap 保证不会重复输出
        self.finish();
    }
}

/// 简单的步骤进度 (用于编排器)
pub struct StepProgress {
    total: usize,
    current: usize,
    start_time: std::time::Instant,
}

impl StepProgress {
    pub fn new(total: usize) -> Self {
        eprintln!("📋 {} tasks to execute", total);
        Self { total, current: 0, start_time: std::time::Instant::now() }
    }

    pub fn step(&mut self, msg: &str) {
        if self.current < self.total {
            self.current += 1;
        }
        eprint!("\r[{}/{}] {}  ", self.current, self.total, msg);
        let _ = io::stderr().flush();
    }

    pub fn finish(&self) {
        let elapsed = self.start_time.elapsed().as_millis();
        eprint!("\r✓ {}/{} tasks completed ({}ms)\n", self.current, self.total, elapsed);
        let _ = io::stderr().flush();
    }
}
