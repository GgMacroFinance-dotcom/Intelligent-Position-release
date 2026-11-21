package dev.intelligent_position.ip_app

import android.content.Intent
import android.os.Handler
import android.os.Looper
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    /** 点锁屏推送的通知打开 App 时，要去的对话（空串是去“待办”）；Flutter 那边取走后清掉。 */
    private var tapped: String? = null
    private var channel: MethodChannel? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        tapped = intent?.getStringExtra(IpPush.EXTRA_THREAD)
        val ch = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "ip/push")
        channel = ch
        val main = Handler(Looper.getMainLooper())
        IpPush.listener = { method, arg -> main.post { ch.invokeMethod(method, arg) } }
        ch.setMethodCallHandler { call, result ->
            when (call.method) {
                "available" -> result.success(IpPush.configured())
                "registration" -> IpPush.registration(applicationContext) { reg, error ->
                    main.post { if (error != null) result.error("push", error, null) else result.success(reg) }
                }
                "reset" -> {
                    IpPush.reset(applicationContext)
                    result.success(null)
                }
                "takeTapped" -> {
                    result.success(tapped)
                    tapped = null
                }
                else -> result.notImplemented()
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        val thread = intent.getStringExtra(IpPush.EXTRA_THREAD) ?: return
        channel?.invokeMethod("tapped", thread)
    }

    override fun onDestroy() {
        IpPush.listener = null
        channel = null
        super.onDestroy()
    }
}
