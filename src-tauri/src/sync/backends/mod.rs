//! Sync backend implementations.
//!
//! v2 GA ships WebDAV + Git. The [`SyncBackend`](super::SyncBackend) trait
//! is intentionally minimal so additional backends (S3, plain HTTP) can land
//! post-GA without protocol churn.

pub mod git;
pub mod webdav;
