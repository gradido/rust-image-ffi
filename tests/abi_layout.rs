//! The header and the Rust definitions describe the same bytes. The C compiler lays the header
//! out, and every size and offset it reports has to match `abi.rs`.
//!
//! Needs a C compiler: `$CC`, else `cc`. Without one the test says so and passes, because the
//! layout it guards cannot have changed without one either on the machine that changed it.

use std::collections::HashMap;
use std::mem::{offset_of, size_of};
use std::process::Command;

use rust_image_ffi::abi::*;

fn c_layout() -> Option<HashMap<String, usize>> {
    let root = env!("CARGO_MANIFEST_DIR");
    let out = std::env::temp_dir().join(format!("rimg_layout_{}", std::process::id()));
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let status = Command::new(&cc)
        .args(["-std=c11", "-Wall", "-Werror", "-I"])
        .arg(format!("{root}/include"))
        .arg(format!("{root}/tests/c/layout.c"))
        .arg("-o")
        .arg(&out)
        .status()
        .ok()?;
    assert!(status.success(), "{cc} could not compile tests/c/layout.c");
    let output = Command::new(&out).output().expect("run layout");
    let _ = std::fs::remove_file(&out);
    Some(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| {
                let (name, value) = line.split_once(' ').unwrap();
                (name.to_owned(), value.parse().unwrap())
            })
            .collect(),
    )
}

#[test]
fn rust_and_c_agree_on_every_layout() {
    let Some(c) = c_layout() else {
        eprintln!("no C compiler found; layout not checked");
        return;
    };
    let rust: Vec<(&str, usize)> = vec![
        ("sizeof.rimg_options", size_of::<rimg_options>()),
        (
            "offsetof.rimg_options.input_formats",
            offset_of!(rimg_options, input_formats),
        ),
        (
            "offsetof.rimg_options.output_format",
            offset_of!(rimg_options, output_format),
        ),
        (
            "offsetof.rimg_options.max_width",
            offset_of!(rimg_options, max_width),
        ),
        (
            "offsetof.rimg_options.max_height",
            offset_of!(rimg_options, max_height),
        ),
        (
            "offsetof.rimg_options.max_pixels",
            offset_of!(rimg_options, max_pixels),
        ),
        (
            "offsetof.rimg_options.max_alloc_bytes",
            offset_of!(rimg_options, max_alloc_bytes),
        ),
        (
            "offsetof.rimg_options.jpeg_quality",
            offset_of!(rimg_options, jpeg_quality),
        ),
        (
            "offsetof.rimg_options.apply_orientation",
            offset_of!(rimg_options, apply_orientation),
        ),
        (
            "offsetof.rimg_options.background",
            offset_of!(rimg_options, background),
        ),
        (
            "offsetof.rimg_options.jpeg_subsampling",
            offset_of!(rimg_options, jpeg_subsampling),
        ),
        (
            "offsetof.rimg_options.jpeg_quality_from_input",
            offset_of!(rimg_options, jpeg_quality_from_input),
        ),
        ("sizeof.rimg_info", size_of::<rimg_info>()),
        (
            "offsetof.rimg_info.input_jpeg_quality",
            offset_of!(rimg_info, input_jpeg_quality),
        ),
        ("offsetof.rimg_info.width", offset_of!(rimg_info, width)),
        ("offsetof.rimg_info.height", offset_of!(rimg_info, height)),
        ("offsetof.rimg_info.has_alpha", offset_of!(rimg_info, has_alpha)),
        ("value.RIMG_ABI_VERSION", RIMG_ABI_VERSION as usize),
        ("value.RIMG_FORMAT_WEBP", RIMG_FORMAT_WEBP as usize),
        ("value.RIMG_ERR_ENCODE", -RIMG_ERR_ENCODE as usize),
    ];
    assert_eq!(
        rust.len(),
        c.len(),
        "tests/c/layout.c and this list name different things"
    );
    for (name, value) in rust {
        assert_eq!(c.get(name), Some(&value), "{name}");
    }
    // Written down so that a change of size is a decision and not an accident: it is an ABI
    // change, and RIMG_ABI_VERSION moves with it.
    assert_eq!(size_of::<rimg_options>(), 40);
    assert_eq!(size_of::<rimg_info>(), 16);
}
