//! The C types of `include/rust_image_ffi.h`, field for field. `tests/abi_layout.rs` compiles the
//! header and holds both layouts to each other.
#![allow(non_camel_case_types)]

/// Moves with every change to the header that a compiled caller would notice.
pub const RIMG_ABI_VERSION: u32 = 4;

pub const RIMG_OK: i32 = 0;
pub const RIMG_ERR_INVALID_ARGUMENT: i32 = -1;
pub const RIMG_ERR_BUFFER_TOO_SMALL: i32 = -2;
pub const RIMG_ERR_NO_MEMORY: i32 = -3;
pub const RIMG_ERR_UNSUPPORTED: i32 = -4;
pub const RIMG_ERR_DECODE: i32 = -5;
pub const RIMG_ERR_LIMIT: i32 = -6;
pub const RIMG_ERR_ENCODE: i32 = -7;
pub const RIMG_ERR_PANIC: i32 = -99;

pub const RIMG_FORMAT_JPEG: u32 = 1;
pub const RIMG_FORMAT_PNG: u32 = 2;
pub const RIMG_FORMAT_WEBP: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct rimg_options {
    pub input_formats: u32,
    pub output_format: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_alloc_bytes: u64,
    pub jpeg_quality: u8,
    pub apply_orientation: u8,
    pub background: [u8; 3],
    pub jpeg_subsampling: u8,
    pub jpeg_quality_from_input: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct rimg_info {
    pub input_format: u32,
    pub width: u32,
    pub height: u32,
    pub has_alpha: u8,
    pub input_jpeg_quality: u8,
}

pub fn default_options() -> rimg_options {
    rimg_options {
        input_formats: RIMG_FORMAT_JPEG,
        output_format: RIMG_FORMAT_JPEG,
        max_width: 8192,
        max_height: 8192,
        max_pixels: 16_000_000,
        max_alloc_bytes: 128 * 1024 * 1024,
        jpeg_quality: 85,
        apply_orientation: 1,
        background: [255, 255, 255],
        jpeg_subsampling: 1,
        jpeg_quality_from_input: 1,
    }
}
