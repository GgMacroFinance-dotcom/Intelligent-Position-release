// 锁屏推送的解密（F-21）：用 RFC 8291 附录 A 的例子逐字节核对，和 Android 的 WebPushCryptoTest.kt、
// 主机的 webpush.rs 用的是同一组数。

#if canImport(CryptoKit)
  import CryptoKit
#else
  import Crypto
#endif
import Foundation
import XCTest

@testable import Runner

final class WebPushCryptoTests: XCTestCase {
  private func b64(_ s: String) -> Data {
    Data(base64URL: s.filter { !$0.isWhitespace })!
  }

  private var uaPublic: Data {
    b64("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4")
  }
  private var uaPrivate: P256.KeyAgreement.PrivateKey {
    try! P256.KeyAgreement.PrivateKey(rawRepresentation: b64("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94"))
  }
  private var auth: Data { b64("BTBZMqHH6r4Tts7J_aSIgg") }
  private var body: Data {
    b64(
      """
      DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml
      mlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPT
      pK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN
      """)
  }

  func testDecryptsTheRfc8291Example() throws {
    XCTAssertEqual(WebPushCrypto.publicKey(of: uaPrivate), uaPublic)
    let plain = try WebPushCrypto.decrypt(body, privateKey: uaPrivate, auth: auth)
    XCTAssertEqual(String(decoding: plain, as: UTF8.self), "When I grow up, I want to be a watermelon")
  }

  func testTamperedOrWrongKeysFail() {
    var bad = body
    bad[bad.count - 1] ^= 1
    XCTAssertThrowsError(try WebPushCrypto.decrypt(bad, privateKey: uaPrivate, auth: auth))
    XCTAssertThrowsError(try WebPushCrypto.decrypt(body, privateKey: uaPrivate, auth: Data(count: 16)))
    XCTAssertThrowsError(try WebPushCrypto.decrypt(body.prefix(30), privateKey: uaPrivate, auth: auth))
    XCTAssertThrowsError(try WebPushCrypto.decrypt(Data(), privateKey: uaPrivate, auth: auth))
    XCTAssertThrowsError(
      try WebPushCrypto.decrypt(body, privateKey: P256.KeyAgreement.PrivateKey(), auth: auth))
  }

  func testOpensWhatTheRelayForwards() {
    // 中转把主机的密文用标准 base64（带填充）放进 APNs 通知的 `ip` 字段。RFC 的明文不是 JSON，解得开但认不出。
    XCTAssertNil(WebPushCrypto.open(sealed: body.base64EncodedString(), privateKey: uaPrivate, auth: auth))
    XCTAssertNil(WebPushCrypto.open(sealed: "不是 base64", privateKey: uaPrivate, auth: auth))
  }

  func testParsesTheHostMessage() {
    let full = Data(#"{"title":"审批","body":"要付 3 元","tag":"approval:a1","thread_id":"t1"}"#.utf8)
    XCTAssertEqual(
      PushMessage.parse(full),
      PushMessage(title: "审批", body: "要付 3 元", tag: "approval:a1", threadId: "t1", silent: false))
    let handled = Data(#"{"title":"已在别处处理","body":"审批","tag":"approval:a1","silent":true}"#.utf8)
    XCTAssertEqual(PushMessage.parse(handled)?.silent, true)
    XCTAssertNil(PushMessage.parse(handled)?.threadId)
    XCTAssertNil(PushMessage.parse(Data(#"{"title":"x","body":"y","tag":""}"#.utf8)))
    XCTAssertNil(PushMessage.parse(Data(#"{"title":"x"}"#.utf8)))
    XCTAssertNil(PushMessage.parse(Data("not json".utf8)))
  }

  func testGeneratedKeysAndEncodings() throws {
    let (key, auth) = WebPushCrypto.generate()
    let pub = WebPushCrypto.publicKey(of: key)
    XCTAssertEqual(pub.count, 65)
    XCTAssertEqual(pub.first, 4)
    XCTAssertEqual(auth.count, 16)
    XCTAssertNotEqual(WebPushCrypto.generate().1, auth)
    // 存进钥匙串的是 32 字节的私钥，读回来是同一把。
    XCTAssertEqual(
      WebPushCrypto.publicKey(of: try P256.KeyAgreement.PrivateKey(rawRepresentation: key.rawRepresentation)), pub)
    // 交给主机的 base64url 不带填充，能解回去。
    XCTAssertFalse(pub.base64URL.contains("="))
    XCTAssertEqual(Data(base64URL: pub.base64URL), pub)
    XCTAssertEqual(Data(base64URL: auth.base64URL), auth)
    // APNs 的设备令牌交给中转时是小写十六进制。
    XCTAssertEqual(Data([0x00, 0x0b, 0xab, 0xff]).hexLower, "000babff")
  }

  func testAlertIdMatchesTheDartSide() {
    // notifications.dart 的 alertId：同一个标签各端算出同一个数。期望值和 notify_test.dart、WebPushCryptoTest.kt 一样。
    XCTAssertEqual(WebPushCrypto.alertId(""), 18_652_613)
    XCTAssertEqual(WebPushCrypto.alertId("approval:a1"), 518_277_434)
    XCTAssertEqual(WebPushCrypto.alertId("thread:对话"), 157_519_743)
  }
}

// 解开以后怎么显示（Shared/PushNotice.swift）。UserNotifications 只有苹果的平台有，Linux 上跑上面那些时跳过。
#if canImport(UserNotifications)
  import UserNotifications

  final class PushNoticeTests: XCTestCase {
    private let message = PushMessage(title: "审批", body: "要付 3 元", tag: "approval:a1", threadId: "t1", silent: false)

    func testRewritesTitleBodyAndWhereToGo() {
      let loud = PushNotice.content(for: message, quiet: false)
      XCTAssertEqual(loud.title, "审批")
      XCTAssertEqual(loud.body, "要付 3 元")
      XCTAssertEqual(loud.threadIdentifier, "t1")
      XCTAssertNotNil(loud.sound)
      XCTAssertEqual(loud.interruptionLevel, .active)
      XCTAssertTrue(PushNotice.isPush(loud.userInfo))
      XCTAssertEqual(PushNotice.thread(of: loud.userInfo), "t1")
      // 同一件事已经提醒过、或者“已在别处处理”：不响铃、不亮屏。
      let quiet = PushNotice.content(for: message, quiet: true)
      XCTAssertNil(quiet.sound)
      XCTAssertEqual(quiet.interruptionLevel, .passive)
      // 没有对话的去“待办”。
      let noThread = PushMessage(title: "x", body: "", tag: "task:1", threadId: nil, silent: false)
      XCTAssertEqual(PushNotice.thread(of: PushNotice.content(for: noThread, quiet: false).userInfo), "")
    }

    func testRecognisesTheSameEventAndWhoseNotificationItIs() {
      // App 自己弹的（F-16）：flutter_local_notifications 用 alertId 的十进制当通知 id。
      let local = UNNotificationRequest(
        identifier: String(WebPushCrypto.alertId("approval:a1")), content: UNNotificationContent(), trigger: nil)
      XCTAssertTrue(PushNotice.sameEvent(local, tag: "approval:a1"))
      XCTAssertFalse(PushNotice.sameEvent(local, tag: "approval:a2"))
      // 之前的锁屏推送：id 是中转给的 apns-collapse-id，标签记在 userInfo 里。
      let pushed = UNNotificationRequest(
        identifier: "abcDEF_123", content: PushNotice.content(for: message, quiet: false), trigger: nil)
      XCTAssertTrue(PushNotice.sameEvent(pushed, tag: "approval:a1"))
      XCTAssertFalse(PushNotice.sameEvent(pushed, tag: "approval:a2"))
      // flutter_local_notifications 的通知交给插件。
      XCTAssertFalse(PushNotice.isPush(["NOTIFICATION_ID": 1, "payload": "t1"]))
      // 解不开时显示的兜底还带着密文，点开去“待办”。
      XCTAssertTrue(PushNotice.isPush(["ip": "c2VhbGVk", "aps": ["mutable-content": 1]]))
      XCTAssertEqual(PushNotice.thread(of: ["ip": "c2VhbGVk"]), "")
    }
  }
#endif
