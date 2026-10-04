/*
 * rust-image-ffi: image-rs behind a C interface. github.com/gradido/rust-image-ffi
 *
 * One job: take a picture nobody vouches for, decode it under hard limits, and encode the pixels
 * again. What comes out was written by this module's encoder from pixels alone, so nothing of the
 * input's container survives -- no EXIF, ICC profile, comment or text chunk, no bytes behind the
 * end marker, no second file hiding in the first. A picture that does not decode is refused.
 *
 * What it does not do: it does not look at what the pixels show, and it does not scale. The
 * module holds mechanism and no policy -- which formats come in, how large a picture may be and
 * how many bytes it may cost are the caller's, through rimg_options and the size of `out`.
 *
 * There is no handle and no state. Every function is thread-safe, allocates only for the length
 * of the call, and hands nothing across but bytes and lengths: no Rust type, no buffer the caller
 * frees. A panic is caught at the boundary and becomes RIMG_ERR_PANIC -- unwinding into C is
 * undefined behavior.
 *
 * The interface only grows. New fields go at the end of rimg_options, new formats and status
 * codes get new numbers, and nothing is renamed, renumbered or reused. rimg_info is frozen.
 */
#ifndef RUST_IMAGE_FFI_H
#define RUST_IMAGE_FFI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define RIMG_ABI_VERSION 1

/* Status codes. */
#define RIMG_OK 0
#define RIMG_ERR_INVALID_ARGUMENT -1
/* The encoded picture does not fit `out`. *out_len says what it needs. */
#define RIMG_ERR_BUFFER_TOO_SMALL -2
#define RIMG_ERR_NO_MEMORY -3
/* Not a format this build knows, not one rimg_options.input_formats allows, or a variant of an
 * allowed format the decoder does not implement. Decided on the first bytes, never on a name. */
#define RIMG_ERR_UNSUPPORTED -4
/* The format was recognized and the data is not a picture in it: truncated, corrupt, crafted. */
#define RIMG_ERR_DECODE -5
/* The picture is wider, higher or larger than rimg_options allows, or decoding it would take
 * more memory than max_alloc_bytes. Checked on the header, before any pixel is decoded. */
#define RIMG_ERR_LIMIT -6
#define RIMG_ERR_ENCODE -7
#define RIMG_ERR_PANIC -99

/* Formats. Bits, because rimg_options.input_formats is a set of them. */
#define RIMG_FORMAT_JPEG 1u
#define RIMG_FORMAT_PNG 2u
#define RIMG_FORMAT_WEBP 4u /* input only: image-rs encodes WebP lossless only */

typedef struct rimg_options {
    /* sizeof(rimg_options) as the caller compiled it. A newer module reads the fields the caller
     * knows and defaults the rest. rimg_options_default sets it. */
    uint32_t struct_size;
    /* Which formats may come in, RIMG_FORMAT_* or'ed. Default: JPEG only -- every format allowed
     * is one more decoder that reads hostile bytes. */
    uint32_t input_formats;
    /* RIMG_FORMAT_JPEG (default) or RIMG_FORMAT_PNG. */
    uint32_t output_format;
    /* Limits on the picture as stored, before orientation. 0 means no limit of the module's own;
     * the defaults are 8192 x 8192 and 16 000 000 pixels. */
    uint32_t max_width;
    uint32_t max_height;
    uint64_t max_pixels;
    /* What decoding may allocate for pixels. Default 128 MiB; 0 means no limit. */
    uint64_t max_alloc_bytes;
    /* 1..100, default 85. Only for JPEG output. */
    uint8_t jpeg_quality;
    /* Non-zero (default): turn the pixels the way the EXIF orientation says before encoding. The
     * tag itself never survives, so without this a picture taken upright comes out on its side. */
    uint8_t apply_orientation;
    /* R, G, B that transparent pixels are laid over when the output is JPEG, which has no alpha.
     * Default white. PNG output keeps the alpha channel. */
    uint8_t background[3];
    /* Since 0.1.1. Non-zero (default): store color at half resolution in both directions
     * (4:2:0), as cameras and browsers do. 0: full resolution (4:4:4), a third larger and sharper
     * at colored edges -- for drawings and text rather than photos. Only for JPEG output.
     *
     * It lives in what was padding at the end of the struct, so the struct's size did not
     * change: a caller that fills the struct by hand rather than through rimg_options_default
     * has a zero here and gets full resolution, as 0.1.0 wrote it. */
    uint8_t jpeg_subsampling;
} rimg_options;

/* Frozen. */
typedef struct rimg_info {
    uint32_t input_format; /* RIMG_FORMAT_* */
    uint32_t width;
    uint32_t height;
    uint8_t has_alpha; /* the input carries an alpha channel */
} rimg_info;

uint32_t rimg_abi_version(void);

/* A static, NUL-terminated English name for a status code; never NULL. For logs. */
const char *rimg_status_string(int32_t status);

void rimg_options_default(rimg_options *opt);

/* Reads the header only: which format the first bytes say, and the size as stored (before
 * orientation). No limits and no format set apply -- it answers what is there, so that a caller
 * can refuse in its own words. It proves nothing about the rest of the data. */
int32_t rimg_probe(const uint8_t *in, size_t in_len, rimg_info *info);

/* Decodes `in` and encodes it again into `out`.
 *
 *   opt      NULL for the defaults
 *   out      `out_cap` writable bytes; may be NULL when out_cap is 0, and may overlap `in`
 *   out_len  required. Set on RIMG_OK to the bytes written, and on RIMG_ERR_BUFFER_TOO_SMALL to
 *            the bytes this picture needs; 0 on every other error
 *   info     optional. On RIMG_OK and RIMG_ERR_BUFFER_TOO_SMALL: the input's format, and width
 *            and height of the picture as it was encoded -- after orientation
 *
 * `out` is untouched unless the answer is RIMG_OK. A byte budget is an `out_cap`: a caller that
 * stores at most N bytes passes N and treats RIMG_ERR_BUFFER_TOO_SMALL as "too large at this
 * quality". Of an animated picture only the first frame is taken. */
int32_t rimg_reencode(const rimg_options *opt, const uint8_t *in, size_t in_len, uint8_t *out,
                      size_t out_cap, size_t *out_len, rimg_info *info);

#ifdef __cplusplus
}
#endif

#endif
