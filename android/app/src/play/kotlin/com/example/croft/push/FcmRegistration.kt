// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.push

import android.content.Context
import com.example.croft.BuildConfig
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.URL
import java.security.KeyPairGenerator
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import java.util.Base64
import javax.net.ssl.HttpsURLConnection
import org.json.JSONObject

/**
 * Registering the app's FCM token with the push gateway (T34d): the endpoint the gateway gives for it, with Web Push keys for the
 * Companion. [PushRuntime] subscribes it.
 *
 * The Play build does not decrypt the pings (they are empty, T28): FCM delivers to this app only what the app's Firebase project sends,
 * which is the gateway, and the gateway forwards only messages signed with the Companion's VAPID key the endpoint is bound to. The Web
 * Push keys the Companion needs are made here; the private key is not kept, because nothing is ever decrypted with it.
 *
 * Kept in a private preferences file: the Companion's VAPID public key, a hash of the token, and the endpoint with its public keys,
 * so that each start subscribes the same endpoint again without asking the gateway. The endpoint is bound to the Companion's key: one
 * that leaks cannot be sent to without the Companion's private key (gateway/README.md).
 */
internal object FcmRegistration {
    private const val PREFS = "croft-push-fcm"

    fun savedVapid(context: Context): String? = prefs(context).getString("vapid", null)

    fun forget(context: Context) {
        prefs(context).edit().clear().commit()
    }

    /** The endpoint for [token] at the Companion whose key is [vapid]: the kept one while both are unchanged, else a new one from the gateway. */
    suspend fun endpoint(context: Context, token: String, vapid: String): StoreStart.Endpoint? = withContext(Dispatchers.IO) {
        try {
            val p = prefs(context)
            val tokenHash = sha256(token + "\u0000" + vapid)
            val kept = StoreStart.Endpoint(
                p.getString("endpoint", null) ?: "",
                p.getString("p256dh", null) ?: "",
                p.getString("auth", null) ?: "",
            )
            if (p.getString("token", null) == tokenHash && kept.url.isNotEmpty() && kept.p256dh.isNotEmpty() && kept.auth.isNotEmpty()) {
                return@withContext kept
            }
            val url = gatewayEndpoint(token, vapid) ?: return@withContext null
            val (p256dh, auth) = webPushKeys()
            p.edit().putString("vapid", vapid).putString("token", tokenHash).putString("endpoint", url)
                .putString("p256dh", p256dh).putString("auth", auth).commit()
            StoreStart.Endpoint(url, p256dh, auth)
        } catch (e: Exception) {
            null
        }
    }

    /** `POST /register` at the gateway (gateway/README.md): the endpoint it answers, or null when it refused. */
    private fun gatewayEndpoint(token: String, vapid: String): String? {
        val base = BuildConfig.PUSH_GATEWAY.trimEnd('/')
        if (!base.startsWith("https://")) return null
        val c = URL("$base/register").openConnection() as HttpsURLConnection
        try {
            c.instanceFollowRedirects = false
            c.connectTimeout = 15_000
            c.readTimeout = 15_000
            c.requestMethod = "POST"
            c.doOutput = true
            c.setRequestProperty("Content-Type", "application/json")
            val body = JSONObject().put("token", token).put("vapid", vapid).toString().toByteArray()
            c.outputStream.use { it.write(body) }
            if (c.responseCode != 200) return null
            val answer = c.inputStream.use { input ->
                val buf = ByteArray(4097)
                var n = 0
                while (n < buf.size) {
                    val r = input.read(buf, n, buf.size - n)
                    if (r < 0) break
                    n += r
                }
                if (n > 4096) return null
                String(buf, 0, n, Charsets.UTF_8)
            }
            val endpoint = JSONObject(answer).optString("endpoint", "")
            // The gateway's own endpoints only: never subscribe an address it did not make.
            return endpoint.takeIf { it.startsWith("$base/wp/") && it.length <= 2048 }
        } catch (e: org.json.JSONException) {
            return null
        } finally {
            c.disconnect()
        }
    }

    /** A P-256 public key (uncompressed, base64url) and a 16-byte auth secret (base64url) for the Companion's Web Push. */
    private fun webPushKeys(): Pair<String, String> {
        val g = KeyPairGenerator.getInstance("EC")
        g.initialize(ECGenParameterSpec("secp256r1"))
        val pub = g.generateKeyPair().public as ECPublicKey
        val point = ByteArray(65)
        point[0] = 4
        fixed32(pub.w.affineX.toByteArray()).copyInto(point, 1)
        fixed32(pub.w.affineY.toByteArray()).copyInto(point, 33)
        val auth = ByteArray(16).also { SecureRandom().nextBytes(it) }
        val b64 = Base64.getUrlEncoder().withoutPadding()
        return b64.encodeToString(point) to b64.encodeToString(auth)
    }

    /** A big-endian integer as exactly 32 bytes (BigInteger adds a sign byte or drops leading zeros). */
    private fun fixed32(b: ByteArray): ByteArray {
        val out = ByteArray(32)
        val src = if (b.size > 32) b.copyOfRange(b.size - 32, b.size) else b
        src.copyInto(out, 32 - src.size)
        return out
    }

    private fun sha256(s: String): String =
        MessageDigest.getInstance("SHA-256").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }

    private fun prefs(context: Context) = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}
