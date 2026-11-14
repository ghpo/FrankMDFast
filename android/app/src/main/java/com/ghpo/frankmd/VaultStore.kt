package com.ghpo.frankmd

import org.json.JSONArray
import java.io.File

data class NoteEntry(val path: String, val title: String, val encrypted: Boolean = false, val locked: Boolean = false)
data class SearchHit(val path: String, val title: String, val snippet: String, val locked: Boolean)
data class PreviewData(val html: String, val base: String)

class VaultStore(filesDir: File) : AutoCloseable {
    val root: File = File(filesDir, "vault")
    private val filesDir = filesDir
    private var handle: Long

    init {
        root.mkdirs()
        val state = File(filesDir, "state").apply { mkdirs() }
        handle = NativeVault.nativeOpen(root.absolutePath, state.absolutePath)
        check(handle != 0L) { "could not open the vault" }
    }

    fun notes(): List<NoteEntry> {
        val array = JSONArray(NativeVault.nativeList(handle))
        return buildList {
            for (i in 0 until array.length()) {
                val item = array.getJSONObject(i)
                add(
                    NoteEntry(
                        item.getString("path"),
                        item.getString("title"),
                        item.optBoolean("encrypted", false),
                        item.optBoolean("locked", false),
                    )
                )
            }
        }
    }

    fun search(query: String): List<SearchHit> {
        val array = JSONArray(NativeVault.nativeSearch(handle, query, 100))
        return buildList {
            for (i in 0 until array.length()) {
                val item = array.getJSONObject(i)
                add(
                    SearchHit(
                        item.getString("path"),
                        item.getString("title"),
                        item.optString("snippet"),
                        item.optBoolean("locked", false),
                    )
                )
            }
        }
    }

    fun preview(path: String): PreviewData {
        val item = org.json.JSONObject(NativeVault.nativePreview(handle, path))
        return PreviewData(item.getString("html"), item.getString("base"))
    }

    fun read(path: String): String = runCatching {
        NativeVault.nativeRead(handle, path)
    }.getOrElse { error ->
        "⚠️ ${error.message ?: "Não foi possível abrir esta nota"}"
    }

    fun save(path: String, content: String) {
        NativeVault.nativeWrite(handle, path, content)
    }

    fun cryptoStatus(): Pair<Boolean, Boolean> {
        val value = org.json.JSONObject(NativeVault.nativeStatus(handle))
        return value.getBoolean("seal") to value.getBoolean("unlocked")
    }

    fun unlock(password: String): Boolean =
        NativeVault.nativeUnlock(handle, password)

    fun lock() = NativeVault.nativeLock(handle)

    fun rename(oldPath: String, newPath: String) {
        NativeVault.nativeRename(handle, oldPath, newPath)
    }

    fun delete(path: String) {
        NativeVault.nativeDelete(handle, path)
    }

    fun importSeal(json: String) {
        NativeVault.nativeImportSeal(handle, json)
        reopen()
    }

    fun setEncrypted(path: String, encrypted: Boolean) {
        NativeVault.nativeSetEncrypted(handle, path, encrypted)
    }

    private fun reopen() {
        NativeVault.nativeClose(handle)
        val state = File(filesDir, "state").apply { mkdirs() }
        handle = NativeVault.nativeOpen(root.absolutePath, state.absolutePath)
        check(handle != 0L) { "could not reopen the vault" }
    }

    fun rawFiles(): MutableMap<String, File> {
        val out = mutableMapOf<String, File>()
        fun walk(dir: File, prefix: String) {
            dir.listFiles()?.forEach { file ->
                if (file.name.startsWith(".")) return@forEach
                if (file.isDirectory) walk(file, "$prefix${file.name}/")
                else out["$prefix${file.name}"] = file
            }
        }
        walk(root, "")
        return out
    }

    fun create(title: String): String {
        val clean = title.trim()
            .replace(Regex("[/\\\\:*?\"<>|]"), "-")
            .ifBlank { "Nova nota" }
        var path = "$clean.md"
        var n = 2
        while (notes().any { it.path == path }) {
            path = "$clean ${n++}.md"
        }
        save(path, "# $clean\n\n")
        return path
    }

    override fun close() {
        NativeVault.nativeClose(handle)
    }
}
