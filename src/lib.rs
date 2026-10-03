//! itele: a native IPTV player with Xtream Codes support.
//!
//! The library holds everything that does not need a window: the provider
//! client and, later, the cache. The `itele` binary wires it to Slint and
//! libmpv.

#![deny(unsafe_code, missing_docs, rustdoc::broken_intra_doc_links)]

pub mod provider;
pub mod xtream;
