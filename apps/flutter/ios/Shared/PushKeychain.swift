// 锁屏推送（m2-spec D-35，backlog F-21）的密钥：这台手机的 P-256 私钥和 16 字节的 auth。
// 存在钥匙串里 App 和通知扩展共用的分组（两边 .entitlements 的 keychain-access-groups），扩展才读得到；
// 手机解锁过一次以后锁屏时也能读（扩展是锁屏时被叫起的）；只留在这台设备上，不进备份、不跟到新手机。
// 不上传：交给主机的只有公钥和 auth。

import CryptoKit
import Foundation
import Security

enum PushKeychain {
  /// 共用分组去掉 Team ID 前缀的部分，和 Runner.entitlements、NotificationService.entitlements 里写的一样。
  static let groupSuffix = "dev.intelligentposition.ipApp.push"
  private static let service = "dev.intelligentposition.ipApp.push"
  private static let account = "webpush.v1"

  struct Keys {
    let privateKey: P256.KeyAgreement.PrivateKey
    let auth: Data
    var publicKey: Data { WebPushCrypto.publicKey(of: privateKey) }
  }

  struct Failure: Error, CustomStringConvertible {
    let status: OSStatus
    var description: String { "钥匙串出错（\(status)）" }
  }

  private static var item: [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: account,
    ]
  }

  /// 读出密钥。不指定分组：App 能看到自己的和共用的，扩展只看到共用的，按服务名和账号只会找到这一条。
  /// 没有时返回 nil；手机重启后还没解锁过时读不到，会抛错。
  static func load() throws -> Keys? {
    var query = item
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var out: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &out)
    if status == errSecItemNotFound { return nil }
    guard status == errSecSuccess, let data = out as? Data else { throw Failure(status: status) }
    // 32 字节的私钥，接着 16 字节的 auth。长度不对的当没有，重新生成。
    guard data.count == 48, let key = try? P256.KeyAgreement.PrivateKey(rawRepresentation: data.prefix(32)) else {
      return nil
    }
    return Keys(privateKey: key, auth: Data(data.suffix(16)))
  }

  /// 读出密钥，没有就生成一套存进共用分组。只在 App 里调（扩展只读）。
  static func loadOrCreate() throws -> Keys {
    if let keys = try load() { return keys }
    guard let group = accessGroup() else { throw Failure(status: errSecMissingEntitlement) }
    delete()
    let (key, auth) = WebPushCrypto.generate()
    var add = item
    add[kSecAttrAccessGroup as String] = group
    add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    add[kSecValueData as String] = key.rawRepresentation + auth
    let status = SecItemAdd(add as CFDictionary, nil)
    guard status == errSecSuccess else { throw Failure(status: status) }
    return Keys(privateKey: key, auth: auth)
  }

  /// 删掉密钥：之后发来的推送都解不开。
  static func delete() {
    SecItemDelete(item as CFDictionary)
  }

  /// 这个构建能不能用共用分组：签名时带上了 keychain-access-groups 权利（和 aps-environment 在同一个文件里）。
  static func usable() -> Bool {
    guard let group = accessGroup() else { return false }
    var query = item
    query[kSecAttrAccessGroup as String] = group
    let status = SecItemCopyMatching(query as CFDictionary, nil)
    return status == errSecSuccess || status == errSecItemNotFound
  }

  /// 带 Team ID 的共用分组名，比如 `ABCDE12345.dev.intelligentposition.ipApp.push`。
  /// Team ID 从钥匙串里读：在默认分组放一条空的探测项，看它落在哪个分组。签名时才有 Team ID，没签名的构建返回 nil。
  static func accessGroup() -> String? {
    if let cached = cachedGroup { return cached }
    let probe: [String: Any] = [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service + ".probe",
      kSecAttrAccount as String: "team",
    ]
    var query = probe
    query[kSecReturnAttributes as String] = true
    var out: CFTypeRef?
    var status = SecItemCopyMatching(query as CFDictionary, &out)
    if status == errSecItemNotFound {
      var add = probe
      add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
      add[kSecReturnAttributes as String] = true
      status = SecItemAdd(add as CFDictionary, &out)
    }
    guard status == errSecSuccess, let attrs = out as? [String: Any],
      let found = attrs[kSecAttrAccessGroup as String] as? String,
      let dot = found.firstIndex(of: "."), isTeamId(found[..<dot])
    else { return nil }
    let group = String(found[...dot]) + groupSuffix
    cachedGroup = group
    return group
  }

  private static var cachedGroup: String?

  /// Team ID 是 10 位大写字母和数字。没签名的构建（比如模拟器上不带团队）分组前面没有它。
  private static func isTeamId(_ s: Substring) -> Bool {
    s.count == 10 && s.allSatisfy { ("A"..."Z").contains($0) || ("0"..."9").contains($0) }
  }
}
