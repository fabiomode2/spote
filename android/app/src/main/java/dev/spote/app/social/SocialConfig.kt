package dev.spote.app.social

import android.content.Context
import android.content.SharedPreferences
import android.util.Log
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

/**
 * Persistence for the social session: server URL, personal client key, session
 * token, display name and the shared invite code.
 *
 * Values live in [EncryptedSharedPreferences] backed by an AES-256-GCM master
 * key in the Android Keystore.
 *
 * Fallback policy: if the Keystore/crypto provider cannot be initialized
 * (corrupted Keystore entry, broken OEM provider, missing AndroidX security
 * support), this class falls back to a private `MODE_PRIVATE` preferences file
 * and logs a warning. That file is app-private but **not encrypted at rest**.
 * There is deliberately no other plaintext path; callers can check
 * [storageIsEncrypted] and refuse to save a token when it is `false` if their
 * threat model requires it.
 *
 * Use one instance per process (e.g. from the application container). The
 * class is safe to read from any thread; writes are applied asynchronously
 * like any [SharedPreferences] write.
 */
class SocialConfig(context: Context) {

    private val appContext = context.applicationContext
    private val store: Store = createStore(appContext)

    /** `true` when values are protected by the Android Keystore. */
    val storageIsEncrypted: Boolean get() = store.encrypted

    /** Base URL without a trailing slash, e.g. `https://spote.example.com`. */
    var serverUrl: String
        get() = store.prefs.getString(KEY_SERVER_URL, "").orEmpty()
        set(value) = store.prefs.edit().putString(KEY_SERVER_URL, value.trim()).apply()

    /** Per-device/personal client key sent as `X-Spote-Key`. Never logged. */
    var clientKey: String
        get() = store.prefs.getString(KEY_CLIENT_KEY, "").orEmpty()
        set(value) = store.prefs.edit().putString(KEY_CLIENT_KEY, value).apply()

    /** Session token sent as `Authorization: Bearer <token>`. Never logged. */
    var token: String
        get() = store.prefs.getString(KEY_TOKEN, "").orEmpty()
        set(value) = store.prefs.edit().putString(KEY_TOKEN, value).apply()

    /** Last name used to log in; prefilled in the login form. */
    var displayName: String
        get() = store.prefs.getString(KEY_DISPLAY_NAME, "").orEmpty()
        set(value) = store.prefs.edit().putString(KEY_DISPLAY_NAME, value.trim()).apply()

    /** Shared group invite; kept so the user does not retype it. */
    var invite: String
        get() = store.prefs.getString(KEY_INVITE, "").orEmpty()
        set(value) = store.prefs.edit().putString(KEY_INVITE, value.trim()).apply()

    /** `true` when the client can open a WebSocket without prompting again. */
    val isLoggedIn: Boolean
        get() = serverUrl.isNotEmpty() && clientKey.isNotEmpty() && token.isNotEmpty()

    /** Drops the session token but keeps URL/key/name/invite for a re-login. */
    fun clearSession() {
        store.prefs.edit().remove(KEY_TOKEN).apply()
    }

    /** Removes every stored value (logout + "forget this server"). */
    fun clearAll() {
        store.prefs.edit().clear().apply()
    }

    /**
     * Synchronously wipes all values. Use before handing the device over or
     * when the user asks to remove the account. The SharedPreferences file may
     * remain on disk, but it no longer contains credentials; the Keystore key
     * is left in place so a future login can reuse it.
     */
    fun destroy() {
        store.prefs.edit().clear().commit()
    }

    private data class Store(
        val prefs: SharedPreferences,
        /** `false` only for the app-private plaintext fallback. */
        val encrypted: Boolean,
    )

    private companion object {
        const val TAG = "SocialConfig"

        const val SECURE_FILE = "spote_social_secure"
        const val FALLBACK_FILE = "spote_social_fallback"

        const val KEY_SERVER_URL = "server_url"
        const val KEY_CLIENT_KEY = "client_key"
        const val KEY_TOKEN = "token"
        const val KEY_DISPLAY_NAME = "display_name"
        const val KEY_INVITE = "invite"

        fun createStore(context: Context): Store = try {
            val masterKey = MasterKey.Builder(context)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build()
            val prefs = EncryptedSharedPreferences.create(
                context,
                SECURE_FILE,
                masterKey,
                EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
            )
            Store(prefs, encrypted = true)
        } catch (error: Exception) {
            // Keystore unavailable/corrupted. Keep the app usable, but make the
            // downgrade visible in logs and through `storageIsEncrypted`.
            Log.w(
                TAG,
                "EncryptedSharedPreferences unavailable; using app-private plaintext fallback",
                error,
            )
            Store(
                context.getSharedPreferences(FALLBACK_FILE, Context.MODE_PRIVATE),
                encrypted = false,
            )
        }
    }
}
