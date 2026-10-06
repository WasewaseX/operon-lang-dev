package com.tubeforge.lite

/**
 * OperonBridge — the Kotlin surface of the embedded Operon core.
 * liboperon.so is the full Operon language core (tree-walk + VM) compiled
 * for this device's ABI; policy programs run under the default-deny
 * sandbox exactly like the desktop TubeForge policy lane.
 */
object OperonBridge {
    init {
        // liboperon.so rides as a DT_NEEDED dependency of the glue
        System.loadLibrary("operon-glue")
    }

    external fun coreVersion(): String
    external fun canary(): Int
    external fun runPolicyFile(path: String): String
}
