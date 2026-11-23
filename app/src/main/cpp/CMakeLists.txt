cmake_minimum_required(VERSION 3.22.1)
project(secparaudio C)

# Stream audio through AAudio, which reaches the low-latency (MMAP) output that AudioTrack cannot.
add_library(secparaudio SHARED secpar_audio.c)
# The app supports Android 8.0; calls added in Android 9 are guarded with __builtin_available.
target_compile_definitions(secparaudio PRIVATE __ANDROID_UNAVAILABLE_SYMBOLS_ARE_WEAK__)
target_compile_options(secparaudio PRIVATE -Werror=unguarded-availability)
target_link_libraries(secparaudio aaudio log)
