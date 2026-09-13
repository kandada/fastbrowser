// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 多标签异步编排（tokio 原生：timeout + select 语义）。

use std::time::Duration;

use tokio::sync::broadcast;

use crate::engine::{EngineError, ErrorKind, PageEvent, Result, TabId};

/// 无事件时的复查上限（等价于旧轮询粒度，但事件会即时唤醒）。
const EVENT_BOUND: Duration = Duration::from_millis(50);

/// 在事件流上等待某个事件（针对未来事件；配合状态轮询使用可覆盖已错过的事件）。
pub async fn wait_for_event(
    rx: &mut broadcast::Receiver<(TabId, PageEvent)>,
    tab: Option<TabId>,
    timeout: Duration,
    mut pred: impl FnMut(&PageEvent) -> bool,
) -> Result<TabId> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remain = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remain.is_zero() {
            return Err(timeout_err("wait_for_event", timeout));
        }
        match tokio::time::timeout(remain, rx.recv()).await {
            Err(_) => return Err(timeout_err("wait_for_event", timeout)),
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue, // 慢消费者丢帧：跳过继续
            Ok(Err(_)) => return Err(closed_err("wait_for_event")),
            Ok(Ok((t, ev))) => {
                if let Some(want) = tab {
                    if t != want {
                        continue;
                    }
                }
                if pred(&ev) {
                    return Ok(t);
                }
            }
        }
    }
}

/// 任意一个目标标签页产生匹配事件即返回（tokio select 语义）。
pub async fn wait_any(
    rx: &mut broadcast::Receiver<(TabId, PageEvent)>,
    tabs: &[TabId],
    timeout: Duration,
    mut pred: impl FnMut(&PageEvent) -> bool,
) -> Result<TabId> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remain = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remain.is_zero() {
            return Err(timeout_err("wait_any", timeout));
        }
        match tokio::time::timeout(remain, rx.recv()).await {
            Err(_) => return Err(timeout_err("wait_any", timeout)),
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue, // 慢消费者丢帧：跳过继续
            Ok(Err(_)) => return Err(closed_err("wait_any")),
            Ok(Ok((t, ev))) => {
                if !tabs.contains(&t) {
                    continue;
                }
                if pred(&ev) {
                    return Ok(t);
                }
            }
        }
    }
}

/// 轮询式等待：周期性检查 `check`，直到满足或超时。适合"已发生状态"的等待。
pub async fn wait_until_poll<F, Fut>(check: F, timeout: Duration, what: &str) -> Result<()>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if check().await {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(EngineError::new(
                ErrorKind::Timeout,
                format!("{what} timed out after {timeout:?}"),
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// 事件驱动的异步等待（"先查当前、再等未来事件"）：每轮先执行 `check`（命中
/// 立即返回），未命中则订阅事件流等待一个该标签页事件；事件到来立即复查，无事件
/// 时按有界间隔兜底复查（兼容"状态变化但不产生事件"的场景，如定时器/动画）。
pub async fn wait_until_event<F, Fut>(
    rx: &mut broadcast::Receiver<(TabId, PageEvent)>,
    tab: TabId,
    timeout: Duration,
    what: &str,
    mut check: F,
) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool>>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if check().await? {
            return Ok(());
        }
        let remain = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remain.is_zero() {
            return Err(timeout_err(what, timeout));
        }
        match tokio::time::timeout(remain.min(EVENT_BOUND), rx.recv()).await {
            Ok(Ok((t, _))) if t == tab => {} // 事件到来 → 复查
            _ => {}                          // 超时 / Lagged / Closed → 复查（由 check 兜底）
        }
    }
}

fn timeout_err(what: &str, timeout: Duration) -> EngineError {
    EngineError::new(
        ErrorKind::Timeout,
        format!("{what} timed out after {timeout:?}"),
    )
}

fn closed_err(what: &str) -> EngineError {
    EngineError::new(ErrorKind::Internal, format!("{what}: event stream closed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    #[tokio::test]
    async fn wait_any_returns_first_matching_tab() {
        let (tx, _) = broadcast::channel(16);
        let mut rx = tx.subscribe();
        let t1 = TabId(1);
        let t2 = TabId(2);
        // 先派发一个 t1 的导航完成事件
        tx.send((
            t1,
            PageEvent::NavigationCompleted {
                url: "u".into(),
                status: 200,
            },
        ))
        .unwrap();
        let got = wait_any(&mut rx, &[t1, t2], Duration::from_secs(1), |e| {
            matches!(e, PageEvent::NavigationCompleted { .. })
        })
        .await
        .unwrap();
        assert_eq!(got, t1);
    }

    #[tokio::test]
    async fn wait_for_event_filters_tab_and_times_out() {
        let (tx, _) = broadcast::channel(16);
        let mut rx = tx.subscribe();
        let t1 = TabId(1);
        tx.send((
            t1,
            PageEvent::NavigationCompleted {
                url: "u".into(),
                status: 200,
            },
        ))
        .unwrap();
        // 指定错误标签 → 事件被过滤 → 超时
        let res =
            wait_for_event(&mut rx, Some(TabId(9)), Duration::from_millis(50), |_| true).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn wait_until_event_fast_path_and_timeout() {
        let (tx, _) = broadcast::channel(16);
        let mut rx = tx.subscribe();
        let tab = TabId(1);
        // 快路径：check 立即为真 → 一次即返回
        let mut calls = 0;
        wait_until_event(&mut rx, tab, Duration::from_secs(1), "fast", || {
            calls += 1;
            async move { Ok::<_, EngineError>(true) }
        })
        .await
        .unwrap();
        assert_eq!(calls, 1);
        // 超时：check 恒假 → Err
        let res = wait_until_event(&mut rx, tab, Duration::from_millis(30), "never", || async {
            Ok::<_, EngineError>(false)
        })
        .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn wait_until_event_wakes_on_tab_event() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let (tx, _) = broadcast::channel(16);
        let mut rx = tx.subscribe();
        let tab = TabId(1);
        let state = Arc::new(AtomicBool::new(false));
        let state_for_task = state.clone();
        let tx2 = tx.clone();
        // 后台任务 30ms 后置状态为真并投递一个该 tab 事件
        let h = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            state_for_task.store(true, Ordering::Relaxed);
            let _ = tx2.send((tab, PageEvent::DomChanged));
        });
        let start = tokio::time::Instant::now();
        wait_until_event(&mut rx, tab, Duration::from_secs(2), "evt", || {
            let s = state.clone();
            async move { Ok::<_, EngineError>(s.load(Ordering::Relaxed)) }
        })
        .await
        .unwrap();
        h.await.unwrap();
        assert!(state.load(Ordering::Relaxed));
        // 事件驱动：应远早于 2s 超时返回
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "should wake on event"
        );
    }

    #[tokio::test]
    async fn wait_until_poll_ok_and_timeout() {
        let n = std::rc::Rc::new(std::cell::Cell::new(0u32));
        wait_until_poll(
            || {
                let n = n.clone();
                async move {
                    let v = n.get() + 1;
                    n.set(v);
                    v >= 3
                }
            },
            Duration::from_secs(1),
            "poll",
        )
        .await
        .unwrap();
        assert!(n.get() >= 3);
        let res = wait_until_poll(|| async { false }, Duration::from_millis(30), "never").await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn wait_for_event_predicate_filters() {
        let (tx, _) = broadcast::channel(16);
        let mut rx = tx.subscribe();
        let t1 = TabId(1);
        // 先发一个 Dialog 事件，再发 NavigationCompleted
        tx.send((
            t1,
            PageEvent::Dialog {
                message: "x".into(),
                kind: "alert".into(),
            },
        ))
        .unwrap();
        tx.send((
            t1,
            PageEvent::NavigationCompleted {
                url: "u".into(),
                status: 200,
            },
        ))
        .unwrap();
        // 谓词只匹配 NavigationCompleted → 应命中并跳过 Dialog
        let got = wait_for_event(&mut rx, Some(t1), Duration::from_secs(1), |e| {
            matches!(e, PageEvent::NavigationCompleted { .. })
        })
        .await
        .unwrap();
        assert_eq!(got, t1);
    }

    #[tokio::test]
    async fn wait_any_ignores_non_target_tabs() {
        let (tx, _) = broadcast::channel(16);
        let mut rx = tx.subscribe();
        let t1 = TabId(1);
        let t2 = TabId(2);
        // 先发一个非目标标签 t3 的事件，再发目标 t2 的
        tx.send((
            TabId(3),
            PageEvent::NavigationCompleted {
                url: "u".into(),
                status: 200,
            },
        ))
        .unwrap();
        tx.send((
            t2,
            PageEvent::NavigationCompleted {
                url: "v".into(),
                status: 200,
            },
        ))
        .unwrap();
        let got = wait_any(&mut rx, &[t1, t2], Duration::from_secs(1), |_| true)
            .await
            .unwrap();
        assert_eq!(got, t2, "wait_any must skip non-target tabs");
    }

    #[tokio::test]
    async fn wait_for_event_never_matches_times_out() {
        let (tx, _) = broadcast::channel(16);
        let mut rx = tx.subscribe();
        let t1 = TabId(1);
        tx.send((
            t1,
            PageEvent::NavigationCompleted {
                url: "u".into(),
                status: 200,
            },
        ))
        .unwrap();
        let res = wait_for_event(&mut rx, Some(t1), Duration::from_millis(40), |e| {
            matches!(e, PageEvent::Dialog { .. })
        })
        .await;
        assert!(res.is_err());
        let msg = res.unwrap_err().to_string();
        assert!(msg.contains("timed out"), "timeout message: {msg}");
    }

    #[tokio::test]
    async fn wait_any_respects_timeout() {
        let (tx, _) = broadcast::channel(16);
        let mut rx = tx.subscribe();
        // 只发非匹配事件，目标谓词永不命中 → 超时
        tx.send((
            TabId(1),
            PageEvent::Console {
                level: crate::engine::ConsoleLevel::Log,
                message: "x".into(),
            },
        ))
        .unwrap();
        let res = wait_any(&mut rx, &[TabId(1)], Duration::from_millis(40), |e| {
            matches!(e, PageEvent::Dialog { .. })
        })
        .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn wait_for_event_works_after_lag_clears() {
        // broadcast 消费者慢时可能 Lagged；恢复后应继续工作
        let (tx, _) = broadcast::channel(4);
        let mut rx = tx.subscribe();
        let t1 = TabId(1);
        // 灌满再超发，触发 lag
        for i in 0..8 {
            let _ = tx.send((t1, PageEvent::DomChanged));
            let _ = i;
        }
        let got = wait_for_event(&mut rx, Some(t1), Duration::from_secs(1), |e| {
            matches!(e, PageEvent::NavigationCompleted { .. })
        })
        .await;
        // Lagged 恢复后没有更多事件 → 超时（不 panic、不阻塞）
        assert!(got.is_err());
    }
}
