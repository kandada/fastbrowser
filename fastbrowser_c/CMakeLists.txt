# Copyright (c) 2025 xiefujin <490021684@qq.com>
# Licensed under Apache-2.0.
#
# fastbrowser_c — Android integration (方案 B: staticlib + NDK CMake).
# Produces libfastbrowser_jni.so from jni_glue.c + libfastbrowser.a.

cmake_minimum_required(VERSION 3.18)

project(fastbrowser_c LANGUAGES C)

set(FASTBROWSER_STATIC_LIB
    "${CMAKE_CURRENT_SOURCE_DIR}/../../c_dist/${ANDROID_ABI}/libfastbrowser.a"
)

add_library(fastbrowser_jni SHARED
    src/jni_glue.c
)

target_include_directories(fastbrowser_jni PRIVATE
    include
)

target_link_libraries(fastbrowser_jni
    "${FASTBROWSER_STATIC_LIB}"
    log
    dl
    m
    z
)

set_target_properties(fastbrowser_jni PROPERTIES
    C_STANDARD 11
    OUTPUT_NAME "fastbrowser_jni"
)

# 避免导出非必要符号
if(CMAKE_C_COMPILER_ID MATCHES "GNU|Clang")
    target_link_options(fastbrowser_jni PRIVATE
        -Wl,--gc-sections -Wl,--exclude-libs,ALL -Wl,--no-undefined
        -Wl,-z,max-page-size=16384
    )
endif()
