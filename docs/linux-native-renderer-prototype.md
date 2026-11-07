# Native Linux rendering prototype

Branch feat/linux-native-renderer is isolated from published3.12.0. New modules
are included only in tests. The application still uses its previous renderer.

Implemented components:
- dmabuf.rs: VA surface synchronization/export without mapping pixels to CPU;
  retains GstSample and OwnedFd. Validates NV12 R8/GR88 plane layouts against
  importer-supported modifiers, visible dimensions and bounded allocation padding.
- dma_import.rs: current EGL context/thread checks,2D modifier intersection,
  separate-plane imports and retained frame ownership. GL fences allow nonblocking
  retirement; finish-and-release is reserved for context-owned shutdown.
- native_decode.rs: separate nonblocking compressed input and latest decoded
  output; NV12 VAMemory caps, bounded compressed input and capture-clock timestamps.
  Export runs on the decoder worker, never the UI. Legacy GstVaapi memory is rejected.
- yuv_shader.rs: GPU NV12 conversion using metadata range/matrix, chroma siting,
  visible/storage crop and edge clamps. No RGB image upload in that component.

Verified on the build runner, without a GPU or any user display/device mounts:
- Six descriptor/FD ownership/layout tests, including1080→1088 storage padding.
- Five importer tests for missing EGL context, modifier bounds/filtering and
  Send+Sync ownership across decoder/UI threads. GL3.1 without ARB_sync is
  rejected before image import; core GL3.2/GLES3 fences are accepted.
- Actual asynchronous decode fixture: eight dependent H.264 frames decode to NV12
  while output is not drained; only the latest sample remains, with correct capture
  PTS and negotiated duration. Backwards/overflow timestamps are rejected. This
  uses a CPU decoder and does not validate VA export or GPU throughput.
- Actual software OpenGL3.3 and GLES3 draws of known NV12 patches:16 combinations
  of BT.601/BT.709, full/limited range, padded/unpadded storage. Read pixels verify
  colors and vertical orientation. This is a shader correctness test, not GPU
  acceleration or throughput evidence.

Reproduce with desktop/packaging/test-native-video.sh TEST_BINARY APPIMAGE.
Actual VA export/EGL import on the X250 remains UNVERIFIED. Import success,
GPU fence lifetime and sustained real-stream throughput still require integration
and physical validation. Do not enable this from factory presence alone.

Next integration:
1. Connect the new VAMemory NV12 decoder/export stage to the existing bounded
   worker. Preserve dependency/keyframe recovery, startup grace and stall handling.
2. Initialize EGL importer/shader with the UI context current. Verify a decoded,
   known fixture end-to-end before advertising the native path.
3. Draw from imported plane textures in an egui callback; retain prior frames in
   a bounded fence retirement queue. Handle disconnect, context loss and shutdown.
4. Fall back to software on decode/export/import/presentation failure. Keep audio
   and authenticated transport independent of UI repainting, as3.12.0 does.
5. Test real X250 playback against Moonlight, including scaling, fullscreen,
   minimizing, silence recovery and returning after another app covers the window.

Primary specifications:
https://registry.khronos.org/EGL/extensions/EXT/EGL_EXT_image_dma_buf_import.txt
https://registry.khronos.org/EGL/extensions/EXT/EGL_EXT_image_dma_buf_import_modifiers.txt
https://registry.khronos.org/OpenGL/extensions/OES/OES_EGL_image.txt
https://github.com/moonlight-stream/moonlight-qt/blob/master/app/streaming/video/ffmpeg-renderers/vaapi.cpp
