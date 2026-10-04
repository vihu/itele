//! itele: a native IPTV player with Xtream Codes support.
//!
//! The library holds everything that does not need a window: the provider
//! client, the provider settings and cache, the programme guide, the watch
//! history, favorites, and the user's settings. The `itele` binary wires it to Slint and libmpv.

#![deny(unsafe_code, missing_docs, rustdoc::broken_intra_doc_links)]

pub mod epg;
pub mod favorites;
pub mod history;
pub mod provider;
pub mod settings;
pub mod xtream;
