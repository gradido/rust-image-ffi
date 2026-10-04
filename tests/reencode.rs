//! The module through its C interface, against pictures built here: clean ones, ones that carry
//! something besides pixels, and ones that are not pictures.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, GenericImageView, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use rust_image_ffi::abi::*;
use rust_image_ffi::ffi::*;

const PAYLOAD: &[u8] = b"<script>alert('rimg')</script>";

/// Left half red, right half blue: enough to tell which way a picture has been turned.
fn picture(width: u32, height: u32) -> RgbImage {
    RgbImage::from_fn(width, height, |x, _| {
        if x < width / 2 {
            Rgb([220, 20, 20])
        } else {
            Rgb([20, 20, 220])
        }
    })
}

fn jpeg(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    picture(width, height)
        .write_with_encoder(JpegEncoder::new_with_quality(&mut out, 90))
        .unwrap();
    out
}

fn encoded(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, format).unwrap();
    out.into_inner()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// A JPEG segment, placed right behind the start marker.
fn with_segment(jpeg: &[u8], marker: u8, body: &[u8]) -> Vec<u8> {
    let mut out = jpeg[..2].to_vec();
    out.extend([0xff, marker]);
    out.extend(((body.len() + 2) as u16).to_be_bytes());
    out.extend(body);
    out.extend(&jpeg[2..]);
    out
}

/// An EXIF block that says nothing but the orientation.
fn exif(orientation: u16) -> Vec<u8> {
    let mut out = b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01".to_vec();
    out.extend(orientation.to_be_bytes());
    out.extend([0, 0, 0, 0, 0, 0]);
    out
}

fn options() -> rimg_options {
    let mut opt = default_options();
    unsafe { rimg_options_default(&mut opt) };
    opt
}

fn run(opt: &rimg_options, input: &[u8], cap: usize) -> (i32, Vec<u8>, usize, rimg_info) {
    let mut out = vec![0xaa; cap];
    let mut len = usize::MAX;
    let mut info = rimg_info::default();
    let status = unsafe {
        rimg_reencode(
            opt,
            input.as_ptr(),
            input.len(),
            out.as_mut_ptr(),
            cap,
            &mut len,
            &mut info,
        )
    };
    if status == RIMG_OK {
        out.truncate(len);
    }
    (status, out, len, info)
}

fn ok(opt: &rimg_options, input: &[u8]) -> (Vec<u8>, rimg_info) {
    let (status, out, _, info) = run(opt, input, 1 << 20);
    assert_eq!(status, RIMG_OK);
    (out, info)
}

#[test]
fn a_clean_jpeg_comes_back_as_a_jpeg_of_the_same_size() {
    let (out, info) = ok(&options(), &jpeg(64, 48));
    assert_eq!(
        info,
        rimg_info {
            input_format: RIMG_FORMAT_JPEG,
            width: 64,
            height: 48,
            has_alpha: 0,
            input_jpeg_quality: 90
        }
    );
    assert_eq!(image::guess_format(&out).unwrap(), ImageFormat::Jpeg);
    let decoded = image::load_from_memory(&out).unwrap();
    assert_eq!(decoded.dimensions(), (64, 48));
    assert!(decoded.get_pixel(5, 5)[0] > 150 && decoded.get_pixel(60, 5)[2] > 150);
}

#[test]
fn nothing_but_pixels_survives() {
    let clean = jpeg(32, 32);
    let reference = ok(&options(), &clean).0;

    // A comment, an application segment, and bytes behind the end marker -- the polyglot case.
    let mut dirty = with_segment(&clean, 0xfe, PAYLOAD);
    dirty = with_segment(&dirty, 0xed, PAYLOAD);
    dirty.extend(PAYLOAD);
    assert!(contains(&dirty, PAYLOAD));

    let (out, _) = ok(&options(), &dirty);
    assert!(!contains(&out, PAYLOAD));
    // Not merely free of the payload: the same bytes as if it had never been there.
    assert_eq!(out, reference);
    assert_eq!(&out[out.len() - 2..], [0xff, 0xd9]);
}

#[test]
fn a_png_text_chunk_does_not_survive() {
    let mut png = encoded(&DynamicImage::ImageRgb8(picture(16, 16)), ImageFormat::Png);
    // A tEXt chunk before IEND. Its CRC is wrong, which a decoder may ignore for a chunk it does
    // not need -- and either answer is fine here: refused, or accepted without the text.
    let iend = png.len() - 12;
    let mut chunk = ((PAYLOAD.len() + 8) as u32).to_be_bytes().to_vec();
    chunk.extend(b"tEXtComment\0");
    chunk.extend(PAYLOAD);
    chunk.extend([0, 0, 0, 0]);
    png.splice(iend..iend, chunk);

    let mut opt = options();
    opt.input_formats = RIMG_FORMAT_PNG;
    opt.output_format = RIMG_FORMAT_PNG;
    let (status, out, _, _) = run(&opt, &png, 1 << 20);
    assert!(status == RIMG_OK || status == RIMG_ERR_DECODE, "{status}");
    if status == RIMG_OK {
        assert!(!contains(&out, PAYLOAD));
        assert_eq!(image::guess_format(&out).unwrap(), ImageFormat::Png);
    }
}

#[test]
fn the_exif_orientation_is_applied_and_then_gone() {
    // 6: the stored picture has to be turned a quarter clockwise to stand upright.
    let turned = with_segment(&jpeg(64, 32), 0xe1, &exif(6));

    let (out, info) = ok(&options(), &turned);
    assert_eq!((info.width, info.height), (32, 64));
    assert!(!contains(&out, b"Exif"));
    let decoded = image::load_from_memory(&out).unwrap();
    assert_eq!(decoded.dimensions(), (32, 64));
    // What was left is now on top.
    assert!(decoded.get_pixel(16, 5)[0] > 150 && decoded.get_pixel(16, 60)[2] > 150);

    let mut opt = options();
    opt.apply_orientation = 0;
    let (out, info) = ok(&opt, &turned);
    assert_eq!((info.width, info.height), (64, 32));
    assert!(!contains(&out, b"Exif"));
}

#[test]
fn only_allowed_formats_come_in_whatever_the_bytes_claim() {
    let png = encoded(&DynamicImage::ImageRgb8(picture(16, 16)), ImageFormat::Png);
    let webp = encoded(&DynamicImage::ImageRgb8(picture(16, 16)), ImageFormat::WebP);
    let gif = b"GIF89a\x01\0\x01\0\0\0\0;";

    // The default takes JPEG and nothing else.
    assert_eq!(run(&options(), &png, 1 << 20).0, RIMG_ERR_UNSUPPORTED);
    assert_eq!(run(&options(), &webp, 1 << 20).0, RIMG_ERR_UNSUPPORTED);
    assert_eq!(run(&options(), gif, 1 << 20).0, RIMG_ERR_UNSUPPORTED);
    assert_eq!(run(&options(), b"", 1 << 20).0, RIMG_ERR_UNSUPPORTED);
    assert_eq!(run(&options(), PAYLOAD, 1 << 20).0, RIMG_ERR_UNSUPPORTED);

    let mut opt = options();
    opt.input_formats = RIMG_FORMAT_PNG | RIMG_FORMAT_WEBP;
    for input in [&png, &webp] {
        let (out, info) = ok(&opt, input);
        assert_eq!((info.width, info.height), (16, 16));
        assert_eq!(image::guess_format(&out).unwrap(), ImageFormat::Jpeg);
    }
    assert_eq!(run(&opt, &jpeg(16, 16), 1 << 20).0, RIMG_ERR_UNSUPPORTED);
}

#[test]
fn what_is_not_a_picture_is_refused() {
    let clean = jpeg(64, 64);
    // Both markers in place and nothing a decoder accepts between them: what a check of the
    // first and last two bytes lets through.
    let mut fake = vec![0xff, 0xd8, 0xff, 0xe0];
    fake.extend(PAYLOAD);
    fake.extend([0xff, 0xd9]);
    assert_eq!(run(&options(), &fake, 1 << 20).0, RIMG_ERR_DECODE);
    assert_eq!(
        run(&options(), &clean[..clean.len() / 3], 1 << 20).0,
        RIMG_ERR_DECODE
    );
    assert_eq!(run(&options(), &clean[..4], 1 << 20).0, RIMG_ERR_DECODE);
    // Without the third byte it is not even recognized.
    assert_eq!(
        run(&options(), &[0xff, 0xd8, 0x00, 0xff, 0xd9], 1 << 20).0,
        RIMG_ERR_UNSUPPORTED
    );

    let mut png = encoded(&DynamicImage::ImageRgb8(picture(64, 64)), ImageFormat::Png);
    let middle = png.len() / 2;
    png[middle..middle + 8].fill(0x55);
    let mut opt = options();
    opt.input_formats = RIMG_FORMAT_PNG;
    assert_eq!(run(&opt, &png, 1 << 20).0, RIMG_ERR_DECODE);
}

#[test]
fn limits_are_checked_on_the_header() {
    let input = jpeg(200, 100);
    let limited = |change: fn(&mut rimg_options)| {
        let mut opt = options();
        change(&mut opt);
        run(&opt, &input, 1 << 20).0
    };
    assert_eq!(limited(|o| o.max_width = 199), RIMG_ERR_LIMIT);
    assert_eq!(limited(|o| o.max_width = 200), RIMG_OK);
    assert_eq!(limited(|o| o.max_height = 99), RIMG_ERR_LIMIT);
    assert_eq!(limited(|o| o.max_pixels = 19_999), RIMG_ERR_LIMIT);
    assert_eq!(limited(|o| o.max_pixels = 20_000), RIMG_OK);
    assert_eq!(limited(|o| o.max_alloc_bytes = 59_999), RIMG_ERR_LIMIT);
    assert_eq!(
        limited(|o| {
            o.max_width = 0;
            o.max_height = 0;
            o.max_pixels = 0;
            o.max_alloc_bytes = 0;
        }),
        RIMG_OK
    );

    // A decompression bomb: a few bytes of header that announce 60000 x 60000 pixels. Refused
    // by the defaults before anything is allocated for it.
    let mut bomb = encoded(&DynamicImage::ImageRgb8(picture(8, 8)), ImageFormat::Png);
    bomb[16..20].copy_from_slice(&60_000u32.to_be_bytes());
    bomb[20..24].copy_from_slice(&60_000u32.to_be_bytes());
    let crc = crc32(&bomb[12..29]);
    bomb[29..33].copy_from_slice(&crc.to_be_bytes());
    let mut opt = options();
    opt.input_formats = RIMG_FORMAT_PNG;
    assert_eq!(run(&opt, &bomb, 1 << 20).0, RIMG_ERR_LIMIT);
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (!(crc & 1)).wrapping_add(1));
        }
    }
    !crc
}

#[test]
fn a_buffer_too_small_says_what_is_needed_and_stays_untouched() {
    let input = jpeg(64, 64);
    let (full, _) = ok(&options(), &input);

    let (status, out, len, info) = run(&options(), &input, full.len() - 1);
    assert_eq!(status, RIMG_ERR_BUFFER_TOO_SMALL);
    assert_eq!(len, full.len());
    assert_eq!((info.width, info.height), (64, 64));
    assert!(out.iter().all(|&b| b == 0xaa));

    // No buffer at all, to ask for the size.
    let mut len = 0usize;
    let status = unsafe {
        rimg_reencode(
            std::ptr::null(),
            input.as_ptr(),
            input.len(),
            std::ptr::null_mut(),
            0,
            &mut len,
            std::ptr::null_mut(),
        )
    };
    assert_eq!((status, len), (RIMG_ERR_BUFFER_TOO_SMALL, full.len()));
    assert_eq!(run(&options(), &input, full.len()).0, RIMG_OK);

    // A lower quality is how a caller gets under a byte budget.
    let mut opt = options();
    opt.jpeg_quality = 30;
    assert!(ok(&opt, &input).0.len() < full.len());
}

#[test]
fn color_is_stored_at_half_resolution_unless_asked_otherwise() {
    // Noise in the color, which is what subsampling saves on.
    let image = RgbImage::from_fn(96, 96, |x, y| {
        Rgb([
            (x * 37 % 256) as u8,
            (y * 91 % 256) as u8,
            ((x + y) * 53 % 256) as u8,
        ])
    });
    let mut input = Vec::new();
    image
        .write_with_encoder(JpegEncoder::new_with_quality(&mut input, 95))
        .unwrap();

    let (subsampled, _) = ok(&options(), &input);
    let mut opt = options();
    opt.jpeg_subsampling = 0;
    let (full, _) = ok(&opt, &input);
    assert!(
        subsampled.len() < full.len(),
        "{} against {}",
        subsampled.len(),
        full.len()
    );

    // The sampling factors are in the frame header: marker ffc0, then length (2), precision (1),
    // height (2), width (2), components (1), and per component id (1), factors (1), table (1).
    let factors = |jpeg: &[u8]| {
        let at = jpeg.windows(2).position(|w| w == [0xff, 0xc0]).unwrap();
        jpeg[at + 11]
    };
    assert_eq!(factors(&subsampled), 0x22);
    assert_eq!(factors(&full), 0x11);
    for out in [&subsampled, &full] {
        assert_eq!(image::load_from_memory(out).unwrap().dimensions(), (96, 96));
    }
}

fn probed(input: &[u8]) -> rimg_info {
    let mut info = rimg_info::default();
    assert_eq!(
        unsafe { rimg_probe(input.as_ptr(), input.len(), &mut info) },
        RIMG_OK
    );
    info
}

#[test]
fn the_quality_a_jpeg_was_written_with_is_found_again() {
    let png = encoded(&DynamicImage::ImageRgb8(picture(48, 48)), ImageFormat::Png);
    let mut opt = options();
    opt.input_formats = RIMG_FORMAT_PNG | RIMG_FORMAT_JPEG;
    opt.jpeg_quality_from_input = 0;
    for quality in [10, 30, 50, 60, 75, 85, 95, 100] {
        // By this module's encoder, with and without subsampling ...
        for subsampling in [1, 0] {
            opt.jpeg_quality = quality;
            opt.jpeg_subsampling = subsampling;
            let (out, info) = ok(&opt, &png);
            assert_eq!(info.input_jpeg_quality, 0, "a PNG has none");
            assert_eq!(probed(&out).input_jpeg_quality, quality);
        }
        // ... and by image-rs's, which is another implementation of the same tables.
        let mut other = Vec::new();
        picture(48, 48)
            .write_with_encoder(JpegEncoder::new_with_quality(&mut other, quality))
            .unwrap();
        assert_eq!(probed(&other).input_jpeg_quality, quality);
        assert_eq!(ok(&opt, &other).1.input_jpeg_quality, quality);
    }
}

#[test]
fn a_jpeg_is_not_encoded_at_a_higher_quality_than_it_came_in_with() {
    let at = |quality: u8| {
        let mut out = Vec::new();
        picture(96, 96)
            .write_with_encoder(JpegEncoder::new_with_quality(&mut out, quality))
            .unwrap();
        out
    };
    // The default: quality 85 at most.
    let opt = options();
    assert_eq!((opt.jpeg_quality, opt.jpeg_quality_from_input), (85, 1));
    assert_eq!(probed(&ok(&opt, &at(60)).0).input_jpeg_quality, 60);
    assert_eq!(probed(&ok(&opt, &at(85)).0).input_jpeg_quality, 85);
    assert_eq!(probed(&ok(&opt, &at(95)).0).input_jpeg_quality, 85);

    // Switched off, jpeg_quality is what is used, whatever came in.
    let mut fixed = options();
    fixed.jpeg_quality_from_input = 0;
    let (larger, _) = ok(&fixed, &at(60));
    assert_eq!(probed(&larger).input_jpeg_quality, 85);
    assert!(ok(&opt, &at(60)).0.len() < larger.len());

    // What has no quality of its own gets jpeg_quality.
    let mut opt = options();
    opt.input_formats = RIMG_FORMAT_PNG;
    let png = encoded(&DynamicImage::ImageRgb8(picture(96, 96)), ImageFormat::Png);
    assert_eq!(probed(&ok(&opt, &png).0).input_jpeg_quality, 85);
}

#[test]
fn gray_stays_gray() {
    let image = image::GrayImage::from_fn(40, 30, |x, _| image::Luma([(x * 6) as u8]));
    let mut input = Vec::new();
    image
        .write_with_encoder(JpegEncoder::new_with_quality(&mut input, 90))
        .unwrap();
    let (out, info) = ok(&options(), &input);
    assert_eq!((info.width, info.height), (40, 30));
    assert_eq!(
        image::load_from_memory(&out).unwrap().color(),
        image::ColorType::L8
    );
}

#[test]
fn the_output_may_overwrite_the_input() {
    let input = jpeg(48, 48);
    let (expected, _) = ok(&options(), &input);
    let mut buffer = input.clone();
    buffer.resize(1 << 16, 0);
    let mut len = 0usize;
    let status = unsafe {
        rimg_reencode(
            std::ptr::null(),
            buffer.as_ptr(),
            input.len(),
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut len,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, RIMG_OK);
    assert_eq!(&buffer[..len], expected);
}

#[test]
fn transparency_is_laid_over_the_background_for_jpeg_and_kept_for_png() {
    let image = RgbaImage::from_fn(16, 16, |x, _| {
        if x < 8 {
            Rgba([0, 0, 0, 0])
        } else {
            Rgba([0, 200, 0, 255])
        }
    });
    let png = encoded(&DynamicImage::ImageRgba8(image), ImageFormat::Png);
    let mut opt = options();
    opt.input_formats = RIMG_FORMAT_PNG;
    opt.background = [250, 250, 0];

    let (out, info) = ok(&opt, &png);
    assert_eq!(info.has_alpha, 1);
    let pixel = image::load_from_memory(&out).unwrap().get_pixel(2, 8);
    assert!(pixel[0] > 220 && pixel[1] > 220 && pixel[2] < 40, "{pixel:?}");

    opt.output_format = RIMG_FORMAT_PNG;
    let (out, _) = ok(&opt, &png);
    let decoded = image::load_from_memory(&out).unwrap();
    assert_eq!(decoded.get_pixel(2, 8)[3], 0);
    assert_eq!(decoded.get_pixel(12, 8), Rgba([0, 200, 0, 255]));
}

#[test]
fn sixteen_bit_comes_out_as_eight() {
    let image =
        image::ImageBuffer::<image::Luma<u16>, _>::from_fn(8, 8, |x, _| image::Luma([(x * 8000) as u16]));
    let png = encoded(&DynamicImage::ImageLuma16(image), ImageFormat::Png);
    let mut opt = options();
    opt.input_formats = RIMG_FORMAT_PNG;
    opt.output_format = RIMG_FORMAT_PNG;
    let (out, _) = ok(&opt, &png);
    assert_eq!(
        image::load_from_memory(&out).unwrap().color(),
        image::ColorType::L8
    );
}

#[test]
fn probe_reads_the_header_whatever_the_options_would_allow() {
    let png = encoded(
        &DynamicImage::ImageRgba8(RgbaImage::new(30, 20)),
        ImageFormat::Png,
    );
    let mut info = rimg_info::default();
    assert_eq!(unsafe { rimg_probe(png.as_ptr(), png.len(), &mut info) }, RIMG_OK);
    assert_eq!(
        info,
        rimg_info {
            input_format: RIMG_FORMAT_PNG,
            width: 30,
            height: 20,
            has_alpha: 1,
            input_jpeg_quality: 0
        }
    );
    assert_eq!(
        unsafe { rimg_probe(PAYLOAD.as_ptr(), PAYLOAD.len(), &mut info) },
        RIMG_ERR_UNSUPPORTED
    );
    assert_eq!(
        unsafe { rimg_probe(png.as_ptr(), png.len(), std::ptr::null_mut()) },
        RIMG_ERR_INVALID_ARGUMENT
    );
}

#[test]
fn arguments_are_checked() {
    let input = jpeg(8, 8);
    let invalid = |change: fn(&mut rimg_options)| {
        let mut opt = options();
        change(&mut opt);
        run(&opt, &input, 1 << 16).0
    };
    assert_eq!(
        invalid(|o| o.output_format = RIMG_FORMAT_WEBP),
        RIMG_ERR_INVALID_ARGUMENT
    );
    assert_eq!(invalid(|o| o.output_format = 0), RIMG_ERR_INVALID_ARGUMENT);
    assert_eq!(invalid(|o| o.input_formats = 0), RIMG_ERR_INVALID_ARGUMENT);
    assert_eq!(invalid(|o| o.input_formats = 8), RIMG_ERR_INVALID_ARGUMENT);
    assert_eq!(invalid(|o| o.jpeg_quality = 0), RIMG_ERR_INVALID_ARGUMENT);
    assert_eq!(invalid(|o| o.jpeg_quality = 101), RIMG_ERR_INVALID_ARGUMENT);
    assert_eq!(invalid(|o| o.struct_size = 2), RIMG_ERR_INVALID_ARGUMENT);

    let mut out = [0u8; 16];
    let mut len = 0usize;
    unsafe {
        assert_eq!(
            rimg_reencode(
                std::ptr::null(),
                input.as_ptr(),
                input.len(),
                out.as_mut_ptr(),
                16,
                std::ptr::null_mut(),
                std::ptr::null_mut()
            ),
            RIMG_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            rimg_reencode(
                std::ptr::null(),
                input.as_ptr(),
                input.len(),
                std::ptr::null_mut(),
                16,
                &mut len,
                std::ptr::null_mut()
            ),
            RIMG_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            rimg_reencode(
                std::ptr::null(),
                std::ptr::null(),
                16,
                out.as_mut_ptr(),
                16,
                &mut len,
                std::ptr::null_mut()
            ),
            RIMG_ERR_INVALID_ARGUMENT
        );
    }
}

#[test]
fn an_older_callers_shorter_options_leave_the_rest_at_the_defaults() {
    // A caller compiled when the struct ended after output_format: it allows PNG and says
    // nothing about limits or quality, and what lies behind its struct is not read.
    let mut opt = options();
    opt.struct_size = 12;
    opt.input_formats = RIMG_FORMAT_PNG;
    opt.max_width = 1;
    opt.jpeg_quality = 0;
    let png = encoded(&DynamicImage::ImageRgb8(picture(16, 16)), ImageFormat::Png);
    assert_eq!(run(&opt, &png, 1 << 16).0, RIMG_OK);
}

#[test]
fn every_status_has_a_name() {
    assert_eq!(rimg_abi_version(), RIMG_ABI_VERSION);
    for status in [0, -1, -2, -3, -4, -5, -6, -7, -99, 12345] {
        let name = unsafe { std::ffi::CStr::from_ptr(rimg_status_string(status)) };
        assert!(!name.to_bytes().is_empty());
    }
}
