// SPDX-License-Identifier: GPL-3.0-only
// FFmpeg integration using Sunshine's D3D11 hardware-frame and low-delay encoder
// configuration. Reference: LizardByte/Sunshine 63d35f702ee9e362e43263742981836ec0710384,
// src/video.cpp (GPL-3.0), Copyright (c) Sunshine contributors.
#include <d3d11.h>
#include <wrl/client.h>
#include <climits>
#include <cerrno>
#include <cstdint>
#include <cstring>
#include <memory>
#include <string>
extern "C" {
#include <libavcodec/avcodec.h>
#include <libavutil/dict.h>
#include <libavutil/error.h>
#include <libavutil/hwcontext.h>
#include <libavutil/hwcontext_d3d11va.h>
}
using Microsoft::WRL::ComPtr;
namespace {
struct Encoder {
    AVCodecContext* codec = nullptr;
    AVBufferRef* device = nullptr;
    AVPacket* packet = nullptr;
    ComPtr<ID3D11DeviceContext> context;
    bool qsv = false;
    bool relaxed_rate_control = false;
    unsigned fps = 0;
    ~Encoder() { av_packet_free(&packet); avcodec_free_context(&codec); av_buffer_unref(&device); }
    int open(ID3D11Device* dev, const char* backend, unsigned format, unsigned w,
             unsigned h, unsigned rate, unsigned kbps, unsigned quality, bool fallback = false) {
        if (!dev || !backend || !w || !h || !rate || !kbps || format > 2 ||
            w > INT_MAX || h > INT_MAX || kbps > INT_MAX / 1000) return AVERROR(EINVAL);
        const char* names[] = {"h264", "hevc", "av1"};
        std::string name = std::string(names[format]) + "_" + backend;
        const AVCodec* implementation = avcodec_find_encoder_by_name(name.c_str());
        if (!implementation) return AVERROR_ENCODER_NOT_FOUND;
        codec = avcodec_alloc_context3(implementation);
        packet = av_packet_alloc();
        if (!codec || !packet) return AVERROR(ENOMEM);
        AVBufferRef* base = av_hwdevice_ctx_alloc(AV_HWDEVICE_TYPE_D3D11VA);
        if (!base) return AVERROR(ENOMEM);
        auto* d3d = static_cast<AVD3D11VADeviceContext*>(reinterpret_cast<AVHWDeviceContext*>(base->data)->hwctx);
        d3d->device = dev;
        dev->AddRef();
        int status = av_hwdevice_ctx_init(base);
        if (status < 0) { av_buffer_unref(&base); return status; }
        qsv = std::strcmp(backend, "qsv") == 0;
        relaxed_rate_control = qsv || std::strcmp(backend, "amf") == 0;
        if (qsv) {
            status = av_hwdevice_ctx_create_derived(&device, AV_HWDEVICE_TYPE_QSV, base, 0);
            av_buffer_unref(&base);
            if (status < 0) return status;
        } else { device = base; }
        dev->GetImmediateContext(context.GetAddressOf());
        codec->width = w; codec->height = h; fps = rate;
        codec->time_base = AVRational{1, 1000000};
        codec->framerate = AVRational{static_cast<int>(rate), 1};
        codec->sample_aspect_ratio = AVRational{1, 1};
        codec->pix_fmt = qsv ? AV_PIX_FMT_QSV : AV_PIX_FMT_D3D11;
        codec->sw_pix_fmt = AV_PIX_FMT_NV12;
        codec->rc_max_rate = static_cast<int64_t>(kbps) * 1000;
        // Sunshine's AMF/QSV flags: CBR_WITH_VBR, NO_RC_BUF_LIMIT and RELAXED_COMPLIANCE.
        codec->bit_rate = codec->rc_max_rate - (relaxed_rate_control ? 1 : 0);
        codec->rc_buffer_size = relaxed_rate_control ? 0 : static_cast<int>(codec->bit_rate / rate);
        if (relaxed_rate_control) codec->strict_std_compliance = FF_COMPLIANCE_UNOFFICIAL;
        codec->max_b_frames = 0;
        codec->gop_size = codec->keyint_min = INT_MAX;
        codec->refs = 1;
        codec->flags = AV_CODEC_FLAG_CLOSED_GOP | AV_CODEC_FLAG_LOW_DELAY;
        codec->flags2 = AV_CODEC_FLAG2_FAST;
        codec->color_range = AVCOL_RANGE_MPEG;
        codec->color_primaries = AVCOL_PRI_BT709;
        codec->color_trc = AVCOL_TRC_BT709;
        codec->colorspace = AVCOL_SPC_BT709;
        codec->hw_frames_ctx = av_hwframe_ctx_alloc(device);
        if (!codec->hw_frames_ctx) return AVERROR(ENOMEM);
        auto* frames = reinterpret_cast<AVHWFramesContext*>(codec->hw_frames_ctx->data);
        frames->format = codec->pix_fmt; frames->sw_format = AV_PIX_FMT_NV12;
        frames->width = w; frames->height = h;
        // Sunshine display_vram::init_hwframes uses a single render-target texture.
        // NV12 array textures without decoder binding are rejected by D3D11 drivers.
        frames->initial_pool_size = 1;
        if (!qsv) {
            auto* textures=static_cast<AVD3D11VAFramesContext*>(frames->hwctx);
            textures->BindFlags=D3D11_BIND_RENDER_TARGET;
            textures->MiscFlags=0;
        }
        status = av_hwframe_ctx_init(codec->hw_frames_ctx);
        if (status < 0) return status;
        AVDictionary* options = nullptr;
        auto set = [&](const char* key, const char* value) { av_dict_set(&options,key,value,0); };
        if (std::strcmp(backend,"nvenc") == 0) {
            set("preset", quality == 2 ? "p7" : quality == 1 ? "p4" : "p1");
            set("tune","ull"); set("rc","cbr"); set("rc-lookahead","0");
            set("zerolatency","1"); set("delay","0"); set("forced-idr","1");
        } else if (qsv) {
            set("preset", quality == 2 ? "slow" : quality == 1 ? "medium" : "veryfast");
            set("forced_idr","1"); set("async_depth","1"); set("low_delay_brc","1");
            set("low_power",fallback ? "0" : "1");
            if (format == 0) {
                set("vcm","1"); set("recovery_point_sei","0");
                set("pic_timing_sei","0"); set("max_dec_frame_buffering","1");
            }
            // Sunshine's SDR high/main profiles, never B frames or lookahead.
            set("profile", format == 0 ? "high" : "main");
        } else if (std::strcmp(backend,"amf") == 0) {
            set("quality",quality == 2 ? "quality" : quality == 1 ? "balanced" : "speed");
            set("usage",fallback ? "lowlatency" : "ultralowlatency"); set("rc","cbr");
            set("forced_idr","1"); set("async_depth","1");
            set("latency",format == 2 ? "lowest_latency" : "1");
            set("filler_data","0"); set("preencode","0");
            if (format == 1) { set("gops_per_idr","1"); set("header_insertion_mode","idr"); }
        }
        status = avcodec_open2(codec, implementation, &options);
        // Treat misspelled/unimplemented options as errors instead of silently changing latency.
        if (status >= 0 && av_dict_count(options)) status = AVERROR_OPTION_NOT_FOUND;
        av_dict_free(&options);
        return status;
    }
    int send(ID3D11Texture2D* texture, uint64_t stamp, bool key) {
        if (!texture || stamp > INT64_MAX) return AVERROR(EINVAL);
        AVFrame* frame = av_frame_alloc();
        AVFrame* mapped = nullptr;
        if (!frame) return AVERROR(ENOMEM);
        int status = av_hwframe_get_buffer(codec->hw_frames_ctx,frame,0);
        AVFrame* gpu = frame;
        if (status >= 0 && qsv) {
            mapped = av_frame_alloc();
            if (!mapped) status = AVERROR(ENOMEM);
            else {
                mapped->format = AV_PIX_FMT_D3D11;
                status = av_hwframe_map(mapped,frame,AV_HWFRAME_MAP_WRITE | AV_HWFRAME_MAP_OVERWRITE);
                gpu = mapped;
            }
        }
        if (status >= 0) {
            auto* destination = reinterpret_cast<ID3D11Texture2D*>(gpu->data[0]);
            if (!destination) status = AVERROR(EINVAL);
            else {
                context->CopySubresourceRegion(destination,static_cast<UINT>(reinterpret_cast<uintptr_t>(gpu->data[1])),0,0,0,texture,0,nullptr);
                frame->pts = static_cast<int64_t>(stamp);
                frame->pict_type = key ? AV_PICTURE_TYPE_I : AV_PICTURE_TYPE_NONE;
                frame->color_range=codec->color_range; frame->colorspace=codec->colorspace;
                frame->color_primaries=codec->color_primaries; frame->color_trc=codec->color_trc;
                status = avcodec_send_frame(codec,frame);
            }
        }
        av_frame_free(&mapped); av_frame_free(&frame);
        return status;
    }
};
}
extern "C" {
void* secpar_avcodec_open(ID3D11Device* dev, const char* backend, unsigned codec, unsigned w, unsigned h,
                          unsigned fps, unsigned kbps, unsigned quality, int* status) noexcept {
    try {
        auto encoder = std::make_unique<Encoder>();
        *status = encoder->open(dev,backend,codec,w,h,fps,kbps,quality);
        // Sunshine retries old/low-end Intel GPUs without low-power encoding,
        // and AMF H.264 with low-latency usage when ultra-low-latency is rejected.
        if (*status < 0 && backend &&
            ((std::strcmp(backend,"qsv")==0 && codec<2) ||
             (std::strcmp(backend,"amf")==0 && codec==0))) {
            encoder=std::make_unique<Encoder>();
            *status=encoder->open(dev,backend,codec,w,h,fps,kbps,quality,true);
        }
        return *status < 0 ? nullptr : encoder.release();
    } catch (...) { *status=AVERROR(ENOMEM); return nullptr; }
}
int secpar_avcodec_send(void* ptr, ID3D11Texture2D* tex, uint64_t stamp, bool key) noexcept {
    return static_cast<Encoder*>(ptr)->send(tex,stamp,key);
}
int secpar_avcodec_receive(void* ptr, const unsigned char** data, unsigned* size, uint64_t* stamp, bool* key) noexcept {
    auto* enc=static_cast<Encoder*>(ptr);
    av_packet_unref(enc->packet);
    int status=avcodec_receive_packet(enc->codec,enc->packet);
    if(status==AVERROR(EAGAIN)) return 0;
    if(status<0) return status;
    if(enc->packet->pts<0 || enc->packet->size<=0) return AVERROR_INVALIDDATA;
    *data=enc->packet->data; *size=enc->packet->size;
    *stamp=static_cast<uint64_t>(enc->packet->pts); *key=(enc->packet->flags&AV_PKT_FLAG_KEY)!=0;
    return 1;
}
int secpar_avcodec_bitrate(void* ptr, unsigned kbps) noexcept {
    if(!kbps || kbps>INT_MAX/1000) return AVERROR(EINVAL);
    auto* enc=static_cast<Encoder*>(ptr);
    enc->codec->rc_max_rate=static_cast<int64_t>(kbps)*1000;
    enc->codec->bit_rate=enc->codec->rc_max_rate-(enc->relaxed_rate_control ? 1 : 0);
    enc->codec->rc_buffer_size=enc->relaxed_rate_control ? 0 : static_cast<int>(enc->codec->bit_rate/enc->fps);
    return 0;
}
void secpar_avcodec_error(int status, char* text, unsigned size) noexcept { av_strerror(status,text,size); }
void secpar_avcodec_close(void* ptr) noexcept { delete static_cast<Encoder*>(ptr); }
}
