//! Decode under limits, encode again. Everything here is safe Rust over slices; `ffi` is the only
//! caller that knows about pointers.
//!
//! The order matters and is the point of the module: the format is decided on the first bytes,
//! the header is read, every limit is checked on what the header says, and only then is a pixel
//! decoded. What is encoded afterwards is the pixel buffer and nothing else -- image-rs writes no
//! metadata it was not handed, and this module hands it none.

use std::io::Cursor;

use image::codecs::png::PngEncoder;
use image::error::{ImageError, LimitErrorKind};
use image::{ColorType, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, RgbImage};

use mozjpeg::ColorSpace as JpegColor;

use crate::abi::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    WebP,
}

impl Format {
    pub fn bit(self) -> u32 {
        match self {
            Format::Jpeg => RIMG_FORMAT_JPEG,
            Format::Png => RIMG_FORMAT_PNG,
            Format::WebP => RIMG_FORMAT_WEBP,
        }
    }

    fn image_format(self) -> ImageFormat {
        match self {
            Format::Jpeg => ImageFormat::Jpeg,
            Format::Png => ImageFormat::Png,
            Format::WebP => ImageFormat::WebP,
        }
    }

    /// By the first bytes. A file name or a content type is the sender's claim, not evidence.
    fn sniff(input: &[u8]) -> Result<Format, Error> {
        match image::guess_format(input) {
            Ok(ImageFormat::Jpeg) => Ok(Format::Jpeg),
            Ok(ImageFormat::Png) => Ok(Format::Png),
            Ok(ImageFormat::WebP) => Ok(Format::WebP),
            _ => Err(Error::Unsupported),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidArgument,
    NoMemory,
    Unsupported,
    Decode,
    Limit,
    Encode,
}

impl Error {
    pub fn status(self) -> i32 {
        match self {
            Error::InvalidArgument => RIMG_ERR_INVALID_ARGUMENT,
            Error::NoMemory => RIMG_ERR_NO_MEMORY,
            Error::Unsupported => RIMG_ERR_UNSUPPORTED,
            Error::Decode => RIMG_ERR_DECODE,
            Error::Limit => RIMG_ERR_LIMIT,
            Error::Encode => RIMG_ERR_ENCODE,
        }
    }
}

fn decode_error(e: ImageError) -> Error {
    match e {
        ImageError::Limits(l) => match l.kind() {
            LimitErrorKind::InsufficientMemory => Error::NoMemory,
            _ => Error::Limit,
        },
        ImageError::Unsupported(_) => Error::Unsupported,
        _ => Error::Decode,
    }
}

/// `rimg_options`, validated: an output format that exists, a quality in range, 0 as "no limit".
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub input_formats: u32,
    pub output: Format,
    pub max_width: Option<u32>,
    pub max_height: Option<u32>,
    pub max_pixels: Option<u64>,
    pub max_alloc_bytes: Option<u64>,
    pub jpeg_quality: u8,
    pub jpeg_subsampling: bool,
    pub apply_orientation: bool,
    pub background: [u8; 3],
}

impl Config {
    pub fn from_options(o: &rimg_options) -> Result<Config, Error> {
        let output = match o.output_format {
            RIMG_FORMAT_JPEG => Format::Jpeg,
            RIMG_FORMAT_PNG => Format::Png,
            _ => return Err(Error::InvalidArgument),
        };
        const KNOWN: u32 = RIMG_FORMAT_JPEG | RIMG_FORMAT_PNG | RIMG_FORMAT_WEBP;
        if o.input_formats == 0 || o.input_formats & !KNOWN != 0 {
            return Err(Error::InvalidArgument);
        }
        if !(1..=100).contains(&o.jpeg_quality) {
            return Err(Error::InvalidArgument);
        }
        Ok(Config {
            input_formats: o.input_formats,
            output,
            max_width: (o.max_width != 0).then_some(o.max_width),
            max_height: (o.max_height != 0).then_some(o.max_height),
            max_pixels: (o.max_pixels != 0).then_some(o.max_pixels),
            max_alloc_bytes: (o.max_alloc_bytes != 0).then_some(o.max_alloc_bytes),
            jpeg_quality: o.jpeg_quality,
            jpeg_subsampling: o.jpeg_subsampling != 0,
            apply_orientation: o.apply_orientation != 0,
            background: o.background,
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Config::from_options(&default_options()).expect("the defaults are valid")
    }
}

fn info(format: Format, (width, height): (u32, u32), color: ColorType) -> rimg_info {
    rimg_info {
        input_format: format.bit(),
        width,
        height,
        has_alpha: color.has_alpha() as u8,
    }
}

/// What the header says, and nothing about the data behind it.
pub fn probe(input: &[u8]) -> Result<rimg_info, Error> {
    let format = Format::sniff(input)?;
    let decoder = ImageReader::with_format(Cursor::new(input), format.image_format())
        .into_decoder()
        .map_err(decode_error)?;
    Ok(info(format, decoder.dimensions(), decoder.color_type()))
}

pub fn reencode(cfg: &Config, input: &[u8]) -> Result<(Vec<u8>, rimg_info), Error> {
    let format = Format::sniff(input)?;
    if cfg.input_formats & format.bit() == 0 {
        return Err(Error::Unsupported);
    }

    let mut limits = Limits::no_limits();
    limits.max_image_width = cfg.max_width;
    limits.max_image_height = cfg.max_height;
    limits.max_alloc = cfg.max_alloc_bytes;
    let mut reader = ImageReader::with_format(Cursor::new(input), format.image_format());
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(decode_error)?;

    // Checked here as well, by hand: whether a decoder honors the limits it is handed is that
    // decoder's business, and these four lines do not depend on it.
    let (width, height) = decoder.dimensions();
    let over = |value: u64, max: Option<u64>| max.is_some_and(|max| value > max);
    if over(width as u64, cfg.max_width.map(u64::from))
        || over(height as u64, cfg.max_height.map(u64::from))
        || over(width as u64 * height as u64, cfg.max_pixels)
        || over(decoder.total_bytes(), cfg.max_alloc_bytes)
    {
        return Err(Error::Limit);
    }
    let color = decoder.color_type();

    // Read before the pixels: the decoder is consumed by decoding. A broken EXIF block is not a
    // reason to refuse a picture whose pixels are fine -- it is dropped either way.
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    if let (true, Some(orientation)) = (cfg.apply_orientation, orientation) {
        image.apply_orientation(orientation);
    }
    let size = (image.width(), image.height());

    let mut out = Vec::new();
    match cfg.output {
        Format::Jpeg => {
            // JPEG is 8 bit, gray or RGB, without alpha.
            if color.has_alpha() {
                encode_jpeg(
                    cfg,
                    &mut out,
                    flatten(&image, cfg.background).as_raw(),
                    size,
                    JpegColor::JCS_RGB,
                )
            } else if color.has_color() {
                encode_jpeg(
                    cfg,
                    &mut out,
                    image.into_rgb8().as_raw(),
                    size,
                    JpegColor::JCS_RGB,
                )
            } else {
                encode_jpeg(
                    cfg,
                    &mut out,
                    image.into_luma8().as_raw(),
                    size,
                    JpegColor::JCS_GRAYSCALE,
                )
            }
        }
        Format::Png => {
            // 8 bit per channel whatever came in: 16 bit and float double the size for nothing a
            // browser shows.
            let image = match (color.has_color(), color.has_alpha()) {
                (true, true) => DynamicImage::ImageRgba8(image.into_rgba8()),
                (true, false) => DynamicImage::ImageRgb8(image.into_rgb8()),
                (false, true) => DynamicImage::ImageLumaA8(image.into_luma_alpha8()),
                (false, false) => DynamicImage::ImageLuma8(image.into_luma8()),
            };
            image
                .write_with_encoder(PngEncoder::new(&mut out))
                .map_err(|_| Error::Encode)
        }
        Format::WebP => Err(Error::InvalidArgument),
    }?;

    Ok((out, info(format, size, color)))
}

/// Not image-rs's own JPEG encoder: that one stores color at full resolution and uses the
/// standard Huffman tables, and a picture that went through it came out half again as large as
/// it went in at the same quality. This is mozjpeg, set to do what libjpeg-turbo does: a baseline
/// JPEG in one interleaved scan, color at half resolution the way cameras and browsers store it
/// (4:2:0), Huffman tables built for the picture. mozjpeg's own defaults -- progressive scans,
/// trellis quantization -- are left off: they are smaller still, and they are a different answer
/// to "quality 60" than every other libjpeg gives.
///
/// An encoder only ever sees pixels, so the C is not part of what reads hostile bytes.
fn encode_jpeg(
    cfg: &Config,
    out: &mut Vec<u8>,
    pixels: &[u8],
    (width, height): (u32, u32),
    color: JpegColor,
) -> Result<(), Error> {
    // libjpeg's limit for each side.
    if width == 0 || height == 0 || width > 65500 || height > 65500 {
        return Err(Error::Encode);
    }
    let encode = || -> std::io::Result<Vec<u8>> {
        let mut compress = mozjpeg::Compress::new(color);
        compress.set_fastest_defaults();
        compress.set_size(width as usize, height as usize);
        compress.set_quality(cfg.jpeg_quality as f32);
        compress.set_optimize_coding(true);
        // A gray picture has no color components to subsample.
        if color == JpegColor::JCS_RGB {
            let pixels_per_sample = if cfg.jpeg_subsampling { (2, 2) } else { (1, 1) };
            compress.set_chroma_sampling_pixel_sizes(pixels_per_sample, pixels_per_sample);
        }
        let mut started = compress.start_compress(Vec::new())?;
        started.write_scanlines(pixels)?;
        started.finish()
    };
    // libjpeg reports an error by not returning, and the wrapper turns that into a panic that
    // unwinds through the C frames. Caught here, so that it is an encoding error and not
    // RIMG_ERR_PANIC.
    match std::panic::catch_unwind(encode) {
        Ok(Ok(encoded)) => {
            *out = encoded;
            Ok(())
        }
        _ => Err(Error::Encode),
    }
}

/// Lays the picture over an opaque background. Dropping the alpha channel instead would show
/// whatever color the transparent pixels happen to carry -- usually black.
fn flatten(image: &DynamicImage, background: [u8; 3]) -> RgbImage {
    let rgba = image.to_rgba8();
    let mut rgb = RgbImage::new(rgba.width(), rgba.height());
    for (src, dst) in rgba.pixels().zip(rgb.pixels_mut()) {
        let alpha = src[3] as u32;
        for c in 0..3 {
            dst[c] = ((src[c] as u32 * alpha + background[c] as u32 * (255 - alpha) + 127) / 255) as u8;
        }
    }
    rgb
}
