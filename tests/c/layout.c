/* Prints the layout of every struct in rust_image_ffi.h as the C compiler sees it, one
 * "name value" per line. tests/abi_layout.rs compares it with the Rust definitions. */
#include <stddef.h>
#include <stdio.h>

#include "rust_image_ffi.h"

#define SIZE(T) printf("sizeof.%s %zu\n", #T, sizeof(T))
#define OFF(T, f) printf("offsetof.%s.%s %zu\n", #T, #f, offsetof(T, f))

int main(void)
{
    SIZE(rimg_options);
    OFF(rimg_options, input_formats);
    OFF(rimg_options, output_format);
    OFF(rimg_options, max_width);
    OFF(rimg_options, max_height);
    OFF(rimg_options, max_pixels);
    OFF(rimg_options, max_alloc_bytes);
    OFF(rimg_options, jpeg_quality);
    OFF(rimg_options, apply_orientation);
    OFF(rimg_options, background);
    OFF(rimg_options, jpeg_subsampling);
    OFF(rimg_options, jpeg_quality_from_input);
    SIZE(rimg_info);
    OFF(rimg_info, width);
    OFF(rimg_info, height);
    OFF(rimg_info, has_alpha);
    OFF(rimg_info, input_jpeg_quality);
    printf("value.RIMG_ABI_VERSION %d\n", RIMG_ABI_VERSION);
    printf("value.RIMG_FORMAT_WEBP %u\n", RIMG_FORMAT_WEBP);
    printf("value.RIMG_ERR_ENCODE %d\n", -(RIMG_ERR_ENCODE));
    return 0;
}
