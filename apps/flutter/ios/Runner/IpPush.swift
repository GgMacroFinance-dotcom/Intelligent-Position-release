// 锁屏推送（m2-spec D-35，backlog F-21）的 iPhone 这一半里 App 的部分：向 APNs 要设备令牌、管密钥、
// 点了锁屏推送的通知去哪，和 Flutter 那边的 `ip/push` 方法通道（lib/src/push/lock_push.dart 的
// NativePushChannel，约定和 Android 的 MainActivity.kt 一样）。解密、改写通知在通知扩展里做
// （NotificationService/NotificationService.swift），App 被杀掉时也能显示。

import Flutter
import UIKit
import UserNotifications

final class IpPush {
  static let shared = IpPush()

  private var channel: FlutterMethodChannel?
  /// 在等 APNs 设备令牌的 `registration` 调用。
  private var waiting: [FlutterResult] = []
  private var timeout: DispatchWorkItem?
  /// 点锁屏推送的通知打开 App 时要去的对话（空串是去“待办”）；Flutter 那边取走前先存着。
  private var tapped: String?
  /// Flutter 那边取过一次 `takeTapped` 了：之后点的通知直接发 `tapped` 过去。
  private var dartReady = false

  /// 上次拿到的设备令牌。也表示锁屏推送开着：`reset` 时清掉。
  private static let lastTokenKey = "ip_push.last_token"

  /// App 启动时：锁屏推送开着的话重新要一次令牌（苹果建议每次启动都要，令牌可能变了）。
  func launched() {
    if UserDefaults.standard.string(forKey: Self.lastTokenKey) != nil {
      UIApplication.shared.registerForRemoteNotifications()
    }
  }

  /// Flutter 引擎建好了。方法都在主线程上调。
  func attach(messenger: FlutterBinaryMessenger) {
    let ch = FlutterMethodChannel(name: "ip/push", binaryMessenger: messenger)
    channel = ch
    ch.setMethodCallHandler { [weak self] call, result in
      guard let self else { return result(FlutterMethodNotImplemented) }
      switch call.method {
      case "available":
        result(PushKeychain.usable())
      case "registration":
        self.registration(result)
      case "reset":
        self.reset()
        result(nil)
      case "takeTapped":
        self.dartReady = true
        result(self.tapped)
        self.tapped = nil
      default:
        result(FlutterMethodNotImplemented)
      }
    }
  }

  // ---------- 登记 ----------

  /// 要设备令牌，连同 Web Push 的公钥和 auth 交给 Flutter 那边（它再向中转登记）。没有密钥时生成一套。
  private func registration(_ result: @escaping FlutterResult) {
    waiting.append(result)
    guard waiting.count == 1 else { return }
    let timer = DispatchWorkItem { [weak self] in
      self?.finish(token: nil, error: "等 APNs 的设备令牌超时，看看网络")
    }
    timeout = timer
    DispatchQueue.main.asyncAfter(deadline: .now() + 30, execute: timer)
    UIApplication.shared.registerForRemoteNotifications()
  }

  func didRegister(_ deviceToken: Data) {
    let token = deviceToken.hexLower
    let defaults = UserDefaults.standard
    let last = defaults.string(forKey: Self.lastTokenKey)
    defaults.set(token, forKey: Self.lastTokenKey)
    if waiting.isEmpty, let last, last != token {
      // 启动时重新要到的令牌变了：旧的订阅地址作废，Flutter 那边开着的话马上重新登记。
      channel?.invokeMethod("tokenChanged", arguments: nil)
    }
    finish(token: token, error: nil)
  }

  func didFail(_ error: Error) {
    finish(token: nil, error: "APNs 不给设备令牌：\(error.localizedDescription)")
  }

  private func finish(token: String?, error: String?) {
    timeout?.cancel()
    timeout = nil
    let results = waiting
    waiting = []
    guard !results.isEmpty else { return }
    let reply: Any
    if let token {
      do {
        let keys = try PushKeychain.loadOrCreate()
        reply = ["token": token, "p256dh": keys.publicKey.base64URL, "auth": keys.auth.base64URL]
      } catch {
        reply = FlutterError(code: "push", message: "推送密钥用不了：\(error)", details: nil)
      }
    } else {
      reply = FlutterError(code: "push", message: error, details: nil)
    }
    results.forEach { $0(reply) }
  }

  /// 关掉锁屏推送：删掉密钥，不再要令牌。之后发来的都解不开。
  private func reset() {
    PushKeychain.delete()
    UserDefaults.standard.removeObject(forKey: Self.lastTokenKey)
    UIApplication.shared.unregisterForRemoteNotifications()
  }

  // ---------- 点通知 ----------

  /// 点了一条锁屏推送的通知。App 是被它叫起来的话先存着，等 Flutter 那边来取。
  func opened(_ userInfo: [AnyHashable: Any]) {
    let thread = PushNotice.thread(of: userInfo)
    if dartReady, let channel {
      channel.invokeMethod("tapped", arguments: thread)
    } else {
      tapped = thread
    }
  }
}
