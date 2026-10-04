//! image-rs behind a C interface.
//!
//! `include/rust_image_ffi.h` is the interface; this crate implements it. One job: decode a
//! picture nobody vouches for under hard limits and encode its pixels again, so that nothing of
//! the input's container gets through.
//!
//! Safe Rust ends at the `extern "C"` line. All `unsafe` lives in [`ffi`], the one module allowed
//! to have it; everything else is denied it.
#![deny(unsafe_code)]

pub mod abi;
#[allow(unsafe_code)]
pub mod ffi;
mod quality;
pub mod reencode;
