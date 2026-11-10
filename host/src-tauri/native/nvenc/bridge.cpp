// SPDX-License-Identifier: GPL-3.0-only
// Direct D3D11 NVENC, following Sunshine's native-resource and low-latency setup.
// The declarations in nvEncodeAPI.h carry their own MIT license.
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <Windows.h>
#include <d3d11.h>
#include <algorithm>
#include <cstring>
#include <chrono>
#include <new>
#include <memory>
#include <vector>
#include "nvEncodeAPI.h"

struct Encoder {
    HMODULE library = nullptr;
    NV_ENCODE_API_FUNCTION_LIST api{};
    void* session = nullptr;
    ID3D11Device* device = nullptr;
    struct Input { ID3D11Texture2D* texture; NV_ENC_REGISTERED_PTR registered; };
    // Conversion cycles through a three-texture ring. Register each resource once.
    // Eight slots also bound resources retained across converter reinitialization.
    std::vector<Input> inputs;
    NV_ENC_INPUT_PTR mapped = nullptr;
    NV_ENC_OUTPUT_PTR output = nullptr;
    HANDLE event = nullptr;
    bool event_registered = false;
    NV_ENC_CONFIG config{};
    NV_ENC_INITIALIZE_PARAMS init{};
    std::vector<unsigned char> bytes;
    bool key = false;

    ~Encoder() {
        if (session) {
            if (mapped) api.nvEncUnmapInputResource(session, mapped);
            for (const auto& input : inputs) api.nvEncUnregisterResource(session, input.registered);
            if (output) api.nvEncDestroyBitstreamBuffer(session, output);
            if (event_registered) {
                NV_ENC_EVENT_PARAMS p{}; p.version = NV_ENC_EVENT_PARAMS_VER; p.completionEvent = event;
                api.nvEncUnregisterAsyncEvent(session, &p);
            }
            api.nvEncDestroyEncoder(session);
        }
        for (const auto& input : inputs) input.texture->Release();
        if (device) device->Release();
        if (event) CloseHandle(event);
        if (library) FreeLibrary(library);
    }

    NVENCSTATUS open(ID3D11Device* dev, unsigned codec, unsigned w, unsigned h, unsigned fps, unsigned kbps) {
        if (codec != 1 && codec != 2) return NV_ENC_ERR_UNSUPPORTED_PARAM;
        library = LoadLibraryExW(L"nvEncodeAPI64.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
        if (!library) return NV_ENC_ERR_NO_ENCODE_DEVICE;
        auto create = reinterpret_cast<NVENCSTATUS(NVENCAPI*)(NV_ENCODE_API_FUNCTION_LIST*)>(GetProcAddress(library, "NvEncodeAPICreateInstance"));
        auto max_version = reinterpret_cast<NVENCSTATUS(NVENCAPI*)(uint32_t*)>(GetProcAddress(library, "NvEncodeAPIGetMaxSupportedVersion"));
        if (!create || !max_version) return NV_ENC_ERR_INVALID_VERSION;
        uint32_t supported = 0;
        auto status = max_version(&supported);
        if (status != NV_ENC_SUCCESS) return status;
        if (supported < ((NVENCAPI_MAJOR_VERSION << 4) | NVENCAPI_MINOR_VERSION)) return NV_ENC_ERR_INVALID_VERSION;
        api.version = NV_ENCODE_API_FUNCTION_LIST_VER;
        status = create(&api); if (status != NV_ENC_SUCCESS) return status;
        NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS open{};
        open.version = NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;
        open.device = dev; open.deviceType = NV_ENC_DEVICE_TYPE_DIRECTX; open.apiVersion = NVENCAPI_VERSION;
        status = api.nvEncOpenEncodeSessionEx(&open, &session); if (status != NV_ENC_SUCCESS) return status;
        device = dev; device->AddRef();
        const GUID guid = codec == 1 ? NV_ENC_CODEC_H264_GUID : NV_ENC_CODEC_HEVC_GUID;
        NV_ENC_PRESET_CONFIG preset{}; preset.version = NV_ENC_PRESET_CONFIG_VER; preset.presetCfg.version = NV_ENC_CONFIG_VER;
        status = api.nvEncGetEncodePresetConfigEx(session, guid, NV_ENC_PRESET_P1_GUID, NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY, &preset);
        if (status != NV_ENC_SUCCESS) return status;
        config = preset.presetCfg;
        config.gopLength = NVENC_INFINITE_GOPLENGTH; config.frameIntervalP = 1;
        config.rcParams.rateControlMode = NV_ENC_PARAMS_RC_CBR;
        config.rcParams.averageBitRate = kbps * 1000; config.rcParams.maxBitRate = kbps * 1000;
        config.rcParams.vbvBufferSize = std::max(kbps * 1000 / fps, 20000u);
        config.rcParams.vbvInitialDelay = config.rcParams.vbvBufferSize;
        config.rcParams.enableLookahead = 0; config.rcParams.lookaheadDepth = 0;
        config.rcParams.zeroReorderDelay = 1; config.rcParams.enableAQ = 0;
        config.rcParams.multiPass = NV_ENC_MULTI_PASS_DISABLED;
        if (codec == 1) {
            config.profileGUID = NV_ENC_H264_PROFILE_HIGH_GUID;
            auto& c = config.encodeCodecConfig.h264Config;
            c.idrPeriod = NVENC_INFINITE_GOPLENGTH; c.repeatSPSPPS = 1;
            auto& v = c.h264VUIParameters;
            v.videoSignalTypePresentFlag = 1; v.videoFormat = NV_ENC_VUI_VIDEO_FORMAT_UNSPECIFIED; v.videoFullRangeFlag = 0;
            v.colourDescriptionPresentFlag = 1; v.colourPrimaries = NV_ENC_VUI_COLOR_PRIMARIES_BT709; v.transferCharacteristics = NV_ENC_VUI_TRANSFER_CHARACTERISTIC_BT709; v.colourMatrix = NV_ENC_VUI_MATRIX_COEFFS_BT709;
        } else {
            config.profileGUID = NV_ENC_HEVC_PROFILE_MAIN_GUID;
            auto& c = config.encodeCodecConfig.hevcConfig;
            c.idrPeriod = NVENC_INFINITE_GOPLENGTH; c.repeatSPSPPS = 1;
            auto& v = c.hevcVUIParameters;
            v.videoSignalTypePresentFlag = 1; v.videoFormat = NV_ENC_VUI_VIDEO_FORMAT_UNSPECIFIED; v.videoFullRangeFlag = 0;
            v.colourDescriptionPresentFlag = 1; v.colourPrimaries = NV_ENC_VUI_COLOR_PRIMARIES_BT709; v.transferCharacteristics = NV_ENC_VUI_TRANSFER_CHARACTERISTIC_BT709; v.colourMatrix = NV_ENC_VUI_MATRIX_COEFFS_BT709;
        }
        // Windows asynchronous completion: never wait for GPU output while holding
        // the D3D immediate-context lock used by desktop capture/conversion.
        event = CreateEventW(nullptr, FALSE, FALSE, nullptr);
        if (!event) return NV_ENC_ERR_OUT_OF_MEMORY;
        init.version = NV_ENC_INITIALIZE_PARAMS_VER; init.encodeGUID = guid;
        init.presetGUID = NV_ENC_PRESET_P1_GUID; init.tuningInfo = NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY;
        init.encodeWidth = w; init.encodeHeight = h; init.darWidth = w; init.darHeight = h;
        init.maxEncodeWidth = w; init.maxEncodeHeight = h;
        init.frameRateNum = fps; init.frameRateDen = 1;
        init.enablePTD = 1; init.enableEncodeAsync = 1; init.encodeConfig = &config;
        status = api.nvEncInitializeEncoder(session, &init); if (status != NV_ENC_SUCCESS) return status;
        NV_ENC_EVENT_PARAMS ep{}; ep.version = NV_ENC_EVENT_PARAMS_VER; ep.completionEvent = event;
        status = api.nvEncRegisterAsyncEvent(session, &ep); if (status != NV_ENC_SUCCESS) return status;
        event_registered = true;
        NV_ENC_CREATE_BITSTREAM_BUFFER bp{}; bp.version = NV_ENC_CREATE_BITSTREAM_BUFFER_VER;
        status = api.nvEncCreateBitstreamBuffer(session, &bp);
        output = bp.bitstreamBuffer;
        return status;
    }

    NVENCSTATUS encode(ID3D11Texture2D* tex, uint64_t timestamp, bool idr, uint64_t* timings) {
        auto timed = [&](unsigned slot, auto call) -> NVENCSTATUS {
            if (!timings) return call();
            const auto before = std::chrono::steady_clock::now();
            const auto status = call();
            timings[slot] = std::chrono::duration_cast<std::chrono::microseconds>(std::chrono::steady_clock::now() - before).count();
            return status;
        };
        auto it = std::find_if(inputs.begin(), inputs.end(), [tex](const Input& input) { return input.texture == tex; });
        NV_ENC_REGISTERED_PTR registered = nullptr;
        if (it != inputs.end()) {
            registered = it->registered;
        } else {
            NV_ENC_REGISTER_RESOURCE p{}; p.version = NV_ENC_REGISTER_RESOURCE_VER;
            p.resourceType = NV_ENC_INPUT_RESOURCE_TYPE_DIRECTX;
            p.width = init.encodeWidth; p.height = init.encodeHeight;
            p.resourceToRegister = tex; p.bufferFormat = NV_ENC_BUFFER_FORMAT_NV12; p.bufferUsage = NV_ENC_INPUT_IMAGE;
            auto status = api.nvEncRegisterResource(session, &p); if (status != NV_ENC_SUCCESS) return status;
            registered = p.registeredResource;
            try { inputs.push_back({tex, registered}); }
            catch (...) { api.nvEncUnregisterResource(session, registered); return NV_ENC_ERR_OUT_OF_MEMORY; }
            tex->AddRef();
            if (inputs.size() > 8) {
                // Only one frame is in flight, and its previous mapping is already
                // complete before this call. Evict the oldest unused resource.
                api.nvEncUnregisterResource(session, inputs.front().registered);
                inputs.front().texture->Release();
                inputs.erase(inputs.begin());
            }
        }
        NV_ENC_MAP_INPUT_RESOURCE mp{}; mp.version = NV_ENC_MAP_INPUT_RESOURCE_VER; mp.registeredResource = registered;
        auto status = timed(0, [&] { return api.nvEncMapInputResource(session, &mp); }); if (status != NV_ENC_SUCCESS) return status;
        mapped = mp.mappedResource;
        NV_ENC_PIC_PARAMS p{}; p.version = NV_ENC_PIC_PARAMS_VER;
        p.inputWidth = init.encodeWidth; p.inputHeight = init.encodeHeight;
        p.inputBuffer = mapped; p.bufferFmt = NV_ENC_BUFFER_FORMAT_NV12;
        p.outputBitstream = output; p.completionEvent = event; p.pictureStruct = NV_ENC_PIC_STRUCT_FRAME;
        p.inputTimeStamp = timestamp; p.inputDuration = 1000000 / init.frameRateNum;
        if (idr) p.encodePicFlags = NV_ENC_PIC_FLAG_FORCEIDR | NV_ENC_PIC_FLAG_OUTPUT_SPSPPS;
        ResetEvent(event);
        status = timed(1, [&] { return api.nvEncEncodePicture(session, &p); });
        if (status == NV_ENC_SUCCESS) status = timed(2, [&] { return WaitForSingleObject(event, 1000) == WAIT_OBJECT_0 ? NV_ENC_SUCCESS : NV_ENC_ERR_GENERIC; });
        if (status == NV_ENC_SUCCESS) {
            NV_ENC_LOCK_BITSTREAM lock{}; lock.version = NV_ENC_LOCK_BITSTREAM_VER; lock.outputBitstream = output;
            status = timed(3, [&] { return api.nvEncLockBitstream(session, &lock); });
            if (status == NV_ENC_SUCCESS) {
                const auto data = static_cast<const unsigned char*>(lock.bitstreamBufferPtr);
                try { timed(4, [&] { bytes.assign(data, data + lock.bitstreamSizeInBytes); return NV_ENC_SUCCESS; }); }
                catch (...) {
                    api.nvEncUnlockBitstream(session, output);
                    api.nvEncUnmapInputResource(session, mapped); mapped = nullptr;
                    return NV_ENC_ERR_OUT_OF_MEMORY;
                }
                key = lock.pictureType == NV_ENC_PIC_TYPE_IDR || lock.pictureType == NV_ENC_PIC_TYPE_I;
                api.nvEncUnlockBitstream(session, output);
            }
        }
        timed(5, [&] { return api.nvEncUnmapInputResource(session, mapped); }); mapped = nullptr;
        return status;
    }

    NVENCSTATUS bitrate(unsigned kbps) {
        auto previous = config.rcParams;
        config.rcParams.averageBitRate = config.rcParams.maxBitRate = kbps * 1000;
        config.rcParams.vbvBufferSize = std::max(kbps * 1000 / init.frameRateNum, 20000u);
        config.rcParams.vbvInitialDelay = config.rcParams.vbvBufferSize;
        NV_ENC_RECONFIGURE_PARAMS p{}; p.version = NV_ENC_RECONFIGURE_PARAMS_VER; p.reInitEncodeParams = init;
        auto status = api.nvEncReconfigureEncoder(session, &p);
        if (status != NV_ENC_SUCCESS) config.rcParams = previous;
        return status;
    }
};

extern "C" {
void* secpar_nvenc_open(ID3D11Device* dev, unsigned codec, unsigned w, unsigned h, unsigned fps, unsigned kbps, int* error) noexcept {
    try {
        auto p = std::make_unique<Encoder>();
        const auto status = p->open(dev, codec, w, h, fps, kbps);
        *error = status;
        if (status != NV_ENC_SUCCESS) return nullptr;
        return p.release();
    } catch (...) { *error = NV_ENC_ERR_OUT_OF_MEMORY; return nullptr; }
}
int secpar_nvenc_encode(void* ptr, ID3D11Texture2D* tex, uint64_t time, bool idr, const unsigned char** data, unsigned* size, bool* key, uint64_t* timings) noexcept {
    try {
        auto p = static_cast<Encoder*>(ptr);
        auto status = p->encode(tex, time, idr, timings);
        if (status == NV_ENC_SUCCESS) { *data = p->bytes.data(); *size = static_cast<unsigned>(p->bytes.size()); *key = p->key; }
        return status;
    } catch (...) { return NV_ENC_ERR_OUT_OF_MEMORY; }
}
int secpar_nvenc_bitrate(void* ptr, unsigned kbps) noexcept { return static_cast<Encoder*>(ptr)->bitrate(kbps); }
void secpar_nvenc_close(void* ptr) noexcept { delete static_cast<Encoder*>(ptr); }
}
