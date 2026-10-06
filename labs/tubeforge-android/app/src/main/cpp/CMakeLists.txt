cmake_minimum_required(VERSION 3.22)
project(tubeforge-android)

# The Operon core, cross-compiled by cargo-ndk in CI, imported prebuilt.
add_library(operon SHARED IMPORTED)
set_target_properties(operon PROPERTIES
    IMPORTED_LOCATION ${CMAKE_CURRENT_SOURCE_DIR}/../jniLibs/${ANDROID_ABI}/liboperon.so)

add_library(operon-glue SHARED jni_glue.c)
target_link_libraries(operon-glue operon log)
