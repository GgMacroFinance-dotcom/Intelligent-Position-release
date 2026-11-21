package dev.intelligent_position.ip_app

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import com.google.firebase.FirebaseApp
import com.google.firebase.FirebaseOptions
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import org.json.JSONObject

/**
 * 锁屏推送（m2-spec D-35，backlog F-21）的 Android 这一半：经 FCM 收推送中转转来的 Web Push 密文，
 * 用这台手机自己的私钥解开，弹系统通知。App 被系统杀掉以后 FCM 照样能叫起这个服务，不用启动 Flutter。
 *
 * Firebase 的配置不放进仓库（没有 google-services.json）：构建时用 Gradle 属性或环境变量
 * `IP_FCM_APP_ID`、`IP_FCM_API_KEY`、`IP_FCM_PROJECT_ID`、`IP_FCM_SENDER_ID` 传进来，没传时锁屏推送不可用。
 */
object IpPush {
    private const val TAG = "IpPush"
    private const val PREFS = "ip_push"
    private const val WRAP_ALIAS = "ip_push_wrap"
    const val EXTRA_THREAD = "ip_push_thread"

    /** 和 Dart 那边（notifications.dart）的两个通知渠道一样。 */
    private const val CHANNEL_NEEDS_ME = "needs_me"
    private const val CHANNEL_ALL = "all"

    /** App 开着时，令牌变了或点了通知，告诉 Flutter 那边。 */
    @Volatile var listener: ((String, Any?) -> Unit)? = null

    fun configured(): Boolean =
        BuildConfig.IP_FCM_APP_ID.isNotEmpty() && BuildConfig.IP_FCM_API_KEY.isNotEmpty() &&
            BuildConfig.IP_FCM_PROJECT_ID.isNotEmpty() && BuildConfig.IP_FCM_SENDER_ID.isNotEmpty()

    /** 按构建时给的配置建 Firebase；没配时返回 false。 */
    fun ensureFirebase(context: Context): Boolean {
        if (!configured()) return false
        if (FirebaseApp.getApps(context).isEmpty()) {
            val options = FirebaseOptions.Builder()
                .setApplicationId(BuildConfig.IP_FCM_APP_ID)
                .setApiKey(BuildConfig.IP_FCM_API_KEY)
                .setProjectId(BuildConfig.IP_FCM_PROJECT_ID)
                .setGcmSenderId(BuildConfig.IP_FCM_SENDER_ID)
                .build()
            FirebaseApp.initializeApp(context.applicationContext, options)
        }
        return true
    }

    // ---------- 密钥：私钥用 Android Keystore 里的一把 AES 密钥包起来，存在应用私有的 SharedPreferences ----------

    private fun wrapKey(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(WRAP_ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        gen.init(
            KeyGenParameterSpec.Builder(WRAP_ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return gen.generateKey()
    }

    private fun b64(bytes: ByteArray) = Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
    private fun unb64(s: String) = Base64.decode(s, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)

    /** 这台手机的 Web Push 密钥：(私钥, 公钥, auth)。没有就生成一套存起来。 */
    @Synchronized
    fun keys(context: Context): Triple<ByteArray, ByteArray, ByteArray> {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val sealed = prefs.getString("priv", null)
        val pub = prefs.getString("pub", null)
        val auth = prefs.getString("auth", null)
        if (sealed != null && pub != null && auth != null) {
            try {
                val raw = unb64(sealed)
                val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                cipher.init(Cipher.DECRYPT_MODE, wrapKey(), GCMParameterSpec(128, raw.copyOfRange(0, 12)))
                return Triple(cipher.doFinal(raw.copyOfRange(12, raw.size)), unb64(pub), unb64(auth))
            } catch (e: Exception) {
                // Keystore 的密钥没了（比如系统清了锁屏密码），重新生成，App 会重新登记。
                Log.w(TAG, "旧的推送密钥打不开，重新生成：${e.javaClass.simpleName}")
            }
        }
        val (priv, newPub) = WebPushCrypto.generate()
        val newAuth = ByteArray(16).also { java.security.SecureRandom().nextBytes(it) }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, wrapKey())
        val wrapped = cipher.iv + cipher.doFinal(priv)
        prefs.edit()
            .putString("priv", b64(wrapped))
            .putString("pub", b64(newPub))
            .putString("auth", b64(newAuth))
            .apply()
        return Triple(priv, newPub, newAuth)
    }

    /** 关掉锁屏推送：删掉密钥和 FCM 令牌，之后发来的都解不开。 */
    fun reset(context: Context) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().clear().apply()
        if (ensureFirebase(context)) FirebaseMessaging.getInstance().deleteToken()
    }

    /** 给 Flutter 那边登记用：FCM 令牌和 Web Push 的公钥、auth（base64url）。 */
    fun registration(context: Context, done: (Map<String, String>?, String?) -> Unit) {
        if (!ensureFirebase(context)) return done(null, null)
        FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
            if (!task.isSuccessful || task.result.isNullOrEmpty()) {
                done(null, "拿不到 FCM 令牌：${task.exception?.javaClass?.simpleName ?: "空的"}")
                return@addOnCompleteListener
            }
            try {
                val (_, pub, auth) = keys(context)
                done(mapOf("token" to task.result, "p256dh" to b64(pub), "auth" to b64(auth)), null)
            } catch (e: Exception) {
                done(null, "推送密钥用不了：${e.javaClass.simpleName}")
            }
        }
    }

    // ---------- 收到推送 ----------

    fun onMessage(context: Context, data: Map<String, String>) {
        val sealed = data["ip"] ?: return
        val payload = try {
            val (priv, pub, auth) = keys(context)
            val body = Base64.decode(sealed, Base64.DEFAULT)
            JSONObject(String(WebPushCrypto.decrypt(body, WebPushCrypto.privateFromPkcs8(priv), pub, auth)))
        } catch (e: Exception) {
            // 解不开的（密钥换过、别人乱发的）直接丢掉，不弹。
            Log.w(TAG, "推送解不开：${e.javaClass.simpleName}")
            return
        }
        val tag = payload.optString("tag")
        if (tag.isEmpty()) return
        val id = WebPushCrypto.alertId(tag)
        val manager = NotificationManagerCompat.from(context)
        // “已在别处处理”：原生 App 能直接撤掉原来那条，不用再弹一条盖上去。
        if (payload.optBoolean("silent")) {
            manager.cancel(id)
            return
        }
        if (!manager.areNotificationsEnabled()) return
        val urgent = data["urgency"] == "high"
        ensureChannels(context)
        val thread = payload.optString("thread_id")
        val open = Intent(context, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP
            putExtra(EXTRA_THREAD, thread)
        }
        val pending = PendingIntent.getActivity(
            context, id, open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val body = payload.optString("body")
        val notification = NotificationCompat.Builder(context, if (urgent) CHANNEL_NEEDS_ME else CHANNEL_ALL)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(payload.optString("title"))
            .apply { if (body.isNotEmpty()) setContentText(body) }
            .setPriority(if (urgent) NotificationCompat.PRIORITY_HIGH else NotificationCompat.PRIORITY_DEFAULT)
            .setContentIntent(pending)
            .setAutoCancel(true)
            // App 在后台还连着时，主机的 alert 已经按同一个 id 弹过一次（F-16），这里只替换、不再响。
            .setOnlyAlertOnce(true)
            .build()
        try {
            manager.notify(id, notification)
        } catch (e: SecurityException) {
            Log.w(TAG, "没有通知权限")
        }
    }

    private fun ensureChannels(context: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = context.getSystemService(NotificationManager::class.java)
        if (manager.getNotificationChannel(CHANNEL_NEEDS_ME) == null) {
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL_NEEDS_ME, "要我处理的", NotificationManager.IMPORTANCE_HIGH).apply {
                    description = "审批卡、员工提问、一轮失败、任务受阻"
                },
            )
        }
        if (manager.getNotificationChannel(CHANNEL_ALL) == null) {
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL_ALL, "员工回复和任务完成", NotificationManager.IMPORTANCE_DEFAULT).apply {
                    description = "通知设置选了“全部”时才有"
                },
            )
        }
    }
}

/** FCM 叫起来的服务。App 没开、被杀掉以后也会被叫起。 */
class IpPushService : FirebaseMessagingService() {
    override fun onMessageReceived(message: RemoteMessage) {
        IpPush.onMessage(applicationContext, message.data)
    }

    override fun onNewToken(token: String) {
        // 令牌变了，旧的订阅地址作废。App 开着就马上重新登记，没开的话下次连上主机时登记。
        IpPush.listener?.invoke("tokenChanged", null)
    }
}
