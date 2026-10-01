//! App Store IPA toolkit library.
//!
//! Pure helpers (crypto, plist, storefront, SAP wire helpers) + [`HttpClient`]
//! for public iTunes Search. Authenticated App Store ops require native SAP
//! signing (stage 2) and currently return [`IpatoolError::NotImplemented`].

#![forbid(unsafe_code)]

pub mod client;
pub mod crypto;
pub mod device;
pub mod error;
pub mod helpers;
pub mod http;
pub mod hwid;
pub mod plist;
pub mod sap;

pub use client::{
    App, AppStoreClient, AuthInfo, DownloadRequest, LoginRequest, SearchResult, VersionInfo,
};
pub use error::{IpatoolError, Result};
pub use http::HttpClient;
