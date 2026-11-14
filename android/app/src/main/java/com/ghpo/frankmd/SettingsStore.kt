package com.ghpo.frankmd

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import org.json.JSONObject
import java.io.File

class SettingsStore(context: Context) {
    private val prefs = context.getSharedPreferences("frankmd", Context.MODE_PRIVATE)
    private val securePrefs = EncryptedSharedPreferences.create(
        context,
        "frankmd_secure",
        MasterKey.Builder(context).setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build(),
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )
    private val editMarker = File(context.filesDir, "state/editing")

    var serverUrl: String
        get() = prefs.getString("server_url", "").orEmpty()
        set(value) = prefs.edit().putString("server_url", value.trimEnd('/')).apply()

    var themeMode: String
        get() = prefs.getString("theme_mode", "auto").orEmpty()
        set(value) = prefs.edit().putString("theme_mode", value).apply()

    var token: String
        get() = securePrefs.getString("token", null)
            ?: prefs.getString("token", "").orEmpty()
        set(value) = securePrefs.edit()
            .putString(
                "token",
                value.trim()
                    .removePrefix("Bearer ")
                    .trim()
                    .trim('"')
                    .filterNot(Char::isWhitespace),
            )
            .apply()

    fun syncBase(): Map<String, String> {
        val raw = securePrefs.getString("sync_base", null)
            ?: prefs.getString("sync_base", "{}").orEmpty()
        return runCatching {
            val objectValue = JSONObject(raw)
            buildMap {
                objectValue.keys().forEach { key -> put(key, objectValue.getString(key)) }
            }
        }.getOrDefault(emptyMap())
    }

    fun saveSyncBase(base: Map<String, String>) {
        val objectValue = JSONObject()
        base.toSortedMap().forEach { (path, revision) -> objectValue.put(path, revision) }
        securePrefs.edit().putString("sync_base", objectValue.toString()).apply()
    }

    fun markEditing() {
        editMarker.parentFile?.mkdirs()
        editMarker.writeText(System.currentTimeMillis().toString())
    }

    fun clearEditing() {
        editMarker.delete()
    }

    fun isEditingRecent(ttlMillis: Long = 120_000): Boolean {
        if (!editMarker.isFile) return false
        val touched = editMarker.readText().trim().toLongOrNull() ?: return false
        return System.currentTimeMillis() - touched in 0..ttlMillis
    }
}
