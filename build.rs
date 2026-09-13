// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

// build.rs — 预留：链接 CEF / 平台原生库的钩子。
//
// engine-cef 启用时，CEF 预编译库由 `cef` crate 的构建脚本处理；
// 未来需要额外的平台链接（如 macOS 框架）时在此追加。

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-env-changed=CEF_PATH");
}
