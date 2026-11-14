package com.ghpo.frankmd

import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters

class SyncWorker(
    appContext: Context,
    params: WorkerParameters,
) : CoroutineWorker(appContext, params) {
    override suspend fun doWork(): Result {
        val settings = SettingsStore(applicationContext)
        if (settings.token.isBlank()) return Result.success()
        if (settings.isEditingRecent()) return Result.retry()
        val vault = VaultStore(applicationContext.filesDir)
        return try {
            SyncManager.sync(vault, settings)
            Result.success()
        } catch (error: Exception) {
            android.util.Log.e("FrankMDSync", "sync failed", error)
            Result.retry()
        } finally {
            vault.close()
        }
    }
}
