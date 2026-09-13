// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Minimal logging (no external deps): writes to stderr when `FASTBROWSER_LOG=1`.
//!
//! Complements the audit log (`audit`): audit records what happened, logs record runtime diagnostics.

/// Emit a log line (active when `FASTBROWSER_LOG=1`).
#[macro_export]
macro_rules! fb_log {
    ($($arg:tt)*) => {
        if std::env::var("FASTBROWSER_LOG").map(|v| v == "1").unwrap_or(false) {
            eprintln!("[fastbrowser] {}", format!($($arg)*));
        }
    };
}

/// Emit a log line unconditionally.
#[macro_export]
macro_rules! fb_log_always {
    ($($arg:tt)*) => {
        eprintln!("[fastbrowser] {}", format!($($arg)*));
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn log_macros_compile() {
        crate::fb_log!("engine mock initialized");
        crate::fb_log_always!("test {}", 1);
    }
}
