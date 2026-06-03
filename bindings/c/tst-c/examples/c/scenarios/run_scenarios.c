/*
 * run_scenarios.c — C adapter for the cross-binding scenario harness (WS-5).
 *
 * WHY THIS FILE EXISTS
 * --------------------
 * The ts-transformer project has three binding layers: Rust (the core),
 * Python (tst-py / PyO3), and C (tst-c / cbindgen). WS-5 of the
 * test-architecture overhaul adds a cross-binding scenario harness that
 * proves all three bindings produce identical, deterministic output from the
 * same committed input artifacts. This file is the C adapter — the third
 * leg of that harness.
 *
 * For each scenario in the committed `scenarios.toml` manifest, this program:
 *   1. Reads the corresponding input artifact.
 *   2. Feeds it through the C ABI (tst_demuxer_* offline demuxer, ABI minor 8).
 *   3. Normalises the result to the binding-neutral golden envelope
 *      (schema_version, lossy, core[], extensions).
 *   4. Compares field-by-field against the committed `golden.json`.
 *   5. Prints a per-scenario pass/fail line; exits non-zero if any fail.
 *
 * The normalisation rules mirror the Rust adapter in
 * `crates/tst-integration/tests/rust_scenarios.rs` and the Python adapter in
 * `bindings/python/tests/test_scenarios.py` exactly.
 *
 * HOW TO BUILD AND RUN
 * --------------------
 * From the workspace root (ts-transformer/):
 *
 *   SRT_FORCE_VENDORED=1 RIST_FORCE_VENDORED=1 cargo build -p tst-c
 *   gcc -I bindings/c/tst-c/include -L target/debug -Wall -Werror \
 *       -o /tmp/run_scenarios \
 *       bindings/c/tst-c/examples/c/scenarios/run_scenarios.c -ltstrans
 *   LD_LIBRARY_PATH=target/debug /tmp/run_scenarios \
 *       crates/tst-integration/tests/fixtures/scenarios
 *
 * Or use the companion shell script:
 *   bash bindings/c/tst-c/examples/c/scenarios/run_scenarios.sh
 *
 * Default scenarios dir (when no argv[1] given):
 *   ../../../tst-integration/tests/fixtures/scenarios  (relative to CWD).
 *   This works when invoked from within bindings/c/tst-c/ or from the workspace
 *   root; adjust with argv[1] if your working directory differs.
 *
 * DESIGN DECISIONS
 * ----------------
 *
 * SHA-256 (no external dep):
 *   The C standard library has no SHA-256. mbedTLS *is* statically linked
 *   into libtstrans.so but its symbols are not exported (only tst_* symbols
 *   are exported). Rather than pull in libcrypto or require an extra -I for
 *   mbedtls headers, this file embeds a compact public-domain SHA-256
 *   implementation (~80 lines). It produces the same digest as Rust's sha2
 *   crate — verified against the committed golden's payload_sha256 field.
 *
 * TOML parsing (hand-rolled minimal parser):
 *   scenarios.toml is a simple fixed-shape manifest. Pulling in a TOML
 *   library into a C teaching example would be heavyweight and inappropriate.
 *   The parser here handles exactly the shape needed:
 *     [[scenario]]
 *     id = "..."
 *     kind = "..."
 *     input = "..."
 *     golden = "..."
 *   It ignores all other keys. It does NOT handle TOML string escapes beyond
 *   basic quoted strings — the manifest uses only plain ASCII identifiers and
 *   file paths, so this is safe. Document any future non-ASCII paths before
 *   adding them to scenarios.toml.
 *
 * JSON parsing (hand-rolled minimal parser):
 *   The golden.json files have a known fixed shape. Rather than link a JSON
 *   library, this adapter implements a targeted extractor for the fields it
 *   needs. For golden comparison, it compares the normalised fields
 *   individually (program, pid, stream_type, pts, key, payload_sha256, set)
 *   rather than serialising its own output to a JSON string and doing a
 *   string compare — this is more robust to key ordering.
 *
 * Roundtrip scenario (re-mux in C):
 *   When built with the `srt` feature (TST_HAS_SRT defined — the default), the
 *   offline muxer surface (tst_muxer_open/push_video/pull/close) is available.
 *   The adapter re-runs the exact video-roundtrip recipe in C:
 *     - tst_mux_config_new + add_program(1, 0x1000) + add_video_stream(0x1011, H264)
 *     - tst_muxer_open, push one synthetic H.264 IDR at pts=0 (key_frame),
 *       drain with a 1316-byte pull loop, close.
 *     - Compare the C-produced bytes to the committed output.ts byte-for-byte
 *       AND assert their sha256 equals golden.extensions.output_sha256.
 *   This proves C reproduces the mux output, not merely that it can hash a
 *   committed file. When built WITHOUT srt (TST_HAS_SRT undefined), the muxer
 *   surface is unavailable: fall back to reading output.ts, hashing it, and
 *   comparing to output_sha256 (weaker, but keeps the adapter buildable).
 *
 * Strict-rejection scenario:
 *   Feed 8192 × 0xFF through tst_demuxer_feed (default config, no strict
 *   mode). The all-0xFF input has no 0x47 MPEG-TS sync bytes; after scanning
 *   SYNC_SEARCH_WINDOW (188 × 32 = 6016) bytes without finding a sync byte
 *   the demuxer returns an Unrecoverable error (negative tst_e code). Any
 *   negative code maps to the umbrella public code "STRICT_REJECTION" —
 *   the same umbrella mapping the Rust and Python adapters use. Also verifies
 *   that tst_demuxer_close(NULL) is safe (null-safe contract).
 */

#include "tstrans.h"
#include <inttypes.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ── Compact SHA-256 implementation (public domain) ─────────────────────────
 *
 * WHY: The C standard library has no SHA-256. libtstrans exports only tst_*
 * symbols (mbedTLS is linked but its symbols are hidden). Rather than depend
 * on libssl/libcrypto, a compact (~80 LOC) public-domain implementation is
 * embedded here. Produces the same output as Rust's sha2::Sha256.
 *
 * Source: adapted from the public-domain "FIPS 180-4 reference implementation"
 * pattern used widely in embedded and test code.
 */

/* SHA-256 initial hash values (first 32 bits of the fractional parts of the
 * square roots of the first 8 primes — FIPS 180-4 §5.3.3). */
static const uint32_t SHA256_H0[8] = {
    0x6a09e667u, 0xbb67ae85u, 0x3c6ef372u, 0xa54ff53au,
    0x510e527fu, 0x9b05688cu, 0x1f83d9abu, 0x5be0cd19u,
};

/* SHA-256 round constants (first 32 bits of the fractional parts of the cube
 * roots of the first 64 primes — FIPS 180-4 §4.2.2). */
static const uint32_t SHA256_K[64] = {
    0x428a2f98u, 0x71374491u, 0xb5c0fbcfu, 0xe9b5dba5u,
    0x3956c25bu, 0x59f111f1u, 0x923f82a4u, 0xab1c5ed5u,
    0xd807aa98u, 0x12835b01u, 0x243185beu, 0x550c7dc3u,
    0x72be5d74u, 0x80deb1feu, 0x9bdc06a7u, 0xc19bf174u,
    0xe49b69c1u, 0xefbe4786u, 0x0fc19dc6u, 0x240ca1ccu,
    0x2de92c6fu, 0x4a7484aau, 0x5cb0a9dcu, 0x76f988dau,
    0x983e5152u, 0xa831c66du, 0xb00327c8u, 0xbf597fc7u,
    0xc6e00bf3u, 0xd5a79147u, 0x06ca6351u, 0x14292967u,
    0x27b70a85u, 0x2e1b2138u, 0x4d2c6dfcu, 0x53380d13u,
    0x650a7354u, 0x766a0abbu, 0x81c2c92eu, 0x92722c85u,
    0xa2bfe8a1u, 0xa81a664bu, 0xc24b8b70u, 0xc76c51a3u,
    0xd192e819u, 0xd6990624u, 0xf40e3585u, 0x106aa070u,
    0x19a4c116u, 0x1e376c08u, 0x2748774cu, 0x34b0bcb5u,
    0x391c0cb3u, 0x4ed8aa4au, 0x5b9cca4fu, 0x682e6ff3u,
    0x748f82eeu, 0x78a5636fu, 0x84c87814u, 0x8cc70208u,
    0x90befffau, 0xa4506cebu, 0xbef9a3f7u, 0xc67178f2u,
};

#define SHA256_ROTR(x, n) (((x) >> (n)) | ((x) << (32 - (n))))
#define SHA256_CH(x, y, z) (((x) & (y)) ^ (~(x) & (z)))
#define SHA256_MAJ(x, y, z) (((x) & (y)) ^ ((x) & (z)) ^ ((y) & (z)))
#define SHA256_EP0(x) (SHA256_ROTR(x,  2) ^ SHA256_ROTR(x, 13) ^ SHA256_ROTR(x, 22))
#define SHA256_EP1(x) (SHA256_ROTR(x,  6) ^ SHA256_ROTR(x, 11) ^ SHA256_ROTR(x, 25))
#define SHA256_SIG0(x) (SHA256_ROTR(x,  7) ^ SHA256_ROTR(x, 18) ^ ((x) >>  3))
#define SHA256_SIG1(x) (SHA256_ROTR(x, 17) ^ SHA256_ROTR(x, 19) ^ ((x) >> 10))

typedef struct {
    uint32_t state[8];  /* current hash state (H0..H7) */
    uint64_t bitlen;    /* total bits processed so far */
    uint8_t  data[64];  /* current 512-bit (64-byte) message block */
    size_t   datalen;   /* bytes buffered in data[] */
} sha256_ctx_t;

static void sha256_transform(sha256_ctx_t *ctx, const uint8_t block[64]) {
    /* Expand 16-word block to 64-word schedule (FIPS 180-4 §6.2.2 step 1). */
    uint32_t w[64];
    for (int i = 0; i < 16; i++) {
        w[i] = ((uint32_t)block[i*4  ] << 24)
             | ((uint32_t)block[i*4+1] << 16)
             | ((uint32_t)block[i*4+2] <<  8)
             | ((uint32_t)block[i*4+3]);
    }
    for (int i = 16; i < 64; i++) {
        w[i] = SHA256_SIG1(w[i-2]) + w[i-7] + SHA256_SIG0(w[i-15]) + w[i-16];
    }
    /* Working variables (step 2). */
    uint32_t a = ctx->state[0], b = ctx->state[1],
             c = ctx->state[2], d = ctx->state[3],
             e = ctx->state[4], f = ctx->state[5],
             g = ctx->state[6], h = ctx->state[7];
    /* 64 compression rounds (step 3). */
    for (int i = 0; i < 64; i++) {
        uint32_t t1 = h + SHA256_EP1(e) + SHA256_CH(e,f,g) + SHA256_K[i] + w[i];
        uint32_t t2 = SHA256_EP0(a) + SHA256_MAJ(a,b,c);
        h = g; g = f; f = e; e = d + t1;
        d = c; c = b; b = a; a = t1 + t2;
    }
    /* Add compressed chunk to current hash (step 4). */
    ctx->state[0] += a; ctx->state[1] += b;
    ctx->state[2] += c; ctx->state[3] += d;
    ctx->state[4] += e; ctx->state[5] += f;
    ctx->state[6] += g; ctx->state[7] += h;
}

static void sha256_init(sha256_ctx_t *ctx) {
    memcpy(ctx->state, SHA256_H0, sizeof(SHA256_H0));
    ctx->bitlen = 0;
    ctx->datalen = 0;
}

static void sha256_update(sha256_ctx_t *ctx, const uint8_t *data, size_t len) {
    for (size_t i = 0; i < len; i++) {
        ctx->data[ctx->datalen++] = data[i];
        if (ctx->datalen == 64) {
            sha256_transform(ctx, ctx->data);
            ctx->bitlen += 512;
            ctx->datalen = 0;
        }
    }
}

/* Produce the 32-byte digest into `out`. */
static void sha256_final(sha256_ctx_t *ctx, uint8_t out[32]) {
    size_t i = ctx->datalen;
    /* Pad the remaining data (FIPS 180-4 §5.1.1). */
    if (ctx->datalen < 56) {
        ctx->data[i++] = 0x80;
        while (i < 56) ctx->data[i++] = 0x00;
    } else {
        ctx->data[i++] = 0x80;
        while (i < 64) ctx->data[i++] = 0x00;
        sha256_transform(ctx, ctx->data);
        memset(ctx->data, 0, 56);
    }
    /* Append the original message length in bits as a 64-bit big-endian. */
    ctx->bitlen += (uint64_t)ctx->datalen * 8;
    for (int j = 7; j >= 0; j--) {
        ctx->data[56 + (7 - j)] = (uint8_t)(ctx->bitlen >> (j * 8));
    }
    sha256_transform(ctx, ctx->data);
    /* Produce output in big-endian word order. */
    for (int k = 0; k < 8; k++) {
        out[k*4  ] = (uint8_t)(ctx->state[k] >> 24);
        out[k*4+1] = (uint8_t)(ctx->state[k] >> 16);
        out[k*4+2] = (uint8_t)(ctx->state[k] >>  8);
        out[k*4+3] = (uint8_t)(ctx->state[k]);
    }
}

/* Hash `len` bytes at `data`; write lowercase hex digest into `hex_out`
 * (must be ≥ 65 bytes: 64 hex chars + NUL terminator). */
static void sha256_hex(const uint8_t *data, size_t len, char hex_out[65]) {
    sha256_ctx_t ctx;
    sha256_init(&ctx);
    sha256_update(&ctx, data, len);
    uint8_t digest[32];
    sha256_final(&ctx, digest);
    for (int i = 0; i < 32; i++) {
        sprintf(hex_out + i * 2, "%02x", (unsigned)digest[i]);
    }
    hex_out[64] = '\0';
}

/* ── File I/O helpers ────────────────────────────────────────────────────────*/

/* Read the entire contents of `path` into a heap-allocated buffer.
 * Sets *out_len to the number of bytes read. Returns NULL on error. */
static uint8_t *read_file(const char *path, size_t *out_len) {
    FILE *f = fopen(path, "rb");
    if (!f) {
        fprintf(stderr, "ERROR: cannot open '%s'\n", path);
        return NULL;
    }
    if (fseek(f, 0, SEEK_END) != 0) {
        fprintf(stderr, "ERROR: fseek failed on '%s'\n", path);
        fclose(f);
        return NULL;
    }
    long size = ftell(f);
    if (size < 0) {
        fprintf(stderr, "ERROR: ftell failed on '%s'\n", path);
        fclose(f);
        return NULL;
    }
    rewind(f);
    uint8_t *buf = (uint8_t *)malloc((size_t)size + 1);
    if (!buf) {
        fprintf(stderr, "ERROR: malloc failed (%ld bytes)\n", size);
        fclose(f);
        return NULL;
    }
    size_t n = fread(buf, 1, (size_t)size, f);
    fclose(f);
    if ((long)n != size) {
        fprintf(stderr, "ERROR: short read on '%s' (%zu of %ld bytes)\n",
                path, n, size);
        free(buf);
        return NULL;
    }
    buf[size] = '\0'; /* NUL-terminate for text parsing convenience */
    *out_len = n;
    return buf;
}

/* Build an absolute path from a directory and a relative path.
 * Result is malloc'd; caller must free. */
static char *path_join(const char *dir, const char *rel) {
    size_t dlen = strlen(dir);
    size_t rlen = strlen(rel);
    char *out = (char *)malloc(dlen + 1 + rlen + 1);
    if (!out) return NULL;
    memcpy(out, dir, dlen);
    out[dlen] = '/';
    memcpy(out + dlen + 1, rel, rlen);
    out[dlen + 1 + rlen] = '\0';
    return out;
}

/* ── Minimal TOML parser ─────────────────────────────────────────────────────
 *
 * WHY hand-rolled: scenarios.toml is a fixed-shape manifest with only simple
 * quoted-string values. Pulling a TOML library into a C teaching example
 * would be heavyweight. This parser handles exactly the needed shape:
 *
 *   [[scenario]]
 *   id = "..."
 *   kind = "..."
 *   input = "..."
 *   golden = "..."
 *   features = [...]    <-- ignored (array not needed)
 *   tier = "..."        <-- ignored
 *   schema_version = N  <-- ignored
 *
 * Limitations (all safe for the current manifest):
 *   - Reads only the four string keys listed above; ignores all others.
 *   - Does NOT handle TOML string escapes (no backslash processing).
 *   - Does NOT handle multi-line strings.
 *   - Silently accepts malformed lines (skips them).
 *
 * If scenarios.toml ever gains non-ASCII paths or escape sequences, extend
 * this parser or switch to a real TOML library before shipping.
 */

#define MAX_SCENARIOS 32
#define MAX_STR_LEN   256

typedef struct {
    char id[MAX_STR_LEN];
    char kind[MAX_STR_LEN];
    char input[MAX_STR_LEN];
    char golden[MAX_STR_LEN];
} scenario_entry_t;

/* Extract the value from a line like `  key = "value"` or `key = "value"`.
 * Copies the quoted content into `out` (max `out_size` bytes including NUL).
 * Returns 1 on success, 0 if the key doesn't match or line isn't a match. */
static int toml_extract_string(const char *line, const char *key,
                               char *out, size_t out_size) {
    /* Skip leading whitespace. */
    while (*line == ' ' || *line == '\t') line++;
    size_t klen = strlen(key);
    if (strncmp(line, key, klen) != 0) return 0;
    line += klen;
    /* Skip whitespace + '=' + whitespace. */
    while (*line == ' ' || *line == '\t') line++;
    if (*line != '=') return 0;
    line++;
    while (*line == ' ' || *line == '\t') line++;
    if (*line != '"') return 0;
    line++;
    /* Copy up to the closing '"'. */
    size_t i = 0;
    while (*line && *line != '"' && i < out_size - 1) {
        out[i++] = *line++;
    }
    out[i] = '\0';
    return 1;
}

/* Parse `scenarios.toml` content (NUL-terminated) into `entries`.
 * Returns the number of scenarios found (up to MAX_SCENARIOS). */
static int parse_scenarios_toml(const char *text, scenario_entry_t *entries) {
    int count = 0;
    int in_scenario = 0;  /* 1 once we've seen [[scenario]] */

    /* Walk line by line. */
    const char *p = text;
    while (*p) {
        /* Find end of current line. */
        const char *eol = p;
        while (*eol && *eol != '\n') eol++;

        /* Copy the line into a temporary buffer for parsing. */
        size_t llen = (size_t)(eol - p);
        if (llen >= MAX_STR_LEN) {
            /* Skip overlong lines (shouldn't happen in a well-formed manifest). */
            p = (*eol == '\n') ? eol + 1 : eol;
            continue;
        }
        char line[MAX_STR_LEN];
        memcpy(line, p, llen);
        line[llen] = '\0';

        /* Strip trailing '\r' (Windows CRLF). */
        if (llen > 0 && line[llen - 1] == '\r') {
            line[llen - 1] = '\0';
        }

        /* Detect a [[scenario]] section header — start a new entry. */
        {
            const char *t = line;
            while (*t == ' ' || *t == '\t') t++;
            if (strncmp(t, "[[scenario]]", 12) == 0) {
                if (count < MAX_SCENARIOS) {
                    memset(&entries[count], 0, sizeof(entries[count]));
                    count++;
                    in_scenario = 1;
                }
                goto next_line;
            }
        }

        /* Parse key-value pairs inside the current [[scenario]] block. */
        if (in_scenario && count > 0) {
            scenario_entry_t *e = &entries[count - 1];
            /* Try each of the four keys we care about. */
            if (toml_extract_string(line, "id",     e->id,     MAX_STR_LEN)) goto next_line;
            if (toml_extract_string(line, "kind",   e->kind,   MAX_STR_LEN)) goto next_line;
            if (toml_extract_string(line, "input",  e->input,  MAX_STR_LEN)) goto next_line;
            if (toml_extract_string(line, "golden", e->golden, MAX_STR_LEN)) goto next_line;
            /* Any other key (features, tier, schema_version) is silently ignored. */
        }

next_line:
        p = (*eol == '\n') ? eol + 1 : eol;
        /* Stop at the real NUL terminator. */
        if (*p == '\0') break;
    }
    return count;
}

/* ── Minimal JSON extractor ──────────────────────────────────────────────────
 *
 * WHY hand-rolled: golden.json has a fixed known shape. Rather than link a
 * JSON library, this extracts targeted fields. For a golden like:
 *
 *   {"schema_version":0,"lossy":false,"core":[...],"extensions":null}
 *
 * We extract:
 *   - core[] array: each object's "event", "program", "pid", "stream_type",
 *     "pts", "key", "payload_sha256", "set", "code" fields as strings.
 *   - extensions.output_sha256: the sha256 digest string.
 *
 * Approach: scan for `"key":"value"` / `"key":value` patterns. This is
 * order-independent and handles the fixed field set safely. It does NOT
 * handle nested objects, arrays of arrays, or TOML-style escapes beyond
 * `\"` in string values — all safe for the current golden shape.
 */

/* Extract a JSON string value for `key` in `json` (NUL-terminated).
 * `start` is the offset to begin searching from (for scanning arrays).
 * Returns the offset just past the value, or 0 on failure. Copies the
 * string content into `out` (truncated to out_size-1 chars + NUL).
 *
 * Handles optional whitespace between the key colon and the opening quote
 * — the serde_json pretty-printer emits `"key": "value"` (with a space)
 * while the compact form emits `"key":"value"` (no space). Both are valid
 * JSON and both appear in the committed golden files. */
static size_t json_extract_string(const char *json, size_t start,
                                  const char *key,
                                  char *out, size_t out_size) {
    /* Search for `"key":` (key name in quotes followed by colon). */
    char key_pat[MAX_STR_LEN + 4];
    snprintf(key_pat, sizeof(key_pat), "\"%s\":", key);
    const char *found = strstr(json + start, key_pat);
    if (!found) {
        out[0] = '\0';
        return 0;
    }
    const char *p = found + strlen(key_pat);
    /* Skip optional whitespace between ':' and the opening '"'. */
    while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r') p++;
    if (*p != '"') {
        /* Value is not a string (e.g. null, number, boolean). */
        out[0] = '\0';
        return 0;
    }
    p++; /* skip opening '"' */
    size_t i = 0;
    while (*p && *p != '"' && i < out_size - 1) {
        if (*p == '\\' && *(p + 1) == '"') {
            /* Handle escaped quote. */
            out[i++] = '"';
            p += 2;
        } else {
            out[i++] = *p++;
        }
    }
    out[i] = '\0';
    /* Return offset past the closing '"'. */
    return (size_t)((p - json) + 1);
}

/* Extract a JSON integer (int64) for `key`. Returns 1 on success.
 * Handles optional whitespace between ':' and the digit sequence. */
static int json_extract_int64(const char *json, size_t start,
                              const char *key, int64_t *out) {
    char pat[MAX_STR_LEN + 4];
    snprintf(pat, sizeof(pat), "\"%s\":", key);
    const char *found = strstr(json + start, pat);
    if (!found) return 0;
    const char *vstart = found + strlen(pat);
    while (*vstart == ' ' || *vstart == '\t' || *vstart == '\n' || *vstart == '\r') vstart++;
    char *endp;
    *out = (int64_t)strtoll(vstart, &endp, 10);
    return (endp != vstart);
}

/* Extract a JSON boolean for `key`. Returns 1 on success.
 * Handles optional whitespace between ':' and the value. */
static int json_extract_bool(const char *json, size_t start,
                             const char *key, int *out) {
    char pat[MAX_STR_LEN + 4];
    snprintf(pat, sizeof(pat), "\"%s\":", key);
    const char *found = strstr(json + start, pat);
    if (!found) return 0;
    const char *vstart = found + strlen(pat);
    while (*vstart == ' ' || *vstart == '\t' || *vstart == '\n' || *vstart == '\r') vstart++;
    if (strncmp(vstart, "true", 4) == 0)  { *out = 1; return 1; }
    if (strncmp(vstart, "false", 5) == 0) { *out = 0; return 1; }
    return 0;
}

/* ── pid → stream_type map ───────────────────────────────────────────────────
 *
 * WHY two-pass: the golden's stream_type is the raw PMT byte (e.g. "0x1b"),
 * not a codec enum. Like the Rust and Python normalisers, we build a
 * pid → stream_type_byte map from ProgramMap events first, then emit media
 * events using that map. This makes the normalisation binding-neutral.
 */

#define MAX_PIDS 64

typedef struct {
    uint16_t pid;
    uint8_t  stream_type_byte;
} pid_st_entry_t;

typedef struct {
    pid_st_entry_t entries[MAX_PIDS];
    int count;
} pid_st_map_t;

static void pid_st_map_insert(pid_st_map_t *m, uint16_t pid, uint8_t st) {
    /* Only insert if not already present (first ProgramMap wins). */
    for (int i = 0; i < m->count; i++) {
        if (m->entries[i].pid == pid) return;
    }
    if (m->count < MAX_PIDS) {
        m->entries[m->count].pid = pid;
        m->entries[m->count].stream_type_byte = st;
        m->count++;
    }
}

static uint8_t pid_st_map_lookup(const pid_st_map_t *m, uint16_t pid, uint8_t fallback) {
    for (int i = 0; i < m->count; i++) {
        if (m->entries[i].pid == pid) return m->entries[i].stream_type_byte;
    }
    return fallback;
}

/* ── Core event list ─────────────────────────────────────────────────────────
 *
 * A simple growable array of normalised CoreEvent structs, matching the golden
 * envelope's "core" array. Each event carries just the fields that the
 * normalised golden checks.
 */

typedef enum {
    CE_VIDEO = 0,
    CE_AUDIO,
    CE_KLV,
    CE_UNKNOWN,
    CE_ERROR,
} core_event_kind_t;

typedef struct {
    core_event_kind_t kind;
    /* Common fields */
    uint16_t program;
    uint16_t pid;
    char     stream_type[16];  /* "0x1b" etc. */
    /* Video / Audio */
    int64_t  pts;
    int      key;              /* 1=true, 0=false */
    char     payload_sha256[65];
    /* KLV */
    char     set[32];          /* "st0601" or "unknown" */
    /* Error */
    char     code[64];         /* "STRICT_REJECTION" etc. */
} core_event_t;

#define MAX_CORE_EVENTS 256

typedef struct {
    core_event_t events[MAX_CORE_EVENTS];
    int count;
} core_event_list_t;

static int core_event_list_push(core_event_list_t *list, const core_event_t *ev) {
    if (list->count >= MAX_CORE_EVENTS) {
        fprintf(stderr, "WARNING: core event list full (>%d events)\n", MAX_CORE_EVENTS);
        return 0;
    }
    list->events[list->count++] = *ev;
    return 1;
}

/* ── MISB ST 0601 UL prefix (first 13 bytes) ────────────────────────────────
 *
 * Used to identify the KLV set from the raw payload bytes — binding-neutral
 * (the same 16-byte Universal Label is visible to C, Python, and Rust adapters).
 * Returns "st0601" if the payload starts with this prefix, "unknown" otherwise.
 */
static const uint8_t ST0601_UL_PREFIX[13] = {
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01,
    0x0E, 0x01, 0x03, 0x01, 0x01,
};

static const char *klv_set_from_ul(const uint8_t *payload, size_t payload_len) {
    /* The ST 0601 set is identified by the first 13 bytes of its Universal
     * Label key. Bytes 14-16 carry the version and are NOT matched here —
     * only the stable 13-byte prefix matters for set identification. */
    if (payload_len >= sizeof(ST0601_UL_PREFIX)
        && memcmp(payload, ST0601_UL_PREFIX, sizeof(ST0601_UL_PREFIX)) == 0) {
        return "st0601";
    }
    return "unknown";
}

/* ── Demux scenario runner ───────────────────────────────────────────────────
 *
 * Two-pass approach (same as Rust/Python):
 *   Pass 1: Feed all bytes, flush, drain ALL events into a temporary
 *           buffer, building the pid→stream_type map from ProgramMap events.
 *   Pass 2: Walk the buffered events and emit CoreEvents for media events.
 *
 * Why two passes: a ProgramMap event may appear after a Sample event when
 * scanning the ring buffer order; the Rust normaliser has the same
 * observation and drains all events before the final mapping pass.
 */

/* Temporary buffer for raw events so we can do two passes. */
#define MAX_RAW_EVENTS 512

typedef struct {
    tst_event_t event;

    /* We must copy payload data because it's borrowed and only valid
     * until the next tst_demuxer_next_event / tst_demuxer_close call.
     * For this adapter we need:
     *   - Sample: NAL payloads (for sha256 computation)
     *   - Metadata: KLV payload (for UL prefix detection)
     *   - ProgramMap: stream_info entries (for pid→stream_type map)
     *
     * We avoid heap allocation inside the event buffer by storing
     * pre-computed data here: for Samples we store the sha256 of the
     * concatenated NAL/OBU payloads; for Metadata we store the first
     * 32 bytes of payload (enough for the 16-byte UL + BER length);
     * for ProgramMap we store the stream_count entries inline (up to
     * MAX_STREAMS_PER_PMT per event).
     */

    /* Sample: pre-computed sha256 of concatenated NAL/OBU payloads. */
    char sample_payload_sha256[65];

    /* Metadata: copy of first 32 bytes of KLV payload (UL is 16 B). */
    uint8_t metadata_payload_prefix[32];
    size_t  metadata_payload_prefix_len;

    /* ProgramMap: inline copy of stream_info entries. */
#define MAX_STREAMS_PER_PMT 32
    tst_stream_info_t pmt_streams[MAX_STREAMS_PER_PMT];
    size_t pmt_stream_count;
} raw_event_t;

/* Compute the sha256 of a SAMPLE event's payload (video OR audio).
 *
 * MUST be called in Pass 1, while the borrowed arena pointers
 * (nals[i].payload / obus[i].payload / sample.payload) are still valid —
 * i.e. before the next tst_demuxer_next_event or tst_demuxer_close call.
 * Pass 2 reads the precomputed digest; it must NEVER touch the arena
 * pointers after the demuxer is closed (arena-lifetime contract).
 *
 * Video — WHY concatenation: the Rust normaliser's video_payload_bytes()
 * concatenates all NAL unit payloads (RBSP bytes, Annex-B start codes already
 * stripped) and sha256s the result. The C ABI exposes the same RBSP bytes on
 * nals[i].payload. For AV1 it uses obus[i].payload. We mirror that exactly.
 *
 * Audio — the Rust normaliser hashes the raw `frames` bytes from
 * SamplePayload::Audio; the C ABI surfaces the same raw frame bytes on
 * sample.payload/payload_len (post-PES stripping).
 */
static void compute_sample_sha256(const tst_event_t *ev, char out_hex[65]) {
    sha256_ctx_t ctx;
    sha256_init(&ctx);
    if (ev->u.sample.stream_kind == TST_STREAM_KIND_VIDEO) {
        if (ev->u.sample.nal_count > 0) {
            /* H.264 / H.265 / H.266 — hash RBSP payload bytes from each NAL.
             * The NAL header byte(s) are NOT included — `tst_nal_t.payload`
             * is the raw RBSP content after start-code and header stripping,
             * same as what the Rust normaliser reads from NalUnit { payload }. */
            for (size_t i = 0; i < ev->u.sample.nal_count; i++) {
                sha256_update(&ctx,
                    ev->u.sample.nals[i].payload,
                    ev->u.sample.nals[i].payload_len);
            }
        } else if (ev->u.sample.obu_count > 0) {
            /* AV1 — hash OBU payloads (header, extension, LEB128 size stripped). */
            for (size_t i = 0; i < ev->u.sample.obu_count; i++) {
                sha256_update(&ctx,
                    ev->u.sample.obus[i].payload,
                    ev->u.sample.obus[i].payload_len);
            }
        } else {
            /* Fallback: raw sample payload bytes (should not be reached for
             * well-formed H.264/H.265/H.266/AV1 streams). */
            sha256_update(&ctx, ev->u.sample.payload, ev->u.sample.payload_len);
        }
    } else if (ev->u.sample.stream_kind == TST_STREAM_KIND_AUDIO) {
        /* Audio — hash the raw frame bytes (same bytes the Rust normaliser
         * hashes from SamplePayload::Audio's `frames`). */
        sha256_update(&ctx, ev->u.sample.payload, ev->u.sample.payload_len);
    }
    uint8_t digest[32];
    sha256_final(&ctx, digest);
    for (int k = 0; k < 32; k++) {
        sprintf(out_hex + k * 2, "%02x", (unsigned)digest[k]);
    }
    out_hex[64] = '\0';
}

/* Run a demux scenario: feed input bytes, collect events, normalise. */
static int run_demux(const char *scenarios_dir_path,
                     const scenario_entry_t *entry,
                     core_event_list_t *out_events) {
    /* Read the input artifact. */
    char *input_path = path_join(scenarios_dir_path, entry->input);
    if (!input_path) return -1;

    size_t input_len = 0;
    uint8_t *input_data = read_file(input_path, &input_len);
    free(input_path);
    if (!input_data) return -1;

    /* Open a default-config offline demuxer. */
    struct TstDemuxer *demuxer = tst_demuxer_open();
    if (!demuxer) {
        fprintf(stderr, "ERROR: tst_demuxer_open() failed: %s\n",
                tst_get_last_error_str());
        free(input_data);
        return -1;
    }

    /* Feed all input bytes in a single call.
     * Returns negative (TST_E_* code) on error (e.g. Unrecoverable sync loss). */
    int feed_rc = tst_demuxer_feed(demuxer, input_data, input_len);
    free(input_data);

    if (feed_rc < 0) {
        /* Feed error: map to STRICT_REJECTION umbrella code.
         * This mirrors the Rust/Python adapters which catch DemuxError from
         * feed() and return [{"event":"error","code":"STRICT_REJECTION"}]. */
        tst_demuxer_close(demuxer);
        core_event_t err_ev;
        memset(&err_ev, 0, sizeof(err_ev));
        err_ev.kind = CE_ERROR;
        strncpy(err_ev.code, "STRICT_REJECTION", sizeof(err_ev.code) - 1);
        core_event_list_push(out_events, &err_ev);
        return 0; /* Error correctly mapped — not a program failure */
    }

    /* Flush to materialise any PES boundary event at end-of-input. */
    tst_demuxer_flush(demuxer);

    /* ─── Pass 1: drain all events into a temporary buffer ─────────────────
     *
     * We must copy payload data here because event pointers are only valid
     * until the next tst_demuxer_next_event or tst_demuxer_close call. We
     * pre-compute sha256 of NAL payloads while the data is still borrowed,
     * and copy the first 32 bytes of KLV payload for UL detection.
     * ProgramMap stream_info entries are copied inline (stream_type field).
     */
    static raw_event_t raw_events[MAX_RAW_EVENTS];
    int raw_count = 0;

    tst_event_t ev;
    memset(&ev, 0, sizeof(ev));
    while (1) {
        int rc = tst_demuxer_next_event(demuxer, &ev);
        if (rc == TST_E_NOT_AVAILABLE) break; /* drained */
        if (rc != 0) {
            fprintf(stderr, "WARNING: tst_demuxer_next_event rc=%d\n", rc);
            break;
        }
        if (raw_count >= MAX_RAW_EVENTS) {
            fprintf(stderr, "WARNING: raw event buffer full (>%d)\n", MAX_RAW_EVENTS);
            break;
        }

        raw_event_t *re = &raw_events[raw_count];
        memset(re, 0, sizeof(*re));
        re->event = ev; /* struct copy — copies scalars; pointers borrow the arena */

        /* Per-kind payload capture while pointers are still valid. */
        switch (ev.kind) {
            case TST_EVENT_KIND_SAMPLE:
                /* Compute sha256 of the sample payload (video NAL/OBU bytes or
                 * audio frame bytes) NOW, before the next next_event call — and
                 * crucially before tst_demuxer_close — invalidates the borrowed
                 * arena pointers. Pass 2 reads only this precomputed digest. */
                if (ev.u.sample.stream_kind == TST_STREAM_KIND_VIDEO
                    || ev.u.sample.stream_kind == TST_STREAM_KIND_AUDIO) {
                    compute_sample_sha256(&ev, re->sample_payload_sha256);
                }
                break;

            case TST_EVENT_KIND_METADATA:
                /* Copy the first 32 bytes of KLV payload for UL prefix
                 * detection. 16-byte UL + 1-byte BER length = 17 bytes minimum;
                 * 32 bytes is ample headroom. */
                {
                    size_t copy_len = ev.u.metadata.payload_len;
                    if (copy_len > sizeof(re->metadata_payload_prefix)) {
                        copy_len = sizeof(re->metadata_payload_prefix);
                    }
                    if (ev.u.metadata.payload && copy_len > 0) {
                        memcpy(re->metadata_payload_prefix,
                               ev.u.metadata.payload, copy_len);
                    }
                    re->metadata_payload_prefix_len = copy_len;
                }
                break;

            case TST_EVENT_KIND_PROGRAM_MAP:
                /* Copy stream_info entries for pid→stream_type mapping. */
                {
                    size_t sc = ev.u.program_map.stream_count;
                    if (sc > MAX_STREAMS_PER_PMT) sc = MAX_STREAMS_PER_PMT;
                    if (ev.u.program_map.streams && sc > 0) {
                        memcpy(re->pmt_streams, ev.u.program_map.streams,
                               sc * sizeof(tst_stream_info_t));
                    }
                    re->pmt_stream_count = sc;
                }
                break;

            default:
                break;
        }

        raw_count++;
    }

    tst_demuxer_close(demuxer);

    /* ─── Pass 1b: build pid → stream_type map from ProgramMap events ──── */
    pid_st_map_t st_map;
    memset(&st_map, 0, sizeof(st_map));
    for (int i = 0; i < raw_count; i++) {
        const raw_event_t *re = &raw_events[i];
        if (re->event.kind != TST_EVENT_KIND_PROGRAM_MAP) continue;
        for (size_t j = 0; j < re->pmt_stream_count; j++) {
            pid_st_map_insert(&st_map,
                re->pmt_streams[j].pid,
                re->pmt_streams[j].stream_type);
        }
    }

    /* ─── Pass 2: emit CoreEvents for media events ─────────────────────── */
    for (int i = 0; i < raw_count; i++) {
        const raw_event_t *re = &raw_events[i];
        const tst_event_t *e = &re->event;

        switch (e->kind) {
            case TST_EVENT_KIND_PROGRAM_MAP:
                /* Skip — topology event, not in the media golden. */
                break;

            case TST_EVENT_KIND_SAMPLE: {
                int sk = e->u.sample.stream_kind;
                core_event_t cev;
                memset(&cev, 0, sizeof(cev));
                cev.program = e->u.sample.program_number;
                cev.pid     = e->u.sample.pid;

                if (sk == TST_STREAM_KIND_VIDEO) {
                    /* Video fallback stream_type: H.264 = 0x1B, H.265 = 0x24,
                     * H.266 = 0x33, AV1 = 0x06. These match the Rust fallback
                     * table in video_codec_pmt_byte(). */
                    uint8_t fallback;
                    switch (e->u.sample.codec) {
                        case TST_VIDEO_CODEC_H264: fallback = 0x1B; break;
                        case TST_VIDEO_CODEC_H265: fallback = 0x24; break;
                        case TST_VIDEO_CODEC_H266: fallback = 0x33; break;
                        case TST_VIDEO_CODEC_AV1:  fallback = 0x06; break;
                        default:                   fallback = 0x1B; break;
                    }
                    uint8_t st = pid_st_map_lookup(&st_map, e->u.sample.pid, fallback);
                    snprintf(cev.stream_type, sizeof(cev.stream_type), "0x%02x", st);
                    cev.kind = CE_VIDEO;
                    cev.pts  = e->u.sample.pts;
                    cev.key  = e->u.sample.random_access_indicator ? 1 : 0;
                    strncpy(cev.payload_sha256, re->sample_payload_sha256,
                            sizeof(cev.payload_sha256) - 1);
                    core_event_list_push(out_events, &cev);

                } else if (sk == TST_STREAM_KIND_AUDIO) {
                    /* Audio fallback stream_type: MP2 = 0x03, AAC = 0x0F,
                     * AAC-LATM = 0x11, AC-3 = 0x81. */
                    uint8_t fallback;
                    switch (e->u.sample.codec) {
                        case TST_AUDIO_CODEC_MP2:      fallback = 0x03; break;
                        case TST_AUDIO_CODEC_AAC:      fallback = 0x0F; break;
                        case TST_AUDIO_CODEC_AAC_LATM: fallback = 0x11; break;
                        case TST_AUDIO_CODEC_AC3:      fallback = 0x81; break;
                        default:                       fallback = 0x03; break;
                    }
                    uint8_t st = pid_st_map_lookup(&st_map, e->u.sample.pid, fallback);
                    snprintf(cev.stream_type, sizeof(cev.stream_type), "0x%02x", st);
                    cev.kind = CE_AUDIO;
                    cev.pts  = e->u.sample.pts;
                    /* Read the digest precomputed in Pass 1. The arena pointers
                     * (e->u.sample.payload) are already freed by tst_demuxer_close;
                     * we must NEVER dereference them here. */
                    strncpy(cev.payload_sha256, re->sample_payload_sha256,
                            sizeof(cev.payload_sha256) - 1);
                    core_event_list_push(out_events, &cev);

                } else if (sk == TST_STREAM_KIND_UNKNOWN) {
                    /* Unknown stream: emit with just the pid. */
                    cev.kind = CE_UNKNOWN;
                    core_event_list_push(out_events, &cev);

                }
                /* Subtitle (TST_STREAM_KIND_SUBTITLE): skip — matches Rust/Python. */
                break;
            }

            case TST_EVENT_KIND_METADATA: {
                /* KLV metadata event. stream_type fallback:
                 *   KlvSync (metadata_kind == KLV_SYNC_AU_CELL) → 0x15
                 *   KlvAsync (metadata_kind == KLV_ASYNC)        → 0x06
                 * The C ABI exposes metadata_kind on the metadata sub-struct. */
                uint8_t fallback;
                if (e->u.metadata.metadata_kind == TST_METADATA_KIND_KLV_SYNC_AU_CELL) {
                    fallback = 0x15;
                } else {
                    fallback = 0x06; /* KlvAsync + Unknown both map to 0x06 */
                }
                uint8_t st = pid_st_map_lookup(&st_map, e->u.metadata.pid, fallback);

                core_event_t cev;
                memset(&cev, 0, sizeof(cev));
                cev.kind    = CE_KLV;
                cev.program = e->u.metadata.program_number;
                cev.pid     = e->u.metadata.pid;
                snprintf(cev.stream_type, sizeof(cev.stream_type), "0x%02x", st);
                strncpy(cev.set,
                    klv_set_from_ul(re->metadata_payload_prefix,
                                    re->metadata_payload_prefix_len),
                    sizeof(cev.set) - 1);
                core_event_list_push(out_events, &cev);
                break;
            }

            case TST_EVENT_KIND_DISCONTINUITY:
            case TST_EVENT_KIND_NON_CONFORMANT:
            case TST_EVENT_KIND_RECONNECT_DISCONTINUITY:
                /* Diagnostics — skip from the media golden (same as Rust/Python). */
                break;

            default:
                fprintf(stderr, "WARNING: unknown event kind %d\n", e->kind);
                break;
        }
    }

    return 0;
}

/* Synthetic H.264 IDR AU — MUST match tst-integration's synthetic_h264_idr():
 * 4-byte Annex-B start code + IDR NAL header (0x65) + 15 bytes 0xA5 ^ i. */
static size_t synth_h264_idr(uint8_t out[20]) {
    out[0] = 0x00; out[1] = 0x00; out[2] = 0x00; out[3] = 0x01;
    out[4] = 0x65;
    for (uint8_t i = 0; i < 15; i++) out[5 + i] = (uint8_t)(0xA5 ^ i);
    return 20;
}

/* ── Roundtrip scenario runner ── */
static int run_roundtrip(const char *scenarios_dir_path,
                         const scenario_entry_t *entry,
                         const char *golden_json,
                         core_event_list_t *out_events) {
    (void)out_events; /* roundtrip carries no media events — core: [] */

    /* Extract expected sha256 from golden.extensions.output_sha256. */
    char expected_sha256[65];
    size_t ext_off = json_extract_string(golden_json, 0,
        "output_sha256", expected_sha256, sizeof(expected_sha256));
    if (ext_off == 0 || expected_sha256[0] == '\0') {
        fprintf(stderr, "ERROR: cannot extract extensions.output_sha256 from golden\n");
        return -1;
    }

    /* Read the committed artifact (entry->input IS output.ts for roundtrip). */
    char *artifact_path = path_join(scenarios_dir_path, entry->input);
    if (!artifact_path) return -1;
    size_t committed_len = 0;
    uint8_t *committed = read_file(artifact_path, &committed_len);
    free(artifact_path);
    if (!committed) return -1;

#if defined(TST_HAS_SRT)
    /* ── Re-mux the recipe in C and compare to the committed bytes. ──────── */
    struct tst_mux_config_t *cfg = tst_mux_config_new();
    if (!cfg) { fprintf(stderr, "ERROR: tst_mux_config_new failed\n"); free(committed); return -1; }
    tst_program_handle_t prog = tst_mux_config_add_program(cfg, 1, 0x1000);
    if (prog == TST_INVALID_PROGRAM_HANDLE) {
        fprintf(stderr, "ERROR: add_program failed\n"); tst_mux_config_free(cfg); free(committed); return -1;
    }
    tst_video_stream_handle_t vstream =
        tst_mux_config_add_video_stream(cfg, prog, 0x1011, TST_VIDEO_CODEC_H264);
    if (vstream == TST_INVALID_STREAM_HANDLE) {
        fprintf(stderr, "ERROR: add_video_stream failed\n"); tst_mux_config_free(cfg); free(committed); return -1;
    }
    struct tst_muxer_t *mux = tst_muxer_open(cfg);
    tst_mux_config_free(cfg); /* config is consumed/copied at open time */
    if (!mux) { fprintf(stderr, "ERROR: tst_muxer_open failed\n"); free(committed); return -1; }

    uint8_t idr[20];
    size_t idr_len = synth_h264_idr(idr);
    if (tst_muxer_push_video(mux, idr, idr_len, /*pts_90khz=*/0, /*key_frame=*/true) != 0) {
        fprintf(stderr, "ERROR: tst_muxer_push_video failed\n"); tst_muxer_close(mux); free(committed); return -1;
    }

    /* Drain with a 1316-byte (7×188) pull loop, matching the Rust drain_mux. */
    uint8_t *produced = NULL;
    size_t produced_len = 0, produced_cap = 0;
    uint8_t pull_buf[1316];
    for (;;) {
        size_t n = tst_muxer_pull(mux, pull_buf, sizeof(pull_buf));
        if (n == 0) break;
        if (produced_len + n > produced_cap) {
            size_t new_cap = (produced_cap == 0) ? 65536 : produced_cap * 2;
            while (new_cap < produced_len + n) new_cap *= 2;
            uint8_t *grown = realloc(produced, new_cap);
            if (!grown) { fprintf(stderr, "ERROR: OOM draining mux\n"); free(produced); tst_muxer_close(mux); free(committed); return -1; }
            produced = grown; produced_cap = new_cap;
        }
        memcpy(produced + produced_len, pull_buf, n);
        produced_len += n;
    }
    tst_muxer_close(mux);

    /* Byte-for-byte parity with the committed artifact. */
    if (produced_len != committed_len || memcmp(produced, committed, produced_len) != 0) {
        fprintf(stderr,
            "FAIL [%s]: C-produced bytes differ from committed output.ts "
            "(produced %zu bytes, committed %zu bytes)\n",
            entry->id, produced_len, committed_len);
        free(produced); free(committed); return -1;
    }

    /* sha256 parity with the golden. */
    char digest_hex[65];
    sha256_hex(produced, produced_len, digest_hex);
    free(produced); free(committed);
    if (strcmp(digest_hex, expected_sha256) != 0) {
        fprintf(stderr, "FAIL [%s]: sha256 mismatch\n  computed : %s\n  expected : %s\n",
            entry->id, digest_hex, expected_sha256);
        return -1;
    }
    return 0;
#else
    /* ── No srt feature: fall back to hashing the committed artifact. ────── */
    char digest_hex[65];
    sha256_hex(committed, committed_len, digest_hex);
    free(committed);
    if (strcmp(digest_hex, expected_sha256) != 0) {
        fprintf(stderr, "FAIL [%s]: sha256 mismatch (artifact-hash fallback)\n"
            "  computed : %s\n  expected : %s\n", entry->id, digest_hex, expected_sha256);
        return -1;
    }
    return 0;
#endif
}

/* ── Binding contract (strict-rejection) runner ──────────────────────────────
 *
 * Feed 8192 × 0xFF through tst_demuxer_feed with a default-config demuxer.
 * The all-0xFF input has no 0x47 MPEG-TS sync byte. After scanning
 * SYNC_SEARCH_WINDOW (188 × 32 = 6016 bytes) without a sync byte, the demuxer
 * returns a negative (Unrecoverable) error code. Any negative return from feed
 * maps to the umbrella public code "STRICT_REJECTION" — the same mapping used
 * by the Rust and Python adapters.
 *
 * Also verifies two idempotence/safety contracts:
 *   1. tst_demuxer_close(NULL) is safe (null-pointer guard documented in header).
 *   2. tst_demuxer_close on an error-state demuxer does not crash.
 */
static int run_binding_contract(const char *scenarios_dir_path,
                                const scenario_entry_t *entry,
                                core_event_list_t *out_events) {
    if (strcmp(entry->id, "strict-rejection") != 0) {
        fprintf(stderr, "ERROR: unknown binding_contract scenario: %s\n", entry->id);
        return -1;
    }

    /* Read the garbage input artifact (8192 × 0xFF). */
    char *input_path = path_join(scenarios_dir_path, entry->input);
    if (!input_path) return -1;

    size_t input_len = 0;
    uint8_t *input_data = read_file(input_path, &input_len);
    free(input_path);
    if (!input_data) return -1;

    /* Verify null-safe close BEFORE any demuxer is open — confirming the
     * null-pointer guard works independent of the error-path test below. */
    tst_demuxer_close(NULL); /* must not crash */

    /* Open a default-config demuxer. */
    struct TstDemuxer *demuxer = tst_demuxer_open();
    if (!demuxer) {
        fprintf(stderr, "ERROR: tst_demuxer_open() failed: %s\n",
                tst_get_last_error_str());
        free(input_data);
        return -1;
    }

    /* Feed the garbage bytes. Must return a negative code. */
    int feed_rc = tst_demuxer_feed(demuxer, input_data, input_len);
    free(input_data);

    if (feed_rc >= 0) {
        /* Feed didn't error — unexpected for 8192 × 0xFF. */
        fprintf(stderr,
            "FAIL [%s]: expected tst_demuxer_feed to return a negative error "
            "code on garbage input, got %d\n",
            entry->id, feed_rc);
        tst_demuxer_close(demuxer);
        return -1;
    }

    /* feed_rc < 0: error mapped to STRICT_REJECTION umbrella code. */
    fprintf(stdout, "  [strict-rejection] tst_demuxer_feed returned %d — "
            "maps to STRICT_REJECTION\n", feed_rc);

    /* Close the error-state demuxer. Must not crash (idempotence contract). */
    tst_demuxer_close(demuxer);

    /* Emit the single error CoreEvent. */
    core_event_t err_ev;
    memset(&err_ev, 0, sizeof(err_ev));
    err_ev.kind = CE_ERROR;
    strncpy(err_ev.code, "STRICT_REJECTION", sizeof(err_ev.code) - 1);
    core_event_list_push(out_events, &err_ev);

    return 0;
}

/* ── Golden comparison ───────────────────────────────────────────────────────
 *
 * For each scenario we compare observed CoreEvents against the committed
 * golden.json field-by-field. The golden's "core" array is parsed from JSON
 * and checked against the normalised C-adapter output.
 *
 * For the demux scenario, this means comparing:
 *   video: program, pid, stream_type, pts, key, payload_sha256
 *   klv:   program, pid, stream_type, set
 *   error: code
 */

/* Parse and count the golden's "core" events; fill `out_events` up to
 * `max_events` entries. Returns the count. */
static int parse_golden_core(const char *json,
                             core_event_t *out_events, int max_events) {
    int count = 0;
    /* Find "core":[  */
    const char *core_start = strstr(json, "\"core\":");
    if (!core_start) return 0;
    core_start = strchr(core_start, '[');
    if (!core_start) return 0;
    core_start++; /* skip '[' */

    const char *p = core_start;
    /* Walk each {...} object in the array. */
    while (*p && count < max_events) {
        /* Skip whitespace and commas between objects. */
        while (*p == ' ' || *p == '\n' || *p == '\r' || *p == '\t' || *p == ',') p++;
        if (*p == ']') break; /* end of array */
        if (*p != '{') { p++; continue; }

        /* Find the matching '}'. */
        const char *obj_start = p;
        int depth = 0;
        while (*p) {
            if (*p == '{') depth++;
            else if (*p == '}') { depth--; if (depth == 0) { p++; break; } }
            p++;
        }
        size_t obj_len = (size_t)(p - obj_start);
        /* Copy the object to a NUL-terminated buffer. */
        char obj_buf[512];
        if (obj_len >= sizeof(obj_buf)) {
            fprintf(stderr, "WARNING: golden core object too large, skipping\n");
            continue;
        }
        memcpy(obj_buf, obj_start, obj_len);
        obj_buf[obj_len] = '\0';

        /* Extract the "event" tag. */
        core_event_t ev;
        memset(&ev, 0, sizeof(ev));
        char event_tag[32];
        json_extract_string(obj_buf, 0, "event", event_tag, sizeof(event_tag));

        if (strcmp(event_tag, "video") == 0) {
            ev.kind = CE_VIDEO;
            int64_t prog64 = 0, pid64 = 0;
            json_extract_int64(obj_buf, 0, "program", &prog64);
            json_extract_int64(obj_buf, 0, "pid",     &pid64);
            ev.program = (uint16_t)prog64;
            ev.pid     = (uint16_t)pid64;
            json_extract_string(obj_buf, 0, "stream_type",
                                ev.stream_type, sizeof(ev.stream_type));
            json_extract_int64(obj_buf, 0, "pts", &ev.pts);
            int key_val = 0;
            json_extract_bool(obj_buf, 0, "key", &key_val);
            ev.key = key_val;
            json_extract_string(obj_buf, 0, "payload_sha256",
                                ev.payload_sha256, sizeof(ev.payload_sha256));

        } else if (strcmp(event_tag, "audio") == 0) {
            ev.kind = CE_AUDIO;
            int64_t prog64 = 0, pid64 = 0;
            json_extract_int64(obj_buf, 0, "program", &prog64);
            json_extract_int64(obj_buf, 0, "pid",     &pid64);
            ev.program = (uint16_t)prog64;
            ev.pid     = (uint16_t)pid64;
            json_extract_string(obj_buf, 0, "stream_type",
                                ev.stream_type, sizeof(ev.stream_type));
            json_extract_int64(obj_buf, 0, "pts", &ev.pts);
            json_extract_string(obj_buf, 0, "payload_sha256",
                                ev.payload_sha256, sizeof(ev.payload_sha256));

        } else if (strcmp(event_tag, "klv") == 0) {
            ev.kind = CE_KLV;
            int64_t prog64 = 0, pid64 = 0;
            json_extract_int64(obj_buf, 0, "program", &prog64);
            json_extract_int64(obj_buf, 0, "pid",     &pid64);
            ev.program = (uint16_t)prog64;
            ev.pid     = (uint16_t)pid64;
            json_extract_string(obj_buf, 0, "stream_type",
                                ev.stream_type, sizeof(ev.stream_type));
            json_extract_string(obj_buf, 0, "set", ev.set, sizeof(ev.set));

        } else if (strcmp(event_tag, "unknown") == 0) {
            ev.kind = CE_UNKNOWN;
            int64_t pid64 = 0;
            json_extract_int64(obj_buf, 0, "pid", &pid64);
            ev.pid = (uint16_t)pid64;

        } else if (strcmp(event_tag, "error") == 0) {
            ev.kind = CE_ERROR;
            json_extract_string(obj_buf, 0, "code", ev.code, sizeof(ev.code));

        } else {
            fprintf(stderr,
                "ERROR: golden contains unrecognised event tag '%s'; "
                "this C adapter must be updated to handle it.\n", event_tag);
            /* Fail the comparison rather than silently skipping. */
            ev.kind = CE_ERROR;
            strncpy(ev.code, "UNKNOWN_EVENT_TAG", sizeof(ev.code) - 1);
        }

        out_events[count++] = ev;
    }
    return count;
}

/* Compare two core event lists field-by-field.
 * Returns 1 if identical, 0 if different (and prints a diff). */
static int compare_core_events(const char *scenario_id,
                               const core_event_t *observed, int obs_count,
                               const core_event_t *expected, int exp_count) {
    if (obs_count != exp_count) {
        fprintf(stderr, "FAIL [%s]: core event count mismatch: "
                "observed=%d expected=%d\n",
                scenario_id, obs_count, exp_count);
        return 0;
    }

    int ok = 1;
    for (int i = 0; i < obs_count; i++) {
        const core_event_t *o = &observed[i];
        const core_event_t *e = &expected[i];

        if (o->kind != e->kind) {
            fprintf(stderr, "FAIL [%s] event[%d]: kind mismatch "
                    "(observed=%d expected=%d)\n",
                    scenario_id, i, o->kind, e->kind);
            ok = 0;
            continue;
        }

        switch (o->kind) {
            case CE_VIDEO:
                if (o->program != e->program) {
                    fprintf(stderr, "FAIL [%s] video[%d]: program %u != %u\n",
                            scenario_id, i, o->program, e->program); ok = 0;
                }
                if (o->pid != e->pid) {
                    fprintf(stderr, "FAIL [%s] video[%d]: pid %u != %u\n",
                            scenario_id, i, o->pid, e->pid); ok = 0;
                }
                if (strcmp(o->stream_type, e->stream_type) != 0) {
                    fprintf(stderr, "FAIL [%s] video[%d]: stream_type '%s' != '%s'\n",
                            scenario_id, i, o->stream_type, e->stream_type); ok = 0;
                }
                if (o->pts != e->pts) {
                    fprintf(stderr, "FAIL [%s] video[%d]: pts %" PRId64 " != %" PRId64 "\n",
                            scenario_id, i, o->pts, e->pts); ok = 0;
                }
                if (o->key != e->key) {
                    fprintf(stderr, "FAIL [%s] video[%d]: key %d != %d\n",
                            scenario_id, i, o->key, e->key); ok = 0;
                }
                if (strcmp(o->payload_sha256, e->payload_sha256) != 0) {
                    fprintf(stderr, "FAIL [%s] video[%d]: payload_sha256\n"
                            "  observed : %s\n  expected : %s\n",
                            scenario_id, i, o->payload_sha256, e->payload_sha256); ok = 0;
                }
                break;

            case CE_AUDIO:
                if (o->program != e->program) {
                    fprintf(stderr, "FAIL [%s] audio[%d]: program %u != %u\n",
                            scenario_id, i, o->program, e->program); ok = 0;
                }
                if (o->pid != e->pid) {
                    fprintf(stderr, "FAIL [%s] audio[%d]: pid %u != %u\n",
                            scenario_id, i, o->pid, e->pid); ok = 0;
                }
                if (strcmp(o->stream_type, e->stream_type) != 0) {
                    fprintf(stderr, "FAIL [%s] audio[%d]: stream_type '%s' != '%s'\n",
                            scenario_id, i, o->stream_type, e->stream_type); ok = 0;
                }
                if (o->pts != e->pts) {
                    fprintf(stderr, "FAIL [%s] audio[%d]: pts %" PRId64 " != %" PRId64 "\n",
                            scenario_id, i, o->pts, e->pts); ok = 0;
                }
                if (strcmp(o->payload_sha256, e->payload_sha256) != 0) {
                    fprintf(stderr, "FAIL [%s] audio[%d]: payload_sha256\n"
                            "  observed : %s\n  expected : %s\n",
                            scenario_id, i, o->payload_sha256, e->payload_sha256); ok = 0;
                }
                break;

            case CE_KLV:
                if (o->program != e->program) {
                    fprintf(stderr, "FAIL [%s] klv[%d]: program %u != %u\n",
                            scenario_id, i, o->program, e->program); ok = 0;
                }
                if (o->pid != e->pid) {
                    fprintf(stderr, "FAIL [%s] klv[%d]: pid %u != %u\n",
                            scenario_id, i, o->pid, e->pid); ok = 0;
                }
                if (strcmp(o->stream_type, e->stream_type) != 0) {
                    fprintf(stderr, "FAIL [%s] klv[%d]: stream_type '%s' != '%s'\n",
                            scenario_id, i, o->stream_type, e->stream_type); ok = 0;
                }
                if (strcmp(o->set, e->set) != 0) {
                    fprintf(stderr, "FAIL [%s] klv[%d]: set '%s' != '%s'\n",
                            scenario_id, i, o->set, e->set); ok = 0;
                }
                break;

            case CE_UNKNOWN:
                if (o->pid != e->pid) {
                    fprintf(stderr, "FAIL [%s] unknown[%d]: pid %u != %u\n",
                            scenario_id, i, o->pid, e->pid); ok = 0;
                }
                break;

            case CE_ERROR:
                if (strcmp(o->code, e->code) != 0) {
                    fprintf(stderr, "FAIL [%s] error[%d]: code '%s' != '%s'\n",
                            scenario_id, i, o->code, e->code); ok = 0;
                }
                break;

            default:
                /* Defensive: an out-of-range kind must fail loudly rather than
                 * compare equal. Unreachable today (unknown golden tags are
                 * rejected in parse_golden_core), but a silent pass on a future
                 * kind would be a parity hole. */
                fprintf(stderr, "FAIL [%s] event[%d]: unhandled core event "
                        "kind %d\n", scenario_id, i, o->kind);
                ok = 0;
                break;
        }
    }
    return ok;
}

/* ── Main ────────────────────────────────────────────────────────────────────*/

int main(int argc, char **argv) {
    /* The scenarios directory can be passed as argv[1].
     *
     * Default (no argv[1]): "../../../tst-integration/tests/fixtures/scenarios"
     * relative to the current working directory. This works correctly when:
     *   - CWD = ts-transformer/ workspace root (the most common invocation)
     *   - CWD = ts-transformer/bindings/c/tst-c/ (e.g. after `cd` for the build step)
     * Adjust with argv[1] for any other CWD. The companion run_scenarios.sh sets
     * argv[1] to an absolute path derived from the script's own location.
     */
    const char *scenarios_dir_path = (argc >= 2)
        ? argv[1]
        : "crates/tst-integration/tests/fixtures/scenarios";

    fprintf(stdout, "scenarios dir: %s\n", scenarios_dir_path);

    /* Read scenarios.toml. */
    char *manifest_path = path_join(scenarios_dir_path, "scenarios.toml");
    if (!manifest_path) { fprintf(stderr, "ERROR: OOM\n"); return 1; }

    size_t manifest_len = 0;
    uint8_t *manifest_data = read_file(manifest_path, &manifest_len);
    free(manifest_path);
    if (!manifest_data) return 1;

    /* Parse the manifest. */
    static scenario_entry_t entries[MAX_SCENARIOS];
    int scenario_count = parse_scenarios_toml((const char *)manifest_data, entries);
    free(manifest_data);

    if (scenario_count == 0) {
        fprintf(stderr, "ERROR: no scenarios found in scenarios.toml\n");
        return 1;
    }
    fprintf(stdout, "found %d scenario(s)\n\n", scenario_count);

    /* Run each scenario and compare against its golden. */
    int failures = 0;
    for (int s = 0; s < scenario_count; s++) {
        const scenario_entry_t *entry = &entries[s];
        fprintf(stdout, "--- scenario '%s' (kind=%s)\n", entry->id, entry->kind);

        /* Read golden.json. */
        char *golden_path = path_join(scenarios_dir_path, entry->golden);
        if (!golden_path) { fprintf(stderr, "ERROR: OOM\n"); failures++; continue; }

        size_t golden_len = 0;
        uint8_t *golden_data = read_file(golden_path, &golden_len);
        free(golden_path);
        if (!golden_data) { failures++; continue; }
        const char *golden_json = (const char *)golden_data;

        /* Run the scenario. */
        core_event_list_t observed;
        memset(&observed, 0, sizeof(observed));

        int run_rc = 0;
        if (strcmp(entry->kind, "demux") == 0) {
            run_rc = run_demux(scenarios_dir_path, entry, &observed);
        } else if (strcmp(entry->kind, "roundtrip") == 0) {
            run_rc = run_roundtrip(scenarios_dir_path, entry, golden_json, &observed);
        } else if (strcmp(entry->kind, "binding_contract") == 0) {
            run_rc = run_binding_contract(scenarios_dir_path, entry, &observed);
        } else {
            fprintf(stderr, "ERROR: unknown scenario kind '%s'\n", entry->kind);
            run_rc = -1;
        }

        if (run_rc != 0) {
            fprintf(stderr, "FAIL [%s]: scenario runner returned error\n", entry->id);
            free(golden_data);
            failures++;
            continue;
        }

        /* Parse golden's core events and compare. */
        /* Roundtrip scenario: core is empty — comparison trivially passes if
         * run_roundtrip() returned 0 (which already verified the sha256). */
        static core_event_t golden_core_events[MAX_CORE_EVENTS];
        int golden_event_count = parse_golden_core(golden_json,
            golden_core_events, MAX_CORE_EVENTS);
        free(golden_data);

        int match = compare_core_events(entry->id,
            observed.events, observed.count,
            golden_core_events, golden_event_count);

        if (match) {
            fprintf(stdout, "PASS [%s]\n", entry->id);
        } else {
            failures++;
        }
        fprintf(stdout, "\n");
    }

    /* Summary. */
    fprintf(stdout, "=== %d scenario(s): %d passed, %d failed ===\n",
            scenario_count,
            scenario_count - failures,
            failures);

    return failures > 0 ? 1 : 0;
}
