// 锁屏推送（m2-spec D-35，backlog F-21）的解密：RFC 8291 的 Web Push（`aes128gcm`）。主机按 D-33 加密，
// 推送中转经 APNs 原样转来，通知扩展用这台手机自己的私钥解开。App 和通知扩展都编进这个文件。
// 逻辑和 Android 的 WebPushCrypto.kt 一样，测试用同一组 RFC 8291 附录 A 的数（RunnerTests/WebPushCryptoTests.swift）。
// 只用 CryptoKit 和 Foundation；Linux 上用接口相同的 swift-crypto 也能编、能跑测试。

#if canImport(CryptoKit)
  import CryptoKit
#else
  import Crypto
#endif
import Foundation

enum WebPushError: Error, Equatable {
  case malformed(String)
}

enum WebPushCrypto {
  /// 新的一套密钥：P-256 私钥和 16 字节随机的 auth。
  static func generate() -> (P256.KeyAgreement.PrivateKey, Data) {
    var auth = SystemRandomNumberGenerator()
    let bytes = (0..<16).map { _ in UInt8.random(in: .min ... .max, using: &auth) }
    return (P256.KeyAgreement.PrivateKey(), Data(bytes))
  }

  /// 交给主机的公钥：65 字节未压缩的点（0x04 | X | Y）。
  static func publicKey(of key: P256.KeyAgreement.PrivateKey) -> Data {
    key.publicKey.x963Representation
  }

  /// 解开一条推送。`body` 是 `aes128gcm` 的整段：盐（16）、记录大小（4）、密钥 id 长度（1）、
  /// 发送方的临时公钥（65）、密文。只认一条记录（主机发的都放得进一条）。
  static func decrypt(_ body: Data, privateKey: P256.KeyAgreement.PrivateKey, auth: Data) throws -> Data {
    let b = [UInt8](body)
    guard b.count > 21 else { throw WebPushError.malformed("推送太短") }
    let salt = Data(b[0..<16])
    let idLen = Int(b[20])
    guard idLen == 65, b.count > 21 + idLen + 16 else { throw WebPushError.malformed("推送的格式不对") }
    let asPublic = Data(b[21..<(21 + idLen)])
    let ciphertext = Data(b[(21 + idLen)...])

    let shared = try privateKey.sharedSecretFromKeyAgreement(
      with: P256.KeyAgreement.PublicKey(x963Representation: asPublic))
    let keyInfo = Data("WebPush: info".utf8) + [0] + publicKey(of: privateKey) + asPublic
    let ikm = shared.hkdfDerivedSymmetricKey(
      using: SHA256.self, salt: auth, sharedInfo: keyInfo, outputByteCount: 32)
    let cek = HKDF<SHA256>.deriveKey(
      inputKeyMaterial: ikm, salt: salt, info: Data("Content-Encoding: aes128gcm".utf8) + [0],
      outputByteCount: 16)
    let nonce = HKDF<SHA256>.deriveKey(
      inputKeyMaterial: ikm, salt: salt, info: Data("Content-Encoding: nonce".utf8) + [0],
      outputByteCount: 12)

    let box = try AES.GCM.SealedBox(
      nonce: AES.GCM.Nonce(data: nonce.withUnsafeBytes { Data($0) }),
      ciphertext: ciphertext.prefix(ciphertext.count - 16),
      tag: ciphertext.suffix(16))
    let padded = [UInt8](try AES.GCM.open(box, using: cek))
    // 去掉填充：最后一个非零字节是分隔符，最后一条记录是 0x02。
    guard let end = padded.lastIndex(where: { $0 != 0 }), padded[end] == 2 else {
      throw WebPushError.malformed("推送的填充不对")
    }
    return Data(padded[..<end])
  }

  /// 中转放在 APNs 通知 `ip` 字段里的密文（标准 base64，带填充）解开成主机推来的内容。解不开、格式不对时为 nil。
  static func open(sealed: String, privateKey: P256.KeyAgreement.PrivateKey, auth: Data) -> PushMessage? {
    guard let body = Data(base64Encoded: sealed),
      let plain = try? decrypt(body, privateKey: privateKey, auth: auth)
    else { return nil }
    return PushMessage.parse(plain)
  }

  /// 和 Dart 那边（notifications.dart）的 `alertId` 一样：标签的 UTF-16 码元做 FNV-1a，取 31 位。
  /// App 自己弹的通知（F-16）用它的十进制当通知的 id。
  static func alertId(_ tag: String) -> Int {
    var h: UInt32 = 0x811c_9dc5
    for c in tag.utf16 {
      h ^= UInt32(c)
      h = h &* 0x0100_0193
    }
    return Int(h & 0x7fff_ffff)
  }
}

/// 主机推来的内容（rust/crates/ip-server/src/webpush.rs 的 `Message`）。
struct PushMessage: Decodable, Equatable {
  let title: String
  let body: String
  let tag: String
  let threadId: String?
  /// “已在别处处理”：撤掉同一件事的旧通知，不再响铃。
  let silent: Bool

  private enum CodingKeys: String, CodingKey {
    case title, body, tag, silent
    case threadId = "thread_id"
  }

  init(title: String, body: String, tag: String, threadId: String?, silent: Bool) {
    self.title = title
    self.body = body
    self.tag = tag
    self.threadId = threadId
    self.silent = silent
  }

  init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
    body = try c.decodeIfPresent(String.self, forKey: .body) ?? ""
    tag = try c.decode(String.self, forKey: .tag)
    threadId = try c.decodeIfPresent(String.self, forKey: .threadId)
    silent = try c.decodeIfPresent(Bool.self, forKey: .silent) ?? false
  }

  /// 没有标签的不认（和 Android 一样）：主机发的都带标签，同一件事靠它替换、撤销。
  static func parse(_ data: Data) -> PushMessage? {
    guard let m = try? JSONDecoder().decode(PushMessage.self, from: data), !m.tag.isEmpty else { return nil }
    return m
  }
}

extension Data {
  /// base64url，不带填充：交给中转和主机的公钥、auth 用这个格式。
  var base64URL: String {
    base64EncodedString()
      .replacingOccurrences(of: "+", with: "-")
      .replacingOccurrences(of: "/", with: "_")
      .replacingOccurrences(of: "=", with: "")
  }

  init?(base64URL s: String) {
    var t = s.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
    t += String(repeating: "=", count: (4 - t.count % 4) % 4)
    self.init(base64Encoded: t)
  }

  /// 小写十六进制：APNs 的设备令牌交给中转时用这个格式（中转只认十六进制）。
  var hexLower: String {
    let digits = Array("0123456789abcdef".utf8)
    var out = [UInt8]()
    out.reserveCapacity(count * 2)
    for b in self {
      out.append(digits[Int(b >> 4)])
      out.append(digits[Int(b & 0x0f)])
    }
    return String(decoding: out, as: UTF8.self)
  }
}
