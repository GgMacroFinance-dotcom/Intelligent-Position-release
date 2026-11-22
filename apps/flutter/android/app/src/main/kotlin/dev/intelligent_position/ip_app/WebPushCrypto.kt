package dev.intelligent_position.ip_app

import java.math.BigInteger
import java.security.AlgorithmParameters
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.interfaces.ECPrivateKey
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import java.security.spec.ECParameterSpec
import java.security.spec.ECPoint
import java.security.spec.ECPrivateKeySpec
import java.security.spec.ECPublicKeySpec
import java.security.spec.PKCS8EncodedKeySpec
import javax.crypto.Cipher
import javax.crypto.KeyAgreement
import javax.crypto.Mac
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * Web Push 的解密（RFC 8291，`aes128gcm`），锁屏推送用（m2-spec D-35，backlog F-21）。
 * 主机按 D-33 加密，推送中转原样转来，这里用这台手机自己的私钥解开。只用 JDK 自带的密码学接口，
 * 不依赖 Android，JVM 单元测试里能跑。
 */
object WebPushCrypto {
    private val params: ECParameterSpec by lazy {
        AlgorithmParameters.getInstance("EC").run {
            init(ECGenParameterSpec("secp256r1"))
            getParameterSpec(ECParameterSpec::class.java)
        }
    }

    /** 新的一对 P-256 密钥：(PKCS#8 私钥, 65 字节未压缩公钥)。 */
    fun generate(): Pair<ByteArray, ByteArray> {
        val gen = KeyPairGenerator.getInstance("EC")
        gen.initialize(ECGenParameterSpec("secp256r1"))
        val kp = gen.generateKeyPair()
        return kp.private.encoded to rawPublic(kp.public as ECPublicKey)
    }

    fun privateFromPkcs8(der: ByteArray): ECPrivateKey =
        KeyFactory.getInstance("EC").generatePrivate(PKCS8EncodedKeySpec(der)) as ECPrivateKey

    /** 测试用：32 字节的私钥标量。 */
    fun privateFromScalar(d: ByteArray): ECPrivateKey =
        KeyFactory.getInstance("EC").generatePrivate(ECPrivateKeySpec(BigInteger(1, d), params)) as ECPrivateKey

    fun publicFromRaw(raw: ByteArray): ECPublicKey {
        require(raw.size == 65 && raw[0] == 4.toByte()) { "公钥要是 65 字节的未压缩点" }
        val x = BigInteger(1, raw.copyOfRange(1, 33))
        val y = BigInteger(1, raw.copyOfRange(33, 65))
        return KeyFactory.getInstance("EC").generatePublic(ECPublicKeySpec(ECPoint(x, y), params)) as ECPublicKey
    }

    fun rawPublic(key: ECPublicKey): ByteArray {
        fun fixed(n: BigInteger): ByteArray {
            val b = n.toByteArray()
            return when {
                b.size == 32 -> b
                b.size > 32 -> b.copyOfRange(b.size - 32, b.size)
                else -> ByteArray(32 - b.size) + b
            }
        }
        return byteArrayOf(4) + fixed(key.w.affineX) + fixed(key.w.affineY)
    }

    private fun hmac(key: ByteArray, data: ByteArray): ByteArray =
        Mac.getInstance("HmacSHA256").run {
            init(SecretKeySpec(key, "HmacSHA256"))
            doFinal(data)
        }

    /** 只要一块输出（不超过 32 字节）的 HKDF。 */
    private fun hkdf(salt: ByteArray, ikm: ByteArray, info: ByteArray, len: Int): ByteArray =
        hmac(hmac(salt, ikm), info + byteArrayOf(1)).copyOf(len)

    /**
     * 解开一条推送。`body` 是 `aes128gcm` 的整段：盐（16）、记录大小（4）、密钥 id 长度（1）、
     * 发送方的临时公钥（65）、密文。只认一条记录（主机发的都放得进一条）。
     */
    fun decrypt(body: ByteArray, priv: ECPrivateKey, uaPublic: ByteArray, auth: ByteArray): ByteArray {
        require(body.size > 21) { "推送太短" }
        val salt = body.copyOfRange(0, 16)
        val idLen = body[20].toInt() and 0xff
        require(idLen == 65 && body.size > 21 + idLen + 16) { "推送的格式不对" }
        val asPublic = body.copyOfRange(21, 21 + idLen)
        val ciphertext = body.copyOfRange(21 + idLen, body.size)

        val agreement = KeyAgreement.getInstance("ECDH")
        agreement.init(priv)
        agreement.doPhase(publicFromRaw(asPublic), true)
        val shared = agreement.generateSecret()

        val keyInfo = "WebPush: info".toByteArray() + byteArrayOf(0) + uaPublic + asPublic
        val ikm = hkdf(auth, shared, keyInfo, 32)
        val cek = hkdf(salt, ikm, "Content-Encoding: aes128gcm".toByteArray() + byteArrayOf(0), 16)
        val nonce = hkdf(salt, ikm, "Content-Encoding: nonce".toByteArray() + byteArrayOf(0), 12)

        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(cek, "AES"), GCMParameterSpec(128, nonce))
        val padded = cipher.doFinal(ciphertext)
        // 去掉填充：最后一个非零字节是分隔符，最后一条记录是 0x02。
        var end = padded.size - 1
        while (end >= 0 && padded[end] == 0.toByte()) end--
        require(end >= 0 && padded[end] == 2.toByte()) { "推送的填充不对" }
        return padded.copyOf(end)
    }

    /** 和 Dart 那边的 `alertId` 一样：标签的 FNV-1a 取 31 位，同一件事的通知用同一个 id。 */
    fun alertId(tag: String): Int {
        var h = 0x811c9dc5L
        for (c in tag) {
            h = h xor c.code.toLong()
            h = (h * 0x01000193L) and 0xffffffffL
        }
        return (h and 0x7fffffffL).toInt()
    }
}
