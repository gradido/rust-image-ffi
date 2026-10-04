/* One picture through the shipped object: a JPEG with a payload hidden in a comment segment and
 * behind its end marker goes in, and what comes out is a JPEG of the same size without it. What
 * it proves is the link and the C interface -- tests/reencode.rs covers the cases. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "rust_image_ffi.h"

#include "fixture.h"

#define BUFFER_BYTES 65536

static const char payload[] = "<script>alert('rimg')</script>";

static int contains(const uint8_t *data, size_t len, const char *needle)
{
    size_t n = strlen(needle);
    size_t i;

    for (i = 0; i + n <= len; i++) {
        if (memcmp(data + i, needle, n) == 0)
            return 1;
    }
    return 0;
}

#define CHECK(cond)                                                                                \
    do {                                                                                           \
        if (!(cond)) {                                                                             \
            fprintf(stderr, "smoke: failed at line %d: %s\n", __LINE__, #cond);                    \
            return 1;                                                                              \
        }                                                                                          \
    } while (0)

int main(void)
{
    static uint8_t in[BUFFER_BYTES], out[BUFFER_BYTES];
    size_t n = strlen(payload);
    size_t in_len = 0, out_len = 0, needed = 0;
    rimg_options opt;
    rimg_info info;
    int32_t status;

    CHECK(rimg_abi_version() == RIMG_ABI_VERSION);

    /* Start marker, a comment segment with the payload, the rest of the picture, the payload
     * once more behind the end marker. */
    memcpy(in, fixture_jpeg, 2);
    in_len = 2;
    in[in_len++] = 0xff;
    in[in_len++] = 0xfe;
    in[in_len++] = (uint8_t)((n + 2) >> 8);
    in[in_len++] = (uint8_t)(n + 2);
    memcpy(in + in_len, payload, n);
    in_len += n;
    memcpy(in + in_len, fixture_jpeg + 2, sizeof(fixture_jpeg) - 2);
    in_len += sizeof(fixture_jpeg) - 2;
    memcpy(in + in_len, payload, n);
    in_len += n;
    CHECK(contains(in, in_len, payload));

    memset(&info, 0, sizeof(info));
    CHECK(rimg_probe(in, in_len, &info) == RIMG_OK);
    CHECK(info.input_format == RIMG_FORMAT_JPEG);
    CHECK(info.width == FIXTURE_WIDTH && info.height == FIXTURE_HEIGHT);

    rimg_options_default(&opt);
    CHECK(opt.input_formats == RIMG_FORMAT_JPEG);

    /* Asking for the size first, then with a buffer. */
    status = rimg_reencode(&opt, in, in_len, NULL, 0, &needed, NULL);
    CHECK(status == RIMG_ERR_BUFFER_TOO_SMALL && needed > 0);
    memset(&info, 0, sizeof(info));
    status = rimg_reencode(&opt, in, in_len, out, sizeof(out), &out_len, &info);
    if (status != RIMG_OK) {
        fprintf(stderr, "smoke: rimg_reencode: %s\n", rimg_status_string(status));
        return 1;
    }
    CHECK(out_len == needed);
    CHECK(info.width == FIXTURE_WIDTH && info.height == FIXTURE_HEIGHT && info.has_alpha == 0);
    CHECK(out[0] == 0xff && out[1] == 0xd8 && out[out_len - 2] == 0xff && out[out_len - 1] == 0xd9);
    CHECK(!contains(out, out_len, payload));

    /* What is not a picture is refused, and a format that is not allowed as well. */
    CHECK(rimg_reencode(NULL, (const uint8_t *)payload, n, out, sizeof(out), &out_len, NULL) ==
          RIMG_ERR_UNSUPPORTED);
    CHECK(rimg_reencode(&opt, in, in_len / 2, out, sizeof(out), &out_len, NULL) == RIMG_ERR_DECODE);
    opt.input_formats = RIMG_FORMAT_PNG;
    CHECK(rimg_reencode(&opt, in, in_len, out, sizeof(out), &out_len, NULL) ==
          RIMG_ERR_UNSUPPORTED);
    opt.input_formats = RIMG_FORMAT_JPEG;
    opt.max_width = FIXTURE_WIDTH - 1;
    CHECK(rimg_reencode(&opt, in, in_len, out, sizeof(out), &out_len, NULL) == RIMG_ERR_LIMIT);

    printf("ok: %zu bytes in, %zu bytes out, payload gone\n", in_len, needed);
    return 0;
}
