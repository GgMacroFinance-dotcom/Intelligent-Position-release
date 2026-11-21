// 系统通知（backlog F-16）：主机发来的 `alert` 弹成系统通知，`dismiss` 撤掉，点开去那个对话或“待办”。
// 弹不弹由 AppState 决定（App 在前台时不弹，和网页控制台一样）；这里只管怎么弹。

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';

import 'protocol/models.dart';

/// 点了一条通知：去 `threadId` 那个对话，为空时去“待办”。
typedef AlertTap = void Function(String? threadId);

/// 弹系统通知的那一层。测试里换成假的。
abstract class SystemNotifier {
  /// 准备好；App 是被点通知打开的话，马上回调一次 `onTap`。
  Future<void> init(AlertTap onTap);

  /// 向系统要通知权限（Android 13 以上、iOS、macOS 会弹框问人，别的平台直接返回 true）。
  Future<bool> requestPermission();

  /// 弹一条。同一个标签的新通知盖掉旧的。
  Future<void> show(Alert alert);

  /// 撤掉这个标签的通知。
  Future<void> cancel(String tag);
}

/// 什么都不弹（还没初始化好、或者平台不支持时用）。
class SilentNotifier implements SystemNotifier {
  const SilentNotifier();

  @override
  Future<void> init(AlertTap onTap) async {}

  @override
  Future<bool> requestPermission() async => false;

  @override
  Future<void> show(Alert alert) async {}

  @override
  Future<void> cancel(String tag) async {}
}

/// 通知的数字 id：系统按 id 认“同一条”，这里由标签算出来，同一件事的通知就会被替换、能按标签撤。
/// 用 FNV-1a 取 31 位，各平台都当正的 32 位整数用。
int alertId(String tag) {
  var h = 0x811c9dc5;
  for (final c in tag.codeUnits) {
    h ^= c;
    h = (h * 0x01000193) & 0xffffffff;
  }
  return h & 0x7fffffff;
}

/// Android 的通知渠道。要人处理的和其余的分开，人可以在系统设置里分别关。
const _channelNeedsMe = AndroidNotificationDetails(
  'needs_me',
  '要我处理的',
  channelDescription: '审批卡、员工提问、一轮失败、任务受阻',
  importance: Importance.high,
  priority: Priority.high,
);
const _channelAll = AndroidNotificationDetails('all', '员工回复和任务完成', channelDescription: '通知设置选了“全部”时才有');

/// 用 flutter_local_notifications 弹：Android、iOS、macOS、Windows、Linux 都支持。
class LocalNotifier implements SystemNotifier {
  LocalNotifier([FlutterLocalNotificationsPlugin? plugin]) : _plugin = plugin ?? FlutterLocalNotificationsPlugin();

  final FlutterLocalNotificationsPlugin _plugin;
  bool _ready = false;

  @override
  Future<void> init(AlertTap onTap) async {
    void tapped(NotificationResponse r) {
      final p = r.payload;
      onTap(p == null || p.isEmpty ? null : p);
    }

    try {
      await _plugin.initialize(
        settings: const InitializationSettings(
          android: AndroidInitializationSettings('@mipmap/ic_launcher'),
          // 权限在连上主机后再要（requestPermission），不在启动时就弹框。
          iOS: DarwinInitializationSettings(
            requestAlertPermission: false,
            requestBadgePermission: false,
            requestSoundPermission: false,
          ),
          macOS: DarwinInitializationSettings(
            requestAlertPermission: false,
            requestBadgePermission: false,
            requestSoundPermission: false,
          ),
          linux: LinuxInitializationSettings(defaultActionName: '打开'),
          windows: WindowsInitializationSettings(
            appName: 'intelligent-position',
            appUserModelId: 'IntelligentPosition.IpApp',
            // 通知激活回调的标识，固定不变；换了它，旧通知点了就打不开 App。
            guid: '6b0f3c1e-5a8d-4f2b-9c7e-2d1a4e8b7f30',
          ),
        ),
        onDidReceiveNotificationResponse: tapped,
      );
      _ready = true;
    } catch (e) {
      // 系统没有通知服务（比如 Linux 没有通知守护进程）时不弹，App 照常用。
      debugPrint('系统通知用不了：$e');
      return;
    }
    // App 是不是被点通知打开的。Linux 的插件没有这个接口（会抛 UnimplementedError），当作不是。
    try {
      final launch = await _plugin.getNotificationAppLaunchDetails();
      final r = launch?.notificationResponse;
      if (launch?.didNotificationLaunchApp == true && r != null) tapped(r);
    } catch (_) {}
  }

  @override
  Future<bool> requestPermission() async {
    if (!_ready) return false;
    try {
      switch (defaultTargetPlatform) {
        case TargetPlatform.android:
          return await _plugin
                  .resolvePlatformSpecificImplementation<AndroidFlutterLocalNotificationsPlugin>()
                  ?.requestNotificationsPermission() ??
              false;
        case TargetPlatform.iOS:
          return await _plugin
                  .resolvePlatformSpecificImplementation<IOSFlutterLocalNotificationsPlugin>()
                  ?.requestPermissions(alert: true, sound: true) ??
              false;
        case TargetPlatform.macOS:
          return await _plugin
                  .resolvePlatformSpecificImplementation<MacOSFlutterLocalNotificationsPlugin>()
                  ?.requestPermissions(alert: true, sound: true) ??
              false;
        default:
          return true;
      }
    } catch (e) {
      debugPrint('要通知权限失败：$e');
      return false;
    }
  }

  @override
  Future<void> show(Alert alert) async {
    if (!_ready) return;
    try {
      await _plugin.show(
        id: alertId(alert.tag),
        title: alert.title,
        body: alert.body.isEmpty ? null : alert.body,
        payload: alert.threadId ?? '',
        notificationDetails: NotificationDetails(
          android: alert.level == 'all' ? _channelAll : _channelNeedsMe,
          iOS: DarwinNotificationDetails(threadIdentifier: alert.threadId),
          macOS: DarwinNotificationDetails(threadIdentifier: alert.threadId),
        ),
      );
    } catch (e) {
      debugPrint('弹通知失败：$e');
    }
  }

  @override
  Future<void> cancel(String tag) async {
    if (!_ready) return;
    try {
      await _plugin.cancel(id: alertId(tag));
    } catch (e) {
      debugPrint('撤通知失败：$e');
    }
  }
}
