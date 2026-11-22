// 锁屏推送（m2-spec D-35，backlog F-21）怎么显示：通知扩展解开以后改写成什么样，App 怎么认出点的是它。
// 规则和 F-16 的本地通知一样：同一件事（同一个标签）的新通知替换旧的、不再响铃；“已在别处处理”撤掉旧的。

import Foundation
import UserNotifications

enum PushNotice {
  /// 改写后的通知在 `userInfo` 里带的：点开要去的对话（空串是去“待办”）和标签。
  static let threadKey = "ip_thread"
  static let tagKey = "ip_tag"
  /// 中转放密文的字段。解不开时显示的兜底通知还带着它。
  static let sealedKey = "ip"

  /// 是不是锁屏推送的通知（解开改写过的，或者解不开时的兜底）。不是的交给 flutter_local_notifications。
  static func isPush(_ userInfo: [AnyHashable: Any]) -> Bool {
    userInfo[threadKey] != nil || userInfo[sealedKey] != nil
  }

  /// 点开要去的对话；没有对话、或者是解不开的兜底通知时为空串（去“待办”）。
  static func thread(of userInfo: [AnyHashable: Any]) -> String {
    userInfo[threadKey] as? String ?? ""
  }

  /// 已经显示的这条是不是同一件事的：App 自己弹的（F-16，flutter_local_notifications 用 alertId 的十进制当 id），
  /// 或者之前的锁屏推送（id 是中转给的 apns-collapse-id，标签记在 userInfo 里）。
  static func sameEvent(_ request: UNNotificationRequest, tag: String) -> Bool {
    request.identifier == String(WebPushCrypto.alertId(tag)) || request.content.userInfo[tagKey] as? String == tag
  }

  /// 解开以后的通知。`quiet`：同一件事已经提醒过，或者是“已在别处处理”，只替换、不响铃、不亮屏。
  static func content(for message: PushMessage, quiet: Bool) -> UNNotificationContent {
    let c = UNMutableNotificationContent()
    c.title = message.title
    c.body = message.body
    // 同一个对话的通知归成一组。
    c.threadIdentifier = message.threadId ?? ""
    c.userInfo = [threadKey: message.threadId ?? "", tagKey: message.tag]
    c.sound = quiet ? nil : .default
    c.interruptionLevel = quiet ? .passive : .active
    if message.silent { c.relevanceScore = 0 }
    return c
  }
}
