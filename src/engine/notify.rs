// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 事件驱动等待原语：`EventNotifier`（条件变量 + 代数计数器）与
//! `wait_until`（"先查当前、再等未来事件"的等待循环）。
//!
//! 目标：把 wait / actionability 里的 `std::thread::sleep(50ms)` 轮询换成
//! 事件通知——事件到来立即唤醒（低延迟 + 空闲不空转），无事件时按有界间隔
//! 复查兜底（兼容"状态变化但不产生事件"的场景，如定时器/动画）。

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use super::{BrowserEngine, EngineError, ErrorKind, Result, TabId};

/// 条件变量通知器：`notify` 递增代数并唤醒所有等待者；`wait_changed` 阻塞
/// 直到代数变化（有新事件）或超时。基于代数计数器，天然规避"检查后、等待前"
/// 之间丢失唤醒（lost-wakeup）的竞态。
pub struct EventNotifier {
    gen: Mutex<u64>,
    cvar: Condvar,
}

impl Default for EventNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl EventNotifier {
    pub fn new() -> Self {
        EventNotifier {
            gen: Mutex::new(0),
            cvar: Condvar::new(),
        }
    }

    /// 通知有新事件（代数 +1，唤醒所有等待者）。
    pub fn notify(&self) {
        let mut g = self.gen.lock().unwrap_or_else(|e| e.into_inner());
        *g = g.wrapping_add(1);
        // 持有锁时唤醒，避免与 wait_changed 的谓词检查竞态
        self.cvar.notify_all();
    }

    /// 当前代数。
    pub fn generation(&self) -> u64 {
        *self.gen.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 阻塞等待代数 != `since`，或超时。返回 true 表示期间有事件（代数已变化）。
    pub fn wait_changed(&self, since: u64, timeout: Duration) -> bool {
        let guard = self.gen.lock().unwrap_or_else(|e| e.into_inner());
        if *guard != since {
            return true;
        }
        let (_guard, res) = self
            .cvar
            .wait_timeout_while(guard, timeout, |g| *g == since)
            .unwrap_or_else(|e| e.into_inner());
        !res.timed_out()
    }
}

/// 无事件时的复查上限（等价于旧轮询的 50ms 粒度，但事件会即时唤醒）。
pub const EVENT_WAIT_BOUND: Duration = Duration::from_millis(50);

/// "先查当前、再等未来事件"的通用等待循环。
///
/// 每轮先执行 `check`（快速路径，命中立即返回）；未命中则阻塞等待新事件
/// （`engine.wait_event`），事件到来立即唤醒复查；无事件则在有界间隔后兜底
/// 复查。超时返回 `Timeout`。
pub fn wait_until<E: BrowserEngine + ?Sized>(
    engine: &E,
    tab: TabId,
    timeout: Duration,
    what: &str,
    mut check: impl FnMut() -> Result<bool>,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let mut gen = engine.event_generation(tab);
    loop {
        if check()? {
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(EngineError::new(
                ErrorKind::Timeout,
                format!("{what} timed out after {timeout:?}"),
            ));
        }
        let remaining = deadline - now;
        let (new_gen, _) = engine.wait_event(tab, gen, remaining.min(EVENT_WAIT_BOUND));
        gen = new_gen;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn notifier_wakes_waiter() {
        let n = Arc::new(EventNotifier::new());
        let g0 = n.generation();
        let n2 = n.clone();
        let h = thread::spawn(move || n2.notify());
        h.join().unwrap();
        // 通知已发生 → 用旧代数等待应立即返回 true，且代数已递增
        assert!(n.wait_changed(g0, Duration::from_secs(5)));
        assert_ne!(n.generation(), g0);
    }

    #[test]
    fn notifier_times_out() {
        let n = EventNotifier::new();
        let g0 = n.generation();
        let start = Instant::now();
        let woke = n.wait_changed(g0, Duration::from_millis(50));
        assert!(!woke);
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn notifier_no_lost_wakeup_when_notified_before_wait() {
        let n = EventNotifier::new();
        let g0 = n.generation();
        // 事件先于等待发生：用旧代数等待必须立即返回（不能卡到超时）
        n.notify();
        let start = Instant::now();
        let woke = n.wait_changed(g0, Duration::from_millis(500));
        assert!(woke, "wait must not block when generation already advanced");
        assert!(start.elapsed() < Duration::from_millis(400));
    }
}
