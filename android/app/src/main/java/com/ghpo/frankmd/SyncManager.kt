package com.ghpo.frankmd

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.security.MessageDigest

data class SyncResult(val downloaded: Int, val uploaded: Int, val deleted: Int, val conflicts: Int)

object SyncManager {
    private fun encode(path: String): String =
        path.split("/").joinToString("/") { URLEncoder.encode(it, "UTF-8").replace("+", "%20") }

    private fun open(method: String, url: String, token: String): HttpURLConnection =
        (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = method
            setRequestProperty("Authorization", "Bearer $token")
            connectTimeout = 8_000
            readTimeout = 20_000
        }

    private fun sha(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buffer = ByteArray(64 * 1024)
            while (true) {
                val read = input.read(buffer)
                if (read <= 0) break
                digest.update(buffer, 0, read)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    suspend fun sync(
        vault: VaultStore,
        settings: SettingsStore,
        onProgress: (done: Int, total: Int) -> Unit = { _, _ -> },
    ): SyncResult = withContext(Dispatchers.IO) {
        val server = settings.serverUrl
        val token = settings.token
        require(server.startsWith("http://") || server.startsWith("https://")) { "URL do servidor inválida" }
        require(token.isNotBlank()) { "Token não configurado" }

        val remote = remoteManifest(server, token)
        val local = vault.rawFiles().mapValues { (_, file) -> file to sha(file) }
        val base = settings.syncBase()
        val nextBase = mutableMapOf<String, String>()
        var downloaded = 0
        var uploaded = 0
        var deleted = 0
        var conflicts = 0

        val paths = (remote.keys + local.keys + base.keys).toSortedSet()
        onProgress(0, paths.size)
        for ((index, path) in paths.withIndex()) {
            onProgress(index, paths.size)
            val remoteRevision = remote[path]
            val localEntry = local[path]
            val localRevision = localEntry?.second
            val baseRevision = base[path]

            when {
                localRevision != null && remoteRevision == null -> {
                    if (baseRevision != null && localRevision == baseRevision) {
                        // Deleted on the server while unchanged locally.
                        localEntry.first.delete()
                        deleted++
                    } else {
                        nextBase[path] = upload(server, token, path, localEntry.first, null)
                        uploaded++
                    }
                }

                localRevision == null && remoteRevision != null -> {
                    if (baseRevision != null && remoteRevision == baseRevision) {
                        // Deleted locally while unchanged remotely.
                        deleteRemote(server, token, path, remoteRevision)
                        deleted++
                    } else {
                        download(server, token, path, File(vault.root, path))
                        nextBase[path] = remoteRevision
                        downloaded++
                    }
                }

                localRevision != null && remoteRevision != null -> {
                    when {
                        localRevision == remoteRevision -> nextBase[path] = remoteRevision
                        baseRevision == localRevision -> {
                            download(server, token, path, localEntry.first)
                            nextBase[path] = remoteRevision
                            downloaded++
                        }
                        baseRevision == remoteRevision -> {
                            nextBase[path] = upload(server, token, path, localEntry.first, remoteRevision)
                            uploaded++
                        }
                        else -> {
                            val conflict = conflictCopy(localEntry.first)
                            if (conflict != null) {
                                // Keep the offline edit as a new local file and
                                // take the server version for the original path.
                                nextBase.remove(conflict.path)
                            }
                            download(server, token, path, localEntry.first)
                            nextBase[path] = remoteRevision
                            conflicts++
                        }
                    }
                }
            }
        }
        onProgress(paths.size, paths.size)
        settings.saveSyncBase(nextBase)
        SyncResult(downloaded, uploaded, deleted, conflicts)
    }

    private fun remoteManifest(server: String, token: String): Map<String, String> {
        val connection = open("GET", "$server/api/sync/manifest", token)
        return try {
            check(connection.responseCode == 200) { "Manifest HTTP ${connection.responseCode}" }
            val files = JSONObject(connection.inputStream.bufferedReader().readText())
                .getJSONArray("files")
            buildMap {
                for (i in 0 until files.length()) {
                    val item = files.getJSONObject(i)
                    put(item.getString("path"), item.getString("revision"))
                }
            }
        } finally {
            connection.disconnect()
        }
    }

    private fun upload(
        server: String,
        token: String,
        path: String,
        file: File,
        expected: String?,
    ): String {
        val query = buildString {
            append("?path=").append(encode(path))
            if (expected != null) append("&expected_revision=").append(expected)
        }
        val connection = open("PUT", "$server/api/sync/file$query", token)
        return try {
            connection.doOutput = true
            connection.setRequestProperty("Content-Type", "application/octet-stream")
            connection.outputStream.use { output -> output.write(file.readBytes()) }
            val code = connection.responseCode
            check(code in 200..299) { "Upload HTTP $code: $path" }
            JSONObject(connection.inputStream.bufferedReader().readText()).getString("revision")
        } finally {
            connection.disconnect()
        }
    }

    private fun download(server: String, token: String, path: String, file: File) {
        val connection = open("GET", "$server/api/sync/file?path=${encode(path)}", token)
        try {
            check(connection.responseCode == 200) { "Download HTTP ${connection.responseCode}: $path" }
            file.parentFile?.mkdirs()
            connection.inputStream.use { input -> file.outputStream().use { input.copyTo(it) } }
        } finally {
            connection.disconnect()
        }
    }

    private fun deleteRemote(server: String, token: String, path: String, revision: String) {
        val connection = open(
            "DELETE",
            "$server/api/sync/file?path=${encode(path)}&expected_revision=$revision",
            token,
        )
        try {
            check(connection.responseCode in 200..299) {
                "Delete HTTP ${connection.responseCode}: $path"
            }
        } finally {
            connection.disconnect()
        }
    }

    private fun conflictCopy(file: File): File? {
        val stamp = java.time.LocalDateTime.now()
            .format(java.time.format.DateTimeFormatter.ofPattern("yyyy-MM-dd HHmmss"))
        val base = file.nameWithoutExtension.ifBlank { file.name }
        val ext = file.extension.takeIf { it.isNotBlank() }?.let { ".$it" } ?: ""
        var target = File(file.parentFile, "$base (conflito Android $stamp)$ext")
        var n = 2
        while (target.exists()) {
            target = File(file.parentFile, "$base (conflito Android $stamp $n)$ext")
            n++
        }
        return runCatching { file.copyTo(target, overwrite = false) }.getOrNull()
    }
}
