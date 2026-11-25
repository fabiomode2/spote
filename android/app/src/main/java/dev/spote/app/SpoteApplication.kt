package dev.spote.app

import android.app.Application
import dev.spote.app.remote.RemoteResolver

class SpoteApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        // NewPipeExtractor necesita su Downloader antes del primer search/resolve.
        runCatching { RemoteResolver.init(this) }
    }
}
