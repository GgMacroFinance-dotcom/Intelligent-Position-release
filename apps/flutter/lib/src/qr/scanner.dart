// 用摄像头扫二维码（F-19）。Android、iOS 用官方的 camera 插件（Android 走 CameraX），macOS 用 camera_macos，
// 画面都转成灰度图，再用 zxing2 在后台 isolate 里解。Windows、Linux、网页没有相机扫码，配对页不显示“扫码”。

import 'dart:async';
import 'dart:io' show Platform;

import 'package:camera/camera.dart';
import 'package:camera_macos/camera_macos.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import 'decode.dart';

/// 一种扫码方式。界面测试换成假的。
abstract interface class QrScanner {
  /// 相机画面。每解出一个二维码调一次 `onText`（同一个码会连着来好几次）；相机打不开时调 `onError`。
  Widget view({required ValueChanged<String> onText, required ValueChanged<String> onError});
}

/// 这个平台能用的扫码方式；不能扫码时返回 null。
QrScanner? platformQrScanner() {
  if (kIsWeb) return null;
  if (Platform.isAndroid || Platform.isIOS) return const CameraQrScanner();
  if (Platform.isMacOS) return const MacQrScanner();
  return null;
}

/// 相机报错时给人看的话。
String cameraErrorText(Object e) {
  final code = switch (e) {
    CameraException(:final code) => code,
    CameraMacOSException(:final code) => code,
    _ => '',
  };
  // camera 插件给 CameraAccessDenied 之类的代码；camera_macos 只在说明里写 Permission not granted。
  if (code.contains('AccessDenied') || code.contains('AccessRestricted') || '$e'.contains('Permission not granted')) {
    return '没有相机权限。到系统设置里给 intelligent-position 打开相机，再回来扫。';
  }
  return '打不开相机：$e';
}

/// 解码节流：同一时间只解一张，解完再收下一张，其余的画面直接丢掉。
class _Throttle {
  bool _busy = false;
  bool closed = false;

  void offer(LumaFrame Function() frame, ValueChanged<String> onText) {
    if (_busy || closed) return;
    _busy = true;
    unawaited(
      decodeQrInBackground(frame())
          .then((text) {
            if (text != null && !closed) onText(text);
          })
          .whenComplete(() => _busy = false),
    );
  }
}

// ---------- Android、iOS ----------

class CameraQrScanner implements QrScanner {
  const CameraQrScanner();

  @override
  Widget view({required ValueChanged<String> onText, required ValueChanged<String> onError}) =>
      _CameraView(onText: onText, onError: onError);
}

class _CameraView extends StatefulWidget {
  const _CameraView({required this.onText, required this.onError});

  final ValueChanged<String> onText;
  final ValueChanged<String> onError;

  @override
  State<_CameraView> createState() => _CameraViewState();
}

class _CameraViewState extends State<_CameraView> with WidgetsBindingObserver {
  CameraController? _controller;
  _Throttle _throttle = _Throttle();

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    unawaited(_start());
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    unawaited(_stop());
    super.dispose();
  }

  /// 切到后台时放开相机，回来再打开（camera 插件的要求）。
  @override
  void didChangeAppLifecycleState(AppLifecycleState s) {
    if (s == AppLifecycleState.inactive || s == AppLifecycleState.paused) {
      unawaited(_stop());
    } else if (s == AppLifecycleState.resumed && _controller == null) {
      unawaited(_start());
    }
  }

  Future<void> _start() async {
    try {
      final cams = await availableCameras();
      if (cams.isEmpty) {
        widget.onError('这台设备上找不到相机。');
        return;
      }
      final cam = cams.firstWhere((c) => c.lensDirection == CameraLensDirection.back, orElse: () => cams.first);
      final c = CameraController(
        cam,
        ResolutionPreset.medium,
        enableAudio: false,
        // 两边都要 YUV：第一面就是亮度，不用转换。
        imageFormatGroup: ImageFormatGroup.yuv420,
      );
      await c.initialize();
      if (!mounted) {
        await c.dispose();
        return;
      }
      final throttle = _throttle = _Throttle();
      await c.startImageStream((img) {
        final y = img.planes.first;
        throttle.offer(
          () => LumaFrame.yPlane(Uint8List.fromList(y.bytes), img.width, img.height, y.bytesPerRow),
          widget.onText,
        );
      });
      setState(() => _controller = c);
    } catch (e) {
      if (mounted) widget.onError(cameraErrorText(e));
    }
  }

  Future<void> _stop() async {
    _throttle.closed = true;
    final c = _controller;
    _controller = null;
    if (mounted) setState(() {});
    if (c == null) return;
    try {
      if (c.value.isStreamingImages) await c.stopImageStream();
    } catch (_) {
      // 关相机时出的错不用管。
    }
    await c.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = _controller;
    if (c == null || !c.value.isInitialized) return const Center(child: CircularProgressIndicator());
    return ClipRect(
      child: FittedBox(
        fit: BoxFit.cover,
        child: SizedBox(
          // 预览的宽高是横着说的，竖屏时对调。
          width: c.value.previewSize?.height ?? 1,
          height: c.value.previewSize?.width ?? 1,
          child: CameraPreview(c),
        ),
      ),
    );
  }
}

// ---------- macOS ----------

class MacQrScanner implements QrScanner {
  const MacQrScanner();

  @override
  Widget view({required ValueChanged<String> onText, required ValueChanged<String> onError}) =>
      _MacView(onText: onText, onError: onError);
}

class _MacView extends StatefulWidget {
  const _MacView({required this.onText, required this.onError});

  final ValueChanged<String> onText;
  final ValueChanged<String> onError;

  @override
  State<_MacView> createState() => _MacViewState();
}

class _MacViewState extends State<_MacView> {
  CameraMacOSController? _controller;
  final _throttle = _Throttle();
  bool _reported = false;

  @override
  void dispose() {
    _throttle.closed = true;
    final c = _controller;
    if (c != null) {
      unawaited(c.stopImageStream().then((_) => c.destroy()).catchError((Object _) => null));
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => CameraMacOSView(
    cameraMode: CameraMacOSMode.photo,
    enableAudio: false,
    resolution: PictureResolution.medium,
    fit: BoxFit.cover,
    onCameraLoading: (error) {
      if (error != null && !_reported) {
        _reported = true;
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (mounted) widget.onError(cameraErrorText(error));
        });
      }
      return const Center(child: CircularProgressIndicator());
    },
    onCameraInizialized: (c) {
      _controller = c;
      unawaited(
        c.startImageStream((img) {
          if (img == null) return;
          _throttle.offer(() => LumaFrame.from32bpp(img.bytes, img.width, img.height, img.bytesPerRow), widget.onText);
        }),
      );
    },
  );
}
