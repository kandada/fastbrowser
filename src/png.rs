// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! 极简 PNG 解码器（仅 8-bit、非交织 PNG，输出 RGBA8）。
//!
//! 依赖 `flate2` 做 IDAT 的 zlib 解压，其余（分块解析、去滤波、色彩转换）
//! 手工实现，避免引入 `image` 等重依赖。真实浏览器截图（`Page.captureScreenshot`
//! 的 PNG 输出）满足本解码器的输入约束；webview 引擎由宿主返回 RGBA，无需本模块。

use std::io::Read;

use crate::engine::{EngineError, ErrorKind, Result};

/// PNG 签名。
const SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// 解码 PNG 为 RGBA8 位图。返回 `(width, height, rgba)`。
pub fn decode_png(data: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    if data.len() < SIGNATURE.len() || data[..8] != SIGNATURE {
        return Err(err("invalid PNG signature"));
    }
    let mut pos = 8usize;
    let mut width: u32 = 0;
    let mut height: u32 = 0;
    let mut bit_depth: u8 = 0;
    let mut color_type: u8 = 0;
    let mut interlace: u8 = 0;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();
    let mut seen_ihdr = false;

    while pos + 8 <= data.len() {
        let len = read_be32(data, pos)? as usize;
        let pos_type = pos + 4;
        let pos_data = pos + 8;
        let chunk_type = &data[pos_type..pos_data];
        let pos_end = pos_data
            .checked_add(len)
            .and_then(|p| p.checked_add(4))
            .filter(|p| *p <= data.len())
            .ok_or_else(|| err("truncated PNG chunk"))?;
        let chunk_data = &data[pos_data..pos_data + len];

        match chunk_type {
            b"IHDR" => {
                if len < 13 {
                    return Err(err("IHDR too short"));
                }
                width = read_be32(chunk_data, 0)?;
                height = read_be32(chunk_data, 4)?;
                bit_depth = chunk_data[8];
                color_type = chunk_data[9];
                // compression = chunk_data[10]; filter = chunk_data[11];
                interlace = chunk_data[12];
                seen_ihdr = true;
            }
            b"PLTE" => {
                if !len.is_multiple_of(3) {
                    return Err(err("bad PLTE length"));
                }
                palette = chunk_data
                    .chunks_exact(3)
                    .map(|c| [c[0], c[1], c[2]])
                    .collect();
            }
            b"IDAT" => idat.extend_from_slice(chunk_data),
            b"IEND" => break,
            _ => {}
        }
        pos = pos_end;
    }

    if !seen_ihdr {
        return Err(err("missing IHDR"));
    }
    if bit_depth != 8 {
        return Err(err(format!("unsupported bit depth {bit_depth} (only 8)")));
    }
    if interlace != 0 {
        return Err(err(format!("unsupported interlace {interlace}")));
    }
    if width == 0 || height == 0 {
        return Err(err("empty image"));
    }
    let bpp = bytes_per_pixel(color_type)
        .ok_or_else(|| err(format!("unsupported color type {color_type}")))?;
    if color_type == 3 && palette.is_empty() {
        return Err(err("indexed PNG without PLTE"));
    }

    // 解压 IDAT（zlib 流）。
    let raw = inflate(&idat)?;
    let stride = (width as usize) * bpp;
    let expected = (height as usize) * (stride + 1);
    if raw.len() < expected {
        return Err(err(format!(
            "inflated data too short: {} < {expected}",
            raw.len()
        )));
    }

    // 去滤波 → 每行像素字节。
    let mut unfiltered = vec![0u8; height as usize * stride];
    {
        let mut prev = vec![0u8; stride];
        for y in 0..height as usize {
            let row_start = y * (stride + 1);
            let filter = raw[row_start];
            let row = &raw[row_start + 1..row_start + 1 + stride];
            let out = &mut unfiltered[y * stride..(y + 1) * stride];
            unfilter_row(filter, row, &prev, out, bpp);
            prev.copy_from_slice(out);
        }
    }

    // 色彩转换 → RGBA。
    let rgba = to_rgba(
        &unfiltered,
        width as usize,
        height as usize,
        color_type,
        &palette,
    )?;
    Ok((width, height, rgba))
}

fn err<T: Into<String>>(m: T) -> EngineError {
    EngineError::new(ErrorKind::View, format!("png decode: {}", m.into()))
}

/// 从 PNG 字节解析宽高（仅需签名 + IHDR，不解码整图）。非 PNG 返回 None。
pub fn png_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 8 + 8 + 13 || data[..8] != SIGNATURE {
        return None;
    }
    // 第一个 chunk：length(4) + "IHDR"(4) + width(4) + height(4)
    if &data[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

/// 从 JPEG 字节解析宽高（扫描 SOF 段，不做整图解码）。非 JPEG / 未找到返回 None。
pub fn jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 3 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        // 跳过 FF 填充（FF FF ...）
        let mut marker = i + 1;
        while marker < data.len() && data[marker] == 0xFF {
            marker += 1;
        }
        if marker >= data.len() {
            return None;
        }
        let code = data[marker];
        // 无长度段的标记
        if code == 0xD8 || code == 0xD9 || code == 0x01 {
            i = marker + 1;
            continue;
        }
        if marker + 2 >= data.len() {
            return None;
        }
        let seg_len = ((data[marker + 1] as usize) << 8) | data[marker + 2] as usize;
        if seg_len < 2 {
            return None;
        }
        // SOF0..SOF15（基线/扩展/渐进/无损）携带精度(1)+高(2)+宽(2)
        let is_sof = (0xC0..=0xC3).contains(&code)
            || (0xC5..=0xC7).contains(&code)
            || (0xC9..=0xCB).contains(&code)
            || (0xCD..=0xCF).contains(&code);
        if is_sof {
            if marker + 7 >= data.len() {
                return None;
            }
            let h = ((data[marker + 4] as u32) << 8) | data[marker + 5] as u32;
            let w = ((data[marker + 6] as u32) << 8) | data[marker + 7] as u32;
            if w > 0 && h > 0 {
                return Some((w, h));
            }
            return None;
        }
        i = marker + 1 + seg_len;
    }
    None
}

fn read_be32(b: &[u8], at: usize) -> Result<u32> {
    if at + 4 > b.len() {
        return Err(err("read past end"));
    }
    Ok(((b[at] as u32) << 24)
        | ((b[at + 1] as u32) << 16)
        | ((b[at + 2] as u32) << 8)
        | (b[at + 3] as u32))
}

fn bytes_per_pixel(color_type: u8) -> Option<usize> {
    match color_type {
        0 => Some(1),
        2 => Some(3),
        3 => Some(1),
        4 => Some(2),
        6 => Some(4),
        _ => None,
    }
}

fn inflate(idat: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut decoder = flate2::read::ZlibDecoder::new(idat);
    decoder
        .read_to_end(&mut out)
        .map_err(|e| err(format!("inflate: {e}")))?;
    Ok(out)
}

fn paeth(a: i32, b: i32, c: i32) -> u8 {
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

fn unfilter_row(filter: u8, row: &[u8], prev: &[u8], out: &mut [u8], bpp: usize) {
    match filter {
        0 => out.copy_from_slice(row),
        1 => {
            for i in 0..out.len() {
                let left = if i >= bpp { out[i - bpp] as i32 } else { 0 };
                out[i] = (row[i] as i32 + left) as u8;
            }
        }
        2 => {
            for i in 0..out.len() {
                out[i] = (row[i] as i32 + prev[i] as i32) as u8;
            }
        }
        3 => {
            for i in 0..out.len() {
                let left = if i >= bpp { out[i - bpp] as i32 } else { 0 };
                let up = prev[i] as i32;
                out[i] = (row[i] as i32 + ((left + up) / 2)) as u8;
            }
        }
        4 => {
            for i in 0..out.len() {
                let left = if i >= bpp { out[i - bpp] as i32 } else { 0 };
                let up = prev[i] as i32;
                let up_left = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
                out[i] = (row[i] as i32 + paeth(left, up, up_left) as i32) as u8;
            }
        }
        _ => {}
    }
}

fn to_rgba(
    pixels: &[u8],
    w: usize,
    h: usize,
    color_type: u8,
    palette: &[[u8; 3]],
) -> Result<Vec<u8>> {
    let mut out = vec![0u8; w * h * 4];
    let mut o = 0;
    match color_type {
        0 => {
            for &g in pixels {
                out[o..o + 3].copy_from_slice(&[g, g, g]);
                out[o + 3] = 255;
                o += 4;
            }
        }
        2 => {
            for px in pixels.chunks_exact(3) {
                out[o..o + 3].copy_from_slice(px);
                out[o + 3] = 255;
                o += 4;
            }
        }
        3 => {
            for &idx in pixels {
                let (r, g, b) = match palette.get(idx as usize) {
                    Some(c) => (c[0], c[1], c[2]),
                    None => (0, 0, 0),
                };
                out[o] = r;
                out[o + 1] = g;
                out[o + 2] = b;
                out[o + 3] = 255;
                o += 4;
            }
        }
        4 => {
            for px in pixels.chunks_exact(2) {
                out[o] = px[0];
                out[o + 1] = px[0];
                out[o + 2] = px[0];
                out[o + 3] = px[1];
                o += 4;
            }
        }
        6 => {
            out.copy_from_slice(pixels);
        }
        _ => return Err(err("unsupported color type in convert")),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成一张 2x2 的 8-bit RGBA 非交织 PNG（手工构造，用于解码冒烟测试）。
    fn make_png() -> Vec<u8> {
        use std::io::Write;
        let w: u32 = 2;
        let h: u32 = 2;
        // 原始像素：RGBA8, 4 个像素
        let raw: Vec<u8> = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        // 加滤波字节（全部 filter=0 None）
        let mut scan: Vec<u8> = Vec::new();
        for row in 0..h as usize {
            scan.push(0u8);
            scan.extend_from_slice(&raw[row * (w as usize) * 4..(row + 1) * (w as usize) * 4]);
        }
        let compressed = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut enc = compressed;
        enc.write_all(&scan).unwrap();
        let idat = enc.finish().unwrap();

        let mut png = Vec::new();
        png.extend_from_slice(&SIGNATURE);
        push_chunk(&mut png, b"IHDR", &{
            let mut d = Vec::new();
            d.extend_from_slice(&2u32.to_be_bytes());
            d.extend_from_slice(&2u32.to_be_bytes());
            d.push(8); // bit depth
            d.push(6); // color type RGBA
            d.push(0); // compression
            d.push(0); // filter
            d.push(0); // interlace
            d
        });
        push_chunk(&mut png, b"IDAT", &idat);
        push_chunk(&mut png, b"IEND", &[]);
        png
    }

    fn push_chunk(png: &mut Vec<u8>, ctype: &[u8; 4], data: &[u8]) {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        png.extend_from_slice(ctype);
        png.extend_from_slice(data);
        png.extend_from_slice(&[0u8; 4]); // crc 占位（解码端不校验）
    }

    #[test]
    fn decode_rgba_png() {
        let png = make_png();
        let (w, h, rgba) = decode_png(&png).unwrap();
        assert_eq!(w, 2);
        assert_eq!(h, 2);
        assert_eq!(rgba.len(), 16);
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&rgba[4..8], &[0, 255, 0, 255]);
        assert_eq!(&rgba[8..12], &[0, 0, 255, 255]);
        assert_eq!(&rgba[12..16], &[255, 255, 255, 255]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(decode_png(b"not a png").is_err());
        let mut png = make_png();
        png[0] = 0x00;
        assert!(decode_png(&png).is_err());
    }

    #[test]
    fn png_dimensions_from_header() {
        let png = make_png();
        assert_eq!(png_dimensions(&png), Some((2, 2)));
        assert_eq!(png_dimensions(b"not png"), None);
        // 截断头部 → None
        assert_eq!(png_dimensions(&png[..20]), None);
    }

    /// 手工构造一个最小 JPEG（仅 FFD8 + SOF0 段 + FFD9），含 640x480。
    fn minimal_jpeg() -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8]; // SOI
                                      // SOF0 段: FF C0  len(2)  precision(1)  height(2)  width(2)
        let sof_len: u16 = 11;
        b.extend_from_slice(&[0xFF, 0xC0]);
        b.extend_from_slice(&sof_len.to_be_bytes());
        b.push(8); // precision
        b.extend_from_slice(&480u16.to_be_bytes());
        b.extend_from_slice(&640u16.to_be_bytes());
        b.extend_from_slice(&[0x03]); // components
        b.extend_from_slice(&[0xFF, 0xD9]); // EOI
        b
    }

    #[test]
    fn jpeg_dimensions_from_sof() {
        let jpg = minimal_jpeg();
        assert_eq!(jpeg_dimensions(&jpg), Some((640, 480)));
        assert_eq!(jpeg_dimensions(b"not jpeg"), None);
        // 前有其它段（如 APP0）也能扫到 SOF
        let mut with_app = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        with_app.extend_from_slice(&jpg[2..]);
        assert_eq!(jpeg_dimensions(&with_app), Some((640, 480)));
        // 截断 → None
        assert_eq!(jpeg_dimensions(&jpg[..8]), None);
    }
}
