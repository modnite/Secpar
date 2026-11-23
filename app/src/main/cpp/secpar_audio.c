// AAudio output for stream audio: 48 kHz stereo 16-bit, low latency. On devices with an MMAP path
// (Chromecast with Google TV, most recent phones) this skips the mixer's large blocks, which
// AudioTrack cannot: about 37 ms of output delay on a Chromecast instead of about 130 ms.
// Every call is non-blocking; the Kotlin side owns pacing and recovery.
#include <aaudio/AAudio.h>
#include <dlfcn.h>
#include <jni.h>
#include <stdbool.h>
#include <stdint.h>
#include <time.h>

static AAudioStream *stream(jlong handle) { return (AAudioStream *)(intptr_t)handle; }

JNIEXPORT jlong JNICALL Java_com_pcremote_prs_AAudioNative_open(JNIEnv *env, jobject self, jint rate, jint bufferFrames) {
    AAudioStreamBuilder *b;
    if (AAudio_createStreamBuilder(&b) != AAUDIO_OK) return 0;
    AAudioStreamBuilder_setDirection(b, AAUDIO_DIRECTION_OUTPUT);
    AAudioStreamBuilder_setSampleRate(b, rate);
    AAudioStreamBuilder_setChannelCount(b, 2);
    AAudioStreamBuilder_setFormat(b, AAUDIO_FORMAT_PCM_I16);
    AAudioStreamBuilder_setPerformanceMode(b, AAUDIO_PERFORMANCE_MODE_LOW_LATENCY);
    AAudioStreamBuilder_setSharingMode(b, AAUDIO_SHARING_MODE_SHARED);
    if (__builtin_available(android 28, *)) {
        AAudioStreamBuilder_setUsage(b, AAUDIO_USAGE_GAME);
        AAudioStreamBuilder_setContentType(b, AAUDIO_CONTENT_TYPE_MUSIC);
    }
    AAudioStream *s = 0;
    aaudio_result_t r = AAudioStreamBuilder_openStream(b, &s);
    AAudioStreamBuilder_delete(b);
    if (r != AAUDIO_OK || !s) return 0;
    if (AAudioStream_getSampleRate(s) != rate || AAudioStream_getChannelCount(s) != 2 ||
        AAudioStream_getFormat(s) != AAUDIO_FORMAT_PCM_I16) { AAudioStream_close(s); return 0; }
    // Room for the player's whole queue; the player keeps it short itself.
    AAudioStream_setBufferSizeInFrames(s, bufferFrames);
    return (jlong)(intptr_t)s;
}

/** Writes up to [bytes] from [data] at [offset] without waiting; bytes accepted, or a negative AAudio error. */
JNIEXPORT jint JNICALL Java_com_pcremote_prs_AAudioNative_write(JNIEnv *env, jobject self, jlong h, jbyteArray data, jint offset, jint bytes) {
    jbyte *p = (*env)->GetPrimitiveArrayCritical(env, data, 0);
    if (!p) return AAUDIO_ERROR_INTERNAL;
    aaudio_result_t r = AAudioStream_write(stream(h), p + offset, bytes / 4, 0);
    (*env)->ReleasePrimitiveArrayCritical(env, data, p, JNI_ABORT);
    return r < 0 ? r : r * 4;
}

JNIEXPORT jint JNICALL Java_com_pcremote_prs_AAudioNative_start(JNIEnv *env, jobject self, jlong h) { return AAudioStream_requestStart(stream(h)); }
JNIEXPORT jlong JNICALL Java_com_pcremote_prs_AAudioNative_framesRead(JNIEnv *env, jobject self, jlong h) { return AAudioStream_getFramesRead(stream(h)); }
JNIEXPORT jlong JNICALL Java_com_pcremote_prs_AAudioNative_framesWritten(JNIEnv *env, jobject self, jlong h) { return AAudioStream_getFramesWritten(stream(h)); }
JNIEXPORT jint JNICALL Java_com_pcremote_prs_AAudioNative_underruns(JNIEnv *env, jobject self, jlong h) { return AAudioStream_getXRunCount(stream(h)); }
JNIEXPORT jint JNICALL Java_com_pcremote_prs_AAudioNative_burst(JNIEnv *env, jobject self, jlong h) { return AAudioStream_getFramesPerBurst(stream(h)); }
JNIEXPORT jint JNICALL Java_com_pcremote_prs_AAudioNative_bufferFrames(JNIEnv *env, jobject self, jlong h) { return AAudioStream_getBufferSizeInFrames(stream(h)); }
JNIEXPORT jint JNICALL Java_com_pcremote_prs_AAudioNative_deviceId(JNIEnv *env, jobject self, jlong h) { return AAudioStream_getDeviceId(stream(h)); }
JNIEXPORT jint JNICALL Java_com_pcremote_prs_AAudioNative_state(JNIEnv *env, jobject self, jlong h) { return AAudioStream_getState(stream(h)); }

/** Hardware output delay in microseconds from the stream's timestamp, or -1 before it has one. */
JNIEXPORT jlong JNICALL Java_com_pcremote_prs_AAudioNative_latencyUs(JNIEnv *env, jobject self, jlong h) {
    int64_t position, timeNs;
    if (AAudioStream_getTimestamp(stream(h), CLOCK_MONOTONIC, &position, &timeNs) != AAUDIO_OK) return -1;
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    int64_t nowNs = t.tv_sec * 1000000000LL + t.tv_nsec;
    int64_t written = AAudioStream_getFramesWritten(stream(h));
    int32_t rate = AAudioStream_getSampleRate(stream(h));
    return (written - position) * 1000000LL / rate - (nowNs - timeNs) / 1000;
}

/**
 * Whether the stream runs on the MMAP path. Only that path is worth having: without it AAudio
 * wraps an AudioTrack that waits for a full buffer before it starts. The check is exported by
 * libaaudio but not in the NDK headers; where it is missing the answer is no.
 */
JNIEXPORT jboolean JNICALL Java_com_pcremote_prs_AAudioNative_mmapUsed(JNIEnv *env, jobject self, jlong h) {
    static bool (*used)(AAudioStream *) = 0;
    static int looked = 0;
    if (!looked) { looked = 1; used = (bool (*)(AAudioStream *))dlsym(RTLD_DEFAULT, "AAudioStream_isMMapUsed"); }
    return used && used(stream(h)) ? JNI_TRUE : JNI_FALSE;
}

JNIEXPORT void JNICALL Java_com_pcremote_prs_AAudioNative_close(JNIEnv *env, jobject self, jlong h) {
    AAudioStream_requestStop(stream(h));
    AAudioStream_close(stream(h));
}
