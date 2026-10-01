//! App Store IPA toolkit library.
//!
//! Native iTunes Search plus authenticated App Store ops (SAP signing via
//! `ipatool-core`, purchase history via DAAP).

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
