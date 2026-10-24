// membw_read.c — streaming-read bandwidth with AVX2 loads, as a cross-check of
// llama-roofline's numpy ceiling. Usage: membw_read [MiB=1024] [reps=7] threads...
// Build: gcc -O2 -mavx2 -fopenmp membw_read.c -o "$(python3 bench_paths.py data)/membw_read"
#include <immintrin.h>
#include <omp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}

int main(int argc, char **argv) {
    size_t mib = argc > 1 ? strtoul(argv[1], 0, 10) : 1024;
    int reps = argc > 2 ? atoi(argv[2]) : 7;
    if (reps < 1 || reps > 64) reps = reps < 1 ? 1 : 64;  // samples[] holds 64
    size_t bytes = mib << 20, n = bytes / 32;
    __m256i *buf = aligned_alloc(64, bytes);
    if (!buf) return 1;
    memset(buf, 1, bytes);  // fault every page in before timing
    printf("{\"working_set_mib\":%zu,\"reps\":%d,\"runs\":[", mib, reps);
    for (int a = 3; a < argc; a++) {
        int threads = atoi(argv[a]);
        double best = 0, samples[64];
        for (int r = 0; r < reps; r++) {
            __m256i total = _mm256_setzero_si256();
            double t0 = now();
#pragma omp parallel num_threads(threads)
            {
                __m256i s0 = _mm256_setzero_si256(), s1 = s0, s2 = s0, s3 = s0;
#pragma omp for schedule(static) nowait
                for (size_t i = 0; i < n; i += 4) {
                    s0 = _mm256_xor_si256(s0, _mm256_load_si256(buf + i));
                    s1 = _mm256_xor_si256(s1, _mm256_load_si256(buf + i + 1));
                    s2 = _mm256_xor_si256(s2, _mm256_load_si256(buf + i + 2));
                    s3 = _mm256_xor_si256(s3, _mm256_load_si256(buf + i + 3));
                }
                __m256i s = _mm256_xor_si256(_mm256_xor_si256(s0, s1), _mm256_xor_si256(s2, s3));
#pragma omp critical
                total = _mm256_xor_si256(total, s);
            }
            double gbs = bytes / (now() - t0) / 1e9;
            samples[r] = gbs;
            if (gbs > best) best = gbs;
            // Keep the reduction observable so the loads are not removed.
            if (_mm256_extract_epi32(total, 0) == 42) fputc(' ', stderr);
        }
        printf("%s{\"threads\":%d,\"best_GBs\":%.2f,\"samples\":[", a > 3 ? "," : "", threads, best);
        for (int r = 0; r < reps; r++) printf("%s%.2f", r ? "," : "", samples[r]);
        printf("]}");
    }
    printf("]}\n");
    free(buf);
    return 0;
}
