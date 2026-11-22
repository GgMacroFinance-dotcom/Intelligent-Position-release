package dev.intelligent_position.ip_app

import java.util.Base64
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

/** 锁屏推送的解密（F-21）：用 RFC 8291 附录 A 的例子逐字节核对，和主机那边的 webpush.rs 用的是同一组数。 */
class WebPushCryptoTest {
    private fun b64(s: String) = Base64.getUrlDecoder().decode(s.replace(Regex("\\s"), ""))

    private val uaPublic = b64("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4")
    private val uaPrivate = WebPushCrypto.privateFromScalar(b64("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94"))
    private val auth = b64("BTBZMqHH6r4Tts7J_aSIgg")
    private val body = b64(
        """DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml
           mlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPT
           pK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN""",
    )

    @Test
    fun decryptsTheRfc8291Example() {
        val plain = WebPushCrypto.decrypt(body, uaPrivate, uaPublic, auth)
        assertEquals("When I grow up, I want to be a watermelon", String(plain))
    }

    @Test
    fun tamperedOrWrongKeysFail() {
        val bad = body.copyOf().also { it[it.size - 1] = (it[it.size - 1].toInt() xor 1).toByte() }
        assertThrows(Exception::class.java) { WebPushCrypto.decrypt(bad, uaPrivate, uaPublic, auth) }
        assertThrows(Exception::class.java) { WebPushCrypto.decrypt(body, uaPrivate, uaPublic, ByteArray(16)) }
        assertThrows(Exception::class.java) { WebPushCrypto.decrypt(body.copyOf(30), uaPrivate, uaPublic, auth) }
    }

    @Test
    fun generatedKeysRoundTrip() {
        val (priv, pub) = WebPushCrypto.generate()
        assertEquals(65, pub.size)
        val key = WebPushCrypto.privateFromPkcs8(priv)
        assertArrayEquals(pub, WebPushCrypto.rawPublic(WebPushCrypto.publicFromRaw(pub)))
        assertEquals("EC", key.algorithm)
    }

    @Test
    fun alertIdMatchesTheDartSide() {
        // notifications.dart 的 alertId：同一个标签两边算出同一个数，锁屏推送和 App 自己弹的通知互相替换。
        // 期望值按 Dart 那边的算法（UTF-16 码元的 FNV-1a 取 31 位）算出，notify_test.dart 里核对同样的数。
        assertEquals(18652613, WebPushCrypto.alertId(""))
        assertEquals(518277434, WebPushCrypto.alertId("approval:a1"))
        assertEquals(157519743, WebPushCrypto.alertId("thread:对话"))
    }
}
