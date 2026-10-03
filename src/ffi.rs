//! The `extern "C"` functions. This is the only module allowed `unsafe`: it turns the caller's
//! pointers and lengths into Rust values, and nothing else in the crate ever sees a raw pointer.
//!
//! Every function catches panics and answers `RIMG_ERR_PANIC` instead of unwinding into C.

use std::os::raw::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::abi::*;
use crate::reencode::{self, Config};

fn guard(f: impl FnOnce() -> i32) -> i32 {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(RIMG_ERR_PANIC)
}

unsafe fn bytes<'a>(p: *const u8, len: usize) -> Result<&'a [u8], i32> {
    if len == 0 {
        return Ok(&[]);
    }
    if p.is_null() {
        return Err(RIMG_ERR_INVALID_ARGUMENT);
    }
    // SAFETY: the caller passes `len` readable bytes that live for the call.
    Ok(unsafe { std::slice::from_raw_parts(p, len) })
}

/// The defaults, overlaid with as many bytes as the caller's struct has: an older caller's
/// shorter struct leaves the fields it does not know at their defaults.
unsafe fn options(opt: *const rimg_options) -> Result<rimg_options, i32> {
    let mut o = default_options();
    if opt.is_null() {
        return Ok(o);
    }
    // SAFETY: every rimg_options starts with its own size, and the caller owns at least that.
    let size = unsafe { opt.cast::<u32>().read_unaligned() } as usize;
    if size < size_of::<u32>() {
        return Err(RIMG_ERR_INVALID_ARGUMENT);
    }
    let size = size.min(size_of::<rimg_options>());
    // SAFETY: `size` bytes are readable by the line above and fit `o`; every field is a plain
    // integer, so any bytes are a valid value.
    unsafe { std::ptr::copy_nonoverlapping(opt.cast::<u8>(), (&raw mut o).cast::<u8>(), size) };
    Ok(o)
}

#[unsafe(no_mangle)]
pub extern "C" fn rimg_abi_version() -> u32 {
    RIMG_ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn rimg_status_string(status: i32) -> *const c_char {
    let name: &'static std::ffi::CStr = match status {
        RIMG_OK => c"ok",
        RIMG_ERR_INVALID_ARGUMENT => c"invalid argument",
        RIMG_ERR_BUFFER_TOO_SMALL => c"output buffer too small",
        RIMG_ERR_NO_MEMORY => c"out of memory",
        RIMG_ERR_UNSUPPORTED => c"unsupported or not allowed image format",
        RIMG_ERR_DECODE => c"not a valid image",
        RIMG_ERR_LIMIT => c"image exceeds the limits",
        RIMG_ERR_ENCODE => c"encoding failed",
        RIMG_ERR_PANIC => c"internal error",
        _ => c"unknown status",
    };
    name.as_ptr()
}

/// # Safety
/// `opt` is null or points to a writable `rimg_options`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rimg_options_default(opt: *mut rimg_options) {
    if !opt.is_null() {
        // SAFETY: checked for null; the caller owns the struct.
        unsafe { opt.write(default_options()) };
    }
}

/// # Safety
/// `input` points to `in_len` readable bytes, `info` to a writable `rimg_info`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rimg_probe(input: *const u8, in_len: usize, info: *mut rimg_info) -> i32 {
    guard(|| {
        if info.is_null() {
            return RIMG_ERR_INVALID_ARGUMENT;
        }
        let input = match unsafe { bytes(input, in_len) } {
            Ok(input) => input,
            Err(status) => return status,
        };
        match reencode::probe(input) {
            Ok(found) => {
                // SAFETY: checked for null; the caller owns the struct.
                unsafe { info.write(found) };
                RIMG_OK
            }
            Err(e) => e.status(),
        }
    })
}

/// # Safety
/// `opt` is null or points to a `rimg_options` of `struct_size` bytes, `input` to `in_len`
/// readable bytes, `out` to `out_cap` writable bytes, `out_len` to a writable `size_t`, and `info`
/// is null or points to a writable `rimg_info`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rimg_reencode(
    opt: *const rimg_options,
    input: *const u8,
    in_len: usize,
    out: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
    info: *mut rimg_info,
) -> i32 {
    guard(|| {
        if out_len.is_null() {
            return RIMG_ERR_INVALID_ARGUMENT;
        }
        // SAFETY: checked for null; the caller owns it.
        unsafe { out_len.write(0) };
        if out.is_null() && out_cap != 0 {
            return RIMG_ERR_INVALID_ARGUMENT;
        }
        let run = || -> Result<(Vec<u8>, rimg_info), i32> {
            let cfg = Config::from_options(&unsafe { options(opt)? }).map_err(|e| e.status())?;
            reencode::reencode(&cfg, unsafe { bytes(input, in_len)? }).map_err(|e| e.status())
        };
        let (encoded, found) = match run() {
            Ok(done) => done,
            Err(status) => return status,
        };
        // From here on `input` is no longer read, which is what lets `out` overlap it.
        // SAFETY: both checked for null; the caller owns them.
        unsafe {
            out_len.write(encoded.len());
            if !info.is_null() {
                info.write(found);
            }
        }
        if encoded.len() > out_cap {
            return RIMG_ERR_BUFFER_TOO_SMALL;
        }
        // SAFETY: `out` holds `out_cap` writable bytes and `encoded` is this function's own.
        unsafe { std::ptr::copy_nonoverlapping(encoded.as_ptr(), out, encoded.len()) };
        RIMG_OK
    })
}
