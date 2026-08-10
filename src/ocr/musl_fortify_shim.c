/* glibc _FORTIFY_SOURCE shims for musl static builds.
 *
 * ocr-rs builds MNN's C++ sources with fortify enabled, which emits calls
 * to glibc's __*_chk functions. musl does not provide them, so the final
 * link of a musl-static any2md fails with undefined references. These
 * shims simply forward to the plain libc functions; the "known object
 * size" arguments are ignored, which is exactly what a _FORTIFY_SOURCE=0
 * build would have produced.
 */

#include <fcntl.h>
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

/* GCC 11's libstdc++ headers reference glibc's __libc_single_threaded for
 * atomic elision (shared_ptr refcounts etc.). musl has no such symbol.
 * Defining it as 0 is the safe answer: "never single-threaded", which only
 * forgoes the elision. */
char __libc_single_threaded;

/* _FORTIFY_SOURCE open(): the two-argument form without the mode check. */
int __open_2(const char *path, int flags) {
    return open(path, flags);
}

int __printf_chk(int flag, const char *fmt, ...) {
    (void)flag;
    va_list ap;
    va_start(ap, fmt);
    int r = vprintf(fmt, ap);
    va_end(ap);
    return r;
}

int __fprintf_chk(FILE *stream, int flag, const char *fmt, ...) {
    (void)flag;
    va_list ap;
    va_start(ap, fmt);
    int r = vfprintf(stream, fmt, ap);
    va_end(ap);
    return r;
}

int __sprintf_chk(char *s, int flag, size_t slen, const char *fmt, ...) {
    (void)flag;
    (void)slen;
    va_list ap;
    va_start(ap, fmt);
    int r = vsprintf(s, fmt, ap);
    va_end(ap);
    return r;
}

int __snprintf_chk(char *s, size_t n, int flag, size_t slen, const char *fmt, ...) {
    (void)flag;
    (void)slen;
    va_list ap;
    va_start(ap, fmt);
    int r = vsnprintf(s, n, fmt, ap);
    va_end(ap);
    return r;
}

int __vprintf_chk(int flag, const char *fmt, va_list ap) {
    (void)flag;
    return vprintf(fmt, ap);
}

int __vfprintf_chk(FILE *stream, int flag, const char *fmt, va_list ap) {
    (void)flag;
    return vfprintf(stream, fmt, ap);
}

int __vsprintf_chk(char *s, int flag, size_t slen, const char *fmt, va_list ap) {
    (void)flag;
    (void)slen;
    return vsprintf(s, fmt, ap);
}

int __vsnprintf_chk(char *s, size_t n, int flag, size_t slen, const char *fmt, va_list ap) {
    (void)flag;
    (void)slen;
    return vsnprintf(s, n, fmt, ap);
}

void *__memcpy_chk(void *dest, const void *src, size_t n, size_t destlen) {
    (void)destlen;
    return memcpy(dest, src, n);
}

void *__memmove_chk(void *dest, const void *src, size_t n, size_t destlen) {
    (void)destlen;
    return memmove(dest, src, n);
}

void *__memset_chk(void *s, int c, size_t n, size_t slen) {
    (void)slen;
    return memset(s, c, n);
}

char *__strcpy_chk(char *dest, const char *src, size_t destlen) {
    (void)destlen;
    return strcpy(dest, src);
}

char *__strncpy_chk(char *dest, const char *src, size_t n, size_t destlen) {
    (void)destlen;
    return strncpy(dest, src, n);
}

char *__strcat_chk(char *dest, const char *src, size_t destlen) {
    (void)destlen;
    return strcat(dest, src);
}

char *__strncat_chk(char *dest, const char *src, size_t n, size_t destlen) {
    (void)destlen;
    return strncat(dest, src, n);
}
