// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! fastbrowser Python 绑定（PyO3）。
//!
//! 薄封装：`Browser` 包装 `sdk::Fastbrowser`，把 JSON 结果以字符串返回，
//! 由上层 `python/fastbrowser/__init__.py` 转成 Python dict/list。
//! 阻塞操作（open/snapshot/screenshot 等）通过 `py.allow_threads` 释放 GIL。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

use fastbrowser::engine::{
    FrameStreamOptions, PageEvent, PageEventSink, RenderingMode, TabId, ViewFrame, ViewFrameSink,
    ViewHandle,
};
use fastbrowser::sdk::Fastbrowser;
use fastbrowser::Config;

fn value_json(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".to_string())
}

fn json_err(e: fastbrowser::EngineError) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

fn parse_config(config_json: Option<String>) -> PyResult<Config> {
    match config_json {
        Some(s) => serde_json::from_str::<Config>(&s)
            .map_err(|e| PyValueError::new_err(format!("invalid config: {e}"))),
        None => Ok(Config::default()),
    }
}

/// 页面事件回调 → Python callable（解耦派发）。
///
/// 引擎在持有 per-tab 锁时调用 `on_page_event`；这里**只入队**、立即返回，
/// 由独立派发线程在**锁外**拿 GIL 回调 Python。这样回调里重入浏览器（快照/
/// 工具调用）不会死锁。派发线程随 sink 析构自清理（不 join，避免持 GIL 等待）。
struct PyEventSink {
    tx: Mutex<Option<mpsc::SyncSender<(u32, String)>>>,
    shutdown: Arc<AtomicBool>,
}

impl PyEventSink {
    fn new(cb: Py<PyAny>) -> Self {
        let (tx, rx) = mpsc::sync_channel::<(u32, String)>(256);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown2 = shutdown.clone();
        std::thread::spawn(move || {
            for (tab, json) in rx {
                if shutdown2.load(Ordering::Relaxed) {
                    break;
                }
                Python::with_gil(|py| {
                    let _ = cb.bind(py).call1((tab, json));
                });
            }
        });
        PyEventSink {
            tx: Mutex::new(Some(tx)),
            shutdown,
        }
    }
}

impl PageEventSink for PyEventSink {
    fn on_page_event(&self, tab: TabId, event: &PageEvent) {
        // 只入队（非阻塞 try_send），不碰 Python，立即返回
        if let Some(tx) = self.tx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = tx.try_send((tab.as_u32(), event.to_json().to_string()));
        }
    }
}

impl Drop for PyEventSink {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        // 关闭通道唤醒派发线程使其退出；不 join（持有 GIL 时 join 会与派发线程抢 GIL 死锁）
        *self.tx.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// 离屏帧回调 → Python callable（解耦派发，同上）。
struct PyFrameSink {
    tx: Mutex<Option<mpsc::SyncSender<(u32, String)>>>,
    shutdown: Arc<AtomicBool>,
}

impl PyFrameSink {
    fn new(cb: Py<PyAny>) -> Self {
        let (tx, rx) = mpsc::sync_channel::<(u32, String)>(64);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown2 = shutdown.clone();
        std::thread::spawn(move || {
            for (tab, json) in rx {
                if shutdown2.load(Ordering::Relaxed) {
                    break;
                }
                Python::with_gil(|py| {
                    let _ = cb.bind(py).call1((tab, json));
                });
            }
        });
        PyFrameSink {
            tx: Mutex::new(Some(tx)),
            shutdown,
        }
    }
}

impl ViewFrameSink for PyFrameSink {
    fn on_view_frame(&self, tab: TabId, frame: &ViewFrame) {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&frame.rgba);
        let json = serde_json::json!({
            "width": frame.width,
            "height": frame.height,
            "seq": frame.seq,
            "base64": b64,
        })
        .to_string();
        if let Some(tx) = self.tx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let _ = tx.try_send((tab.as_u32(), json));
        }
    }
}

impl Drop for PyFrameSink {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        *self.tx.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

#[pyclass(name = "Browser")]
struct Browser {
    inner: Arc<Fastbrowser>,
}

#[pymethods]
impl Browser {
    #[new]
    fn new() -> Self {
        Browser {
            inner: Arc::new(Fastbrowser::new()),
        }
    }

    /// 初始化内核。`config_json` 为 `Config` 的 JSON 字符串（None 用默认，mock 引擎）。
    #[pyo3(signature = (config_json=None))]
    fn init(&self, py: Python<'_>, config_json: Option<String>) -> PyResult<String> {
        let cfg = parse_config(config_json)?;
        let inner = self.inner.clone();
        py.allow_threads(move || inner.init(cfg))
            .map(|_| "{\"ok\":true}".to_string())
            .map_err(json_err)
    }

    fn is_initialized(&self) -> bool {
        self.inner.is_initialized()
    }

    fn open(&self, py: Python<'_>, url: &str) -> PyResult<String> {
        let inner = self.inner.clone();
        let url = url.to_string();
        py.allow_threads(move || inner.open(&url))
            .map(|v| value_json(&v))
            .map_err(json_err)
    }

    fn navigate(&self, py: Python<'_>, url: &str) -> PyResult<String> {
        let inner = self.inner.clone();
        let url = url.to_string();
        py.allow_threads(move || inner.navigate(&url))
            .map(|v| value_json(&v))
            .map_err(json_err)
    }

    fn tool_call(&self, py: Python<'_>, name: &str, params_json: &str) -> PyResult<String> {
        let params: serde_json::Value = serde_json::from_str(params_json)
            .map_err(|e| PyRuntimeError::new_err(format!("invalid params: {e}")))?;
        let inner = self.inner.clone();
        let name = name.to_string();
        py.allow_threads(move || inner.tool_call(&name, params))
            .map(|v| value_json(&v))
            .map_err(json_err)
    }

    fn tool_list(&self) -> String {
        value_json(&self.inner.tool_list())
    }

    fn tool_count(&self) -> usize {
        self.inner.tool_count()
    }

    fn snapshot(&self, py: Python<'_>) -> PyResult<String> {
        let inner = self.inner.clone();
        py.allow_threads(move || inner.snapshot())
            .map(|s| serde_json::to_string(&s).unwrap_or_else(|_| "null".to_string()))
            .map_err(json_err)
    }

    /// 返回 `(width, height, rgba_bytes)`。
    fn screenshot(&self, py: Python<'_>) -> PyResult<(u32, u32, Vec<u8>)> {
        let inner = self.inner.clone();
        let img = py.allow_threads(move || inner.screenshot()).map_err(json_err)?;
        Ok((img.width, img.height, img.rgba))
    }

    fn set_viewport(&self, py: Python<'_>, width: u32, height: u32) -> PyResult<()> {
        let inner = self.inner.clone();
        py.allow_threads(move || inner.set_viewport(width, height)).map_err(json_err)
    }

    /// 当前标签页的宿主视图句柄（`{"kind": "native"|"osr", "handle": u64}`）。
    fn get_view(&self) -> Option<String> {
        self.inner.get_view().map(|vh| match vh {
            ViewHandle::Native(h) => format!("{{\"kind\":\"native\",\"handle\":{h}}}"),
            ViewHandle::Osr(h) => format!("{{\"kind\":\"osr\",\"handle\":{h}}}"),
        })
    }

    fn start_frame_stream(
        &self,
        py: Python<'_>,
        tab: u32,
        fps: u32,
        max_width: u32,
        max_height: u32,
    ) -> PyResult<String> {
        let inner = self.inner.clone();
        let opts = FrameStreamOptions {
            fps,
            max_width,
            max_height,
            format: "png".into(),
        };
        py.allow_threads(move || inner.start_frame_stream(TabId(tab), opts))
            .map(|v| value_json(&v))
            .map_err(json_err)
    }

    fn stop_frame_stream(&self, py: Python<'_>, tab: u32) -> PyResult<String> {
        let inner = self.inner.clone();
        py.allow_threads(move || inner.stop_frame_stream(TabId(tab)))
            .map(|v| value_json(&v))
            .map_err(json_err)
    }

    fn set_rendering_mode(&self, mode: &str) -> PyResult<()> {
        let m = match mode {
            "hosted" => RenderingMode::Hosted,
            "headless" => RenderingMode::Headless,
            other => {
                return Err(PyRuntimeError::new_err(format!(
                    "unknown rendering mode '{other}' (expected hosted|headless)"
                )))
            }
        };
        self.inner.set_rendering_mode(m);
        Ok(())
    }

    /// 注册页面事件回调 `callable(tab: int, event_json: str)`。
    /// 回调在独立派发线程上执行（锁外），可安全重入浏览器。
    fn register_event_callback(&self, cb: Py<PyAny>) {
        self.inner.register_event_sink(Arc::new(PyEventSink::new(cb)));
    }

    /// 注册离屏帧回调 `callable(tab: int, frame_json: str)`。
    /// 回调在独立派发线程上执行（锁外），可安全重入浏览器。
    fn register_frame_callback(&self, cb: Py<PyAny>) {
        self.inner.register_frame_sink(Arc::new(PyFrameSink::new(cb)));
    }

    fn status(&self) -> String {
        value_json(&self.inner.status())
    }

    fn audit(&self) -> String {
        value_json(&self.inner.audit())
    }

    fn clear_audit(&self) {
        self.inner.clear_audit();
    }

    fn get_info(&self) -> String {
        serde_json::to_string(&self.inner.get_info()).unwrap_or_else(|_| "{}".to_string())
    }

    fn session_save(&self, py: Python<'_>, path: &str) -> PyResult<String> {
        let inner = self.inner.clone();
        let path = path.to_string();
        py.allow_threads(move || inner.session_save(&path))
            .map(|v| value_json(&v))
            .map_err(json_err)
    }

    fn session_load(&self, py: Python<'_>, path: &str) -> PyResult<String> {
        let inner = self.inner.clone();
        let path = path.to_string();
        py.allow_threads(move || inner.session_load(&path))
            .map(|v| value_json(&v))
            .map_err(json_err)
    }

    fn clear_state(&self, py: Python<'_>) -> PyResult<String> {
        let inner = self.inner.clone();
        py.allow_threads(move || inner.clear_state())
            .map(|v| value_json(&v))
            .map_err(json_err)
    }

    fn shutdown(&self) {
        self.inner.shutdown();
    }
}

/// 内核版本号。
#[pyfunction]
fn version() -> &'static str {
    fastbrowser::VERSION
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Browser>()?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    Ok(())
}
