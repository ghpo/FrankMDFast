package com.ghpo.frankmd

object NativeVault {
    init {
        System.loadLibrary("frankmd_mobile")
    }

    external fun nativeOpen(root: String, stateDir: String): Long
    external fun nativeClose(handle: Long)
    external fun nativeList(handle: Long): String
    external fun nativeRead(handle: Long, path: String): String
    external fun nativeWrite(handle: Long, path: String, text: String): String
    external fun nativeDelete(handle: Long, path: String)
    external fun nativeRename(handle: Long, oldPath: String, newPath: String): String
    external fun nativeUnlock(handle: Long, password: String): Boolean
    external fun nativeLock(handle: Long)
    external fun nativeStatus(handle: Long): String
    external fun nativeImportSeal(handle: Long, json: String): Boolean
    external fun nativeSetEncrypted(handle: Long, path: String, encrypted: Boolean): String
    external fun nativeSearch(handle: Long, query: String, limit: Int): String
    external fun nativePreview(handle: Long, path: String): String
}
