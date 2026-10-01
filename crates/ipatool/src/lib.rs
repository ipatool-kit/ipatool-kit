//! App Store IPA toolkit library (`ipatool-kit`).
//!
//! Native iTunes Search plus authenticated App Store ops (SAP signing via
//! `ipatool-kit-core`, purchase history via DAAP).
//!
//! # Library quick start
//!
//! ```no_run
//! use ipatool_kit::{AppStoreClient, HttpClient};
//!
//! let client = HttpClient::new().with_country("us");
//! let found = client.search("pages", 3, None).unwrap();
//! for app in found.results {
//!     println!("{} — {}", app.id, app.name);
//! }
//! ```
//!
//! Authenticated download / purchase history require an Apple ID session
//! (`store::login`, then `store::download` / `store::list_purchases`).
//!
//! # Install the TUI
//!
//! ```bash
//! cargo install ipatool-kit
//! ipatool-kit
//! ```

#![forbid(unsafe_code)]

pub mod client;
pub mod error;
pub mod helpers;
pub mod http;
pub mod plist;
pub mod purchases;
pub mod session;
pub mod store;

pub use client::{
    App, AppStoreClient, AuthInfo, DownloadRequest, LoginRequest, SearchResult, VersionInfo,
};
pub use error::{IpatoolError, Result};
pub use http::HttpClient;
pub use purchases::OwnedApp;
