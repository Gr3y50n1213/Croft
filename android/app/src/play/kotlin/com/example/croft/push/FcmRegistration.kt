// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.push

import android.content.Context
import com.example.croft.BuildConfig
import java.io.IOException
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
 * Registering the app's FCM token with the push gateway and subscribing the endpoint it gives at the Companion (T34d).
 *
 * The Play build does not decrypt the pings (they are empty, T28): FCM delivers to this app only what the app's Firebase project sends,
 * which is the gateway, and the gateway forwards only messages signed with the Companion's VAPID key the endpoint is bound to. The Web
 * Push keys the Companion needs are made here; the private key is not kept, because nothing is ever decrypted with it.
 */
internal object FcmRegistration {
    private const val PREFS = "croft-push-fcm"

    fun saveVapid(context: Context, vapid: String) {
        prefs(context).edit().putString("vapid", vapid).commit()
    }

    fun forget(context: Context) {
        prefs(context).edit().clear().commit()
    }

    /** Registers [token] (again only when it changed); true when the Companion has a subscription for it. */
    suspend fun register(context: Context, token: String): Boolean {
        val vapid = prefs(context).getString("vapid", null) ?: return false
        val push = PushRuntime.registrar(context) ?: return false
        val tokenHash = sha256(token + "\u0000" + vapid)
        if (push.isSubscribed() && prefs(context).getString("token", null) == tokenHash) return true
        val endpoint = try {
            gatewayEndpoint(token, vapid)
        } catch (e: IOException) {
            return false
        } ?: return false
        val (p256dh, auth) = webPushKeys()
        if (!push.onEndpoint(endpoint, p256dh, auth)) return false
        prefs(context).edit().putString("token", tokenHash).commit()
        return true
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
