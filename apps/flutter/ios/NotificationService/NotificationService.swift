// 锁屏推送（m2-spec D-35，backlog F-21）的通知扩展。推送中转经 APNs 发来 `mutable-content` 的通知，
// 主机的 Web Push 密文原样放在 `userInfo["ip"]`。系统先叫起这里：用钥匙串共用分组里的私钥解开，
// 改写标题、正文和点开去哪；解不开（密钥删了、别人乱发的）就显示中转给的兜底“有一条新通知”。
// App 被杀掉、手机锁着时也是这样，不经过 App，也不启动 Flutter。

import Foundation
import UserNotifications

final class NotificationService: UNNotificationServiceExtension {
  private let lock = NSLock()
  private var deliver: ((UNNotificationContent) -> Void)?
  private var fallback: UNNotificationContent?

  override func didReceive(
    _ request: UNNotificationRequest,
    withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
  ) {
    lock.lock()
    deliver = contentHandler
    fallback = request.content
    lock.unlock()

    guard let sealed = request.content.userInfo[PushNotice.sealedKey] as? String,
      let keys = try? PushKeychain.load(),
      let message = WebPushCrypto.open(sealed: sealed, privateKey: keys.privateKey, auth: keys.auth)
    else {
      finish(request.content)
      return
    }

    let center = UNUserNotificationCenter.current()
    center.getDeliveredNotifications { delivered in
      // 同一件事已经显示过的：App 在后台还连着主机时自己弹过一次（F-16），或者是之前的推送。
      let same = delivered.map(\.request).filter { PushNotice.sameEvent($0, tag: message.tag) }
      if message.silent {
        // “已在别处处理”。iOS 不让通知扩展不显示，就撤掉旧的，换成这条不响铃、不亮屏的。
        // 它和原来那条的 apns-collapse-id 一样（主机的 Topic 由标签算出），在通知中心里也是替换。
        center.removeDeliveredNotifications(withIdentifiers: same.map(\.identifier))
      } else {
        // App 自己弹的那条 id 不一样，撤掉免得一件事两条；这条只替换、不再响铃（和 Android 的 setOnlyAlertOnce 一样）。
        let local = same.map(\.identifier).filter { $0 != request.identifier }
        if !local.isEmpty { center.removeDeliveredNotifications(withIdentifiers: local) }
      }
      self.finish(PushNotice.content(for: message, quiet: message.silent || !same.isEmpty))
    }
  }

  /// 系统给的时间（约 30 秒）快到了还没弄完：显示兜底的。
  override func serviceExtensionTimeWillExpire() {
    lock.lock()
    let content = fallback
    lock.unlock()
    if let content { finish(content) }
  }

  /// 只交一次。
  private func finish(_ content: UNNotificationContent) {
    lock.lock()
    let handler = deliver
    deliver = nil
    lock.unlock()
    handler?(content)
  }
}
