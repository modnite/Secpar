// Fills in moonlight-common-c's structures for Secpar and hands Rust plain
// callbacks: one contiguous buffer per video frame, one Opus packet per audio
// sample, and the connection's start and end.
#include "Limelight.h"
#include <stdlib.h>
#include <string.h>

typedef void (*secpar_video_fn)(void *user, const unsigned char *data, int length, int keyframe, int format);
typedef void (*secpar_audio_fn)(void *user, const char *data, int length);
typedef void (*secpar_event_fn)(void *user, int kind, int code);

static secpar_video_fn video_fn;
static secpar_audio_fn audio_fn;
static secpar_event_fn event_fn;
static void *user_data;
static unsigned char *frame;
static int frame_capacity;
static int video_format;

enum { SECPAR_STARTED = 1, SECPAR_TERMINATED = 2, SECPAR_STAGE_FAILED = 3, SECPAR_POOR = 4, SECPAR_OKAY = 5 };

static int dr_setup(int format, int width, int height, int fps, void *context, int flags) {
    (void)width; (void)height; (void)fps; (void)context; (void)flags;
    video_format = format;
    return 0;
}

static int dr_submit(PDECODE_UNIT unit) {
    if (unit->fullLength > frame_capacity) {
        unsigned char *bigger = realloc(frame, (size_t)unit->fullLength * 2);
        if (!bigger) {
            return DR_NEED_IDR;
        }
        frame = bigger;
        frame_capacity = unit->fullLength * 2;
    }
    int at = 0;
    for (PLENTRY entry = unit->bufferList; entry; entry = entry->next) {
        memcpy(frame + at, entry->data, (size_t)entry->length);
        at += entry->length;
    }
    video_fn(user_data, frame, at, unit->frameType == FRAME_TYPE_IDR, video_format);
    return DR_OK;
}

static int ar_init(int config, const POPUS_MULTISTREAM_CONFIGURATION opus, void *context, int flags) {
    (void)config; (void)opus; (void)context; (void)flags;
    return 0;
}

static void ar_sample(char *data, int length) {
    audio_fn(user_data, data, length);
}

static void cl_started(void) { event_fn(user_data, SECPAR_STARTED, 0); }
static void cl_terminated(int code) { event_fn(user_data, SECPAR_TERMINATED, code); }
static void cl_stage_failed(int stage, int code) { (void)stage; event_fn(user_data, SECPAR_STAGE_FAILED, code); }
static void cl_status(int status) { event_fn(user_data, status == CONN_STATUS_POOR ? SECPAR_POOR : SECPAR_OKAY, 0); }

int secpar_start(const char *address, const char *app_version, const char *gfe_version, const char *rtsp_url,
                 int codec_support, int width, int height, int fps, int kbps, int remote,
                 const unsigned char *input_key, int input_key_id, int formats,
                 secpar_video_fn video, secpar_audio_fn audio, secpar_event_fn event, void *user) {
    video_fn = video;
    audio_fn = audio;
    event_fn = event;
    user_data = user;

    SERVER_INFORMATION server;
    LiInitializeServerInformation(&server);
    server.address = address;
    server.serverInfoAppVersion = app_version;
    server.serverInfoGfeVersion = gfe_version;
    server.rtspSessionUrl = rtsp_url;
    server.serverCodecModeSupport = codec_support;

    STREAM_CONFIGURATION config;
    LiInitializeStreamConfiguration(&config);
    config.width = width;
    config.height = height;
    config.fps = fps;
    config.bitrate = kbps;
    config.packetSize = remote ? 1024 : 1392;
    config.streamingRemotely = remote ? STREAM_CFG_REMOTE : STREAM_CFG_AUTO;
    config.audioConfiguration = AUDIO_CONFIGURATION_STEREO;
    config.supportedVideoFormats = formats;
    config.clientRefreshRateX100 = fps * 100;
    config.colorSpace = COLORSPACE_REC_709;
    config.colorRange = COLOR_RANGE_LIMITED;
    config.encryptionFlags = ENCFLG_AUDIO;
    memcpy(config.remoteInputAesKey, input_key, 16);
    memset(config.remoteInputAesIv, 0, 16);
    config.remoteInputAesIv[0] = (char)((input_key_id >> 24) & 0xff);
    config.remoteInputAesIv[1] = (char)((input_key_id >> 16) & 0xff);
    config.remoteInputAesIv[2] = (char)((input_key_id >> 8) & 0xff);
    config.remoteInputAesIv[3] = (char)(input_key_id & 0xff);

    CONNECTION_LISTENER_CALLBACKS listener;
    LiInitializeConnectionCallbacks(&listener);
    listener.connectionStarted = cl_started;
    listener.connectionTerminated = cl_terminated;
    listener.stageFailed = cl_stage_failed;
    listener.connectionStatusUpdate = cl_status;

    DECODER_RENDERER_CALLBACKS decoder;
    LiInitializeVideoCallbacks(&decoder);
    decoder.setup = dr_setup;
    decoder.submitDecodeUnit = dr_submit;
    decoder.capabilities = CAPABILITY_DIRECT_SUBMIT;

    AUDIO_RENDERER_CALLBACKS sound;
    LiInitializeAudioCallbacks(&sound);
    sound.init = ar_init;
    sound.decodeAndPlaySample = ar_sample;
    sound.capabilities = CAPABILITY_DIRECT_SUBMIT;

    return LiStartConnection(&server, &config, &listener, &decoder, &sound, NULL, 0, NULL, 0);
}
