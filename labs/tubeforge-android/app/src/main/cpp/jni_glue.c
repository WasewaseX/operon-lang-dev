/* jni_glue.c — the only C in TubeForge for Android.
 * Bridges Kotlin's JNI to the plain C ABI of liboperon.so (the Operon
 * language core). Strings cross as modified-UTF8; Operon sources and
 * results are ASCII-safe JSON/text by construction (policy programs
 * print key = value lines). */
#include <jni.h>
#include <string.h>
#include <stdlib.h>

const char *operon_version(void);
char *operon_run_file(const char *path);
void operon_string_free(char *ptr);
int operon_ffi_canary(void);

static jstring to_jstring(JNIEnv *env, const char *s) {
    if (!s) return (*env)->NewStringUTF(env, "{}");
    jstring j = (*env)->NewStringUTF(env, s);
    return j;
}

JNIEXPORT jstring JNICALL
Java_com_tubeforge_lite_OperonBridge_coreVersion(JNIEnv *env, jclass clazz) {
    (void)clazz;
    return to_jstring(env, operon_version());
}

JNIEXPORT jint JNICALL
Java_com_tubeforge_lite_OperonBridge_canary(JNIEnv *env, jclass clazz) {
    (void)env; (void)clazz;
    return operon_ffi_canary();
}

/* Runs one .op program under the default-deny sandbox. Path = the app's
 * private copy of a policy asset. Returns the JSON result document. */
JNIEXPORT jstring JNICALL
Java_com_tubeforge_lite_OperonBridge_runPolicyFile(JNIEnv *env, jclass clazz,
                                                   jstring jpath) {
    (void)clazz;
    if (!jpath) return to_jstring(env, "{\"ok\":false,\"stress\":{\"kind\":\"arg\",\"message\":\"null path\"}}");
    const char *path = (*env)->GetStringUTFChars(env, jpath, NULL);
    if (!path) return to_jstring(env, "{\"ok\":false,\"stress\":{\"kind\":\"arg\",\"message\":\"utf8\"}}");
    char *result = operon_run_file(path);
    jstring out = to_jstring(env, result ? result
                                         : "{\"ok\":false,\"stress\":{\"kind\":\"ffi\",\"message\":\"null result\"}}");
    if (result) operon_string_free(result);
    (*env)->ReleaseStringUTFChars(env, jpath, path);
    return out;
}
