// Copyright (c) 2025 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see LICENSE file for full license terms.

//! Session and state management: cookie / storage / profile / manager.
//!
//! - cookie and storage persistence is implemented by the engine; this layer
//!   provides engine-agnostic models, CookieJar filtering, and persistence helpers.
//! - profile provides multi-account isolation config and tab-ownership mapping.

pub mod cookie;
pub mod manager;
pub mod profile;
pub mod session_store;
pub mod storage;

pub use cookie::CookieJar;
pub use manager::SessionManager;
pub use profile::{Profile, ProfileId};
pub use session_store::{SessionState, TabSessionState};
pub use storage::StorageKey;
