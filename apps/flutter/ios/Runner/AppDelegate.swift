import Flutter
import UIKit
import UserNotifications

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    // 通知的代理要在启动完以前设好，点通知叫起 App 时才收得到。锁屏推送的通知（F-21）这里处理，
    // 其余的（flutter_local_notifications 弹的，F-16）由 super 转给插件。
    UNUserNotificationCenter.current().delegate = self
    IpPush.shared.launched()
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    IpPush.shared.attach(messenger: engineBridge.applicationRegistrar.messenger())
  }

  // ---------- 锁屏推送（F-21）：APNs 设备令牌 ----------

  override func application(
    _ application: UIApplication,
    didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
  ) {
    IpPush.shared.didRegister(deviceToken)
    super.application(application, didRegisterForRemoteNotificationsWithDeviceToken: deviceToken)
  }

  override func application(
    _ application: UIApplication,
    didFailToRegisterForRemoteNotificationsWithError error: Error
  ) {
    IpPush.shared.didFail(error)
    super.application(application, didFailToRegisterForRemoteNotificationsWithError: error)
  }

  // ---------- 通知：锁屏推送的自己处理，其余的交给插件 ----------

  override func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    willPresent notification: UNNotification,
    withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
  ) {
    guard PushNotice.isPush(notification.request.content.userInfo) else {
      return super.userNotificationCenter(center, willPresent: notification, withCompletionHandler: completionHandler)
    }
    // App 在前台时不弹（和 F-16 一样）。主机知道 App 在前台时本来不推，这是刚切到前台时还在路上的。
    completionHandler([])
  }

  override func userNotificationCenter(
    _ center: UNUserNotificationCenter,
    didReceive response: UNNotificationResponse,
    withCompletionHandler completionHandler: @escaping () -> Void
  ) {
    let userInfo = response.notification.request.content.userInfo
    guard PushNotice.isPush(userInfo) else {
      return super.userNotificationCenter(center, didReceive: response, withCompletionHandler: completionHandler)
    }
    if response.actionIdentifier == UNNotificationDefaultActionIdentifier {
      IpPush.shared.opened(userInfo)
    }
    completionHandler()
  }
}
