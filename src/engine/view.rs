// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 视图 / 渲染相关类型：视口、矩形、视图句柄、离屏帧、图像。

use serde::{Deserialize, Serialize};

/// 视口（CSS 像素 / 缩放）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    /// 设备像素比（devicePixelRatio）。
    pub device_scale_factor: f64,
}

impl Viewport {
    pub fn new(width: u32, height: u32) -> Self {
        Viewport {
            width,
            height,
            device_scale_factor: 1.0,
        }
    }

    pub fn with_scale(mut self, s: f64) -> Self {
        self.device_scale_factor = s;
        self
    }
}

/// 元素矩形（CSS 像素，左上原点）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// 中心点。
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

/// 渲染模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RenderingMode {
    /// 托管模式：浏览器视图嵌入宿主窗口 / 宿主返回原生视图句柄。
    Hosted,
    /// 无头模式：离屏渲染，不创建窗口。
    Headless,
}

/// 平台原生视图句柄（供应用层嵌入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ViewHandle {
    /// 原生窗口/视图句柄（HWND / NSView / Android View token 等）。
    Native(u64),
    /// 离屏渲染句柄（OSR）。
    Osr(u64),
}

/// 位图（RGBA8）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        Image {
            width,
            height,
            rgba,
        }
    }

    pub fn pixel_count(&self) -> usize {
        (self.width as usize) * (self.height as usize)
    }

    /// 校验 buffer 长度是否为 width*height*4。
    pub fn is_valid(&self) -> bool {
        self.rgba.len() == self.pixel_count() * 4
    }

    /// 编码为 base64（PNG 依赖额外库，这里先提供原始 RGBA 的 base64）。
    pub fn to_base64(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&self.rgba)
    }

    /// 从 PNG 字节解码为 RGBA 位图（真实浏览器截图）。
    pub fn from_png(png: &[u8]) -> crate::engine::Result<Image> {
        let (w, h, rgba) = crate::png::decode_png(png)?;
        Ok(Image::new(w, h, rgba))
    }
}

/// 离屏渲染帧（带序号，供帧推送 / 预览）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// 帧序号，单调递增。
    pub seq: u64,
}

impl ViewFrame {
    pub fn from_image(image: &Image, seq: u64) -> Self {
        ViewFrame {
            width: image.width,
            height: image.height,
            rgba: image.rgba.clone(),
            seq,
        }
    }
}

/// 编码后的离屏帧（JPEG/PNG 字节直传，供 UI 预览）。
/// 相比 `ViewFrame` 的 RGBA 原始位图，JPEG 体积小 10~30 倍，
/// 避免「PNG 解码成 RGBA → base64 → IPC → 前端再解码」的重复开销。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodedViewFrame {
    pub width: u32,
    pub height: u32,
    /// MIME 类型："image/jpeg" / "image/png"。
    pub mime: String,
    /// 编码后字节（JPEG/PNG）。
    pub bytes: Vec<u8>,
    /// 帧序号，单调递增。
    pub seq: u64,
}

/// 帧推送流选项（`start_frame_stream`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameStreamOptions {
    /// 期望帧率上限（引擎按能力尽量逼近；预览窗建议 10，省 CPU）。
    pub fps: u32,
    /// 帧宽上限（0 = 视口原尺寸）。缩小可显著降低编码/传输开销。
    pub max_width: u32,
    /// 帧高上限（0 = 视口原尺寸）。
    pub max_height: u32,
    /// screencast 编码格式："png"（默认）或 "jpeg"（体积小，预览首选）。
    /// CDP 引擎原生支持；其它引擎可忽略。
    pub format: String,
}

impl Default for FrameStreamOptions {
    fn default() -> Self {
        FrameStreamOptions {
            fps: 10,
            max_width: 0,
            max_height: 0,
            format: "png".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_center_and_contains() {
        let r = Rect::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(r.center(), (60.0, 45.0));
        assert!(r.contains(60.0, 45.0));
        assert!(!r.contains(200.0, 200.0));
    }

    #[test]
    fn image_validity_and_base64() {
        let img = Image::new(2, 2, vec![0u8; 16]);
        assert!(img.is_valid());
        assert_eq!(img.to_base64(), "AAAAAAAAAAAAAAAAAAAAAA==");
        let bad = Image::new(2, 2, vec![0u8; 10]);
        assert!(!bad.is_valid());
    }

    #[test]
    fn viewport_scale() {
        let vp = Viewport::new(800, 600).with_scale(2.0);
        assert_eq!(vp.device_scale_factor, 2.0);
    }
}
