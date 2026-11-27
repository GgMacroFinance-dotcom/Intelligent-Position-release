// 桌面外壳的颜色、圆角、动效（设计重构第一步：照 dusk bot 的外层框架和质感）。
// 只参考了 dusk bot 的设计规则和数值（它是 PolyForm Noncommercial，代码、图标、头像都没拷）。
// 规则：不用边框和投影分层，靠底色深浅；一套圆角；动效只用一条曲线。

import 'package:flutter/material.dart';

/// 一套颜色。浅色和深色各一份，挂在 ThemeData.extensions 上，用 `Shell.of(context)` 取。
@immutable
class Shell extends ThemeExtension<Shell> {
  const Shell({
    required this.app,
    required this.panel,
    required this.raised,
    required this.raisedHover,
    required this.card,
    required this.inset,
    required this.control,
    required this.edge,
    required this.hairline,
    required this.ink,
    required this.inkSecondary,
    required this.inkTertiary,
    required this.accent,
    required this.accentText,
    required this.composer,
    required this.composerRing,
    required this.menu,
    required this.menuHover,
    required this.bubbleUser,
    required this.bubbleUserInk,
    required this.success,
    required this.danger,
    required this.warning,
    required this.notice,
    required this.scrim,
    required this.floatShadow,
    required this.popoverShadow,
  });

  /// 对话区的底。
  final Color app;

  /// 左栏、右栏的底。
  final Color panel;

  /// 左栏选中的一行。
  final Color raised;
  final Color raisedHover;

  /// 员工的气泡、卡片、表格。
  final Color card;
  final Color inset;
  final Color control;

  /// 极淡的分隔（只在必须分开的地方用）。
  final Color edge;
  final Color hairline;

  final Color ink;
  final Color inkSecondary;
  final Color inkTertiary;
  final Color accent;
  final Color accentText;

  /// 输入框、菜单、浮着的胶囊。
  final Color composer;
  final Color composerRing;
  final Color menu;
  final Color menuHover;

  /// 人写的消息。
  final Color bubbleUser;
  final Color bubbleUserInk;

  final Color success;
  final Color danger;
  final Color warning;

  /// 输入框上方的提示条、出错条的底。
  final Color notice;
  final Color scrim;

  final List<BoxShadow> floatShadow;
  final List<BoxShadow> popoverShadow;

  /// 当前主题的一套颜色。星图页、旧页面套了自己的主题、没挂这套时，按深浅取默认的一套。
  static Shell of(BuildContext context) {
    final theme = Theme.of(context);
    return theme.extension<Shell>() ?? (theme.brightness == Brightness.dark ? dark : light);
  }

  static const light = Shell(
    app: Color(0xFFFDFDFD),
    panel: Color(0xFFF7F7F7),
    raised: Color(0xFFE4E4E4),
    raisedHover: Color(0xFFEDEDED),
    card: Color(0xFFEFEFEF),
    inset: Color(0xFFE9E9E9),
    control: Color(0xFFE2E2E2),
    edge: Color(0x141C1C1C),
    hairline: Color(0x3D1C1C1C),
    ink: Color(0xFF1C1C1C),
    inkSecondary: Color(0xAD1C1C1C),
    inkTertiary: Color(0xA61C1C1C),
    accent: Color(0xFF126FD4),
    accentText: Color(0xFF0B69CC),
    composer: Color(0xFFFFFFFF),
    composerRing: Color(0x1F1C1C1C),
    menu: Color(0xFFFFFFFF),
    menuHover: Color(0x0F1C1C1C),
    bubbleUser: Color(0xFF1C1C1C),
    bubbleUserInk: Color(0xFFFCFCFC),
    success: Color(0xFF007A44),
    danger: Color(0xFFCD2834),
    warning: Color(0xFF9C5B00),
    notice: Color(0x0F1C1C1C),
    scrim: Color(0x331C1C1C),
    floatShadow: [
      BoxShadow(color: Color(0x141C1C1C), offset: Offset(0, 4), blurRadius: 12, spreadRadius: -1),
      BoxShadow(color: Color(0x0F1C1C1C), offset: Offset(0, 2), blurRadius: 4, spreadRadius: -2),
    ],
    popoverShadow: [
      BoxShadow(color: Color(0x1F1C1C1C), offset: Offset(0, 10), blurRadius: 28, spreadRadius: -12),
      BoxShadow(color: Color(0x141C1C1C), offset: Offset(0, 2), blurRadius: 6, spreadRadius: -2),
    ],
  );

  static const dark = Shell(
    app: Color(0xFF121212),
    panel: Color(0xFF0E0E0E),
    raised: Color(0xFF202020),
    raisedHover: Color(0xFF191919),
    card: Color(0xFF1F1F1F),
    inset: Color(0xFF1B1B1B),
    control: Color(0xFF282828),
    edge: Color(0x0FF5F5F5),
    hairline: Color(0x38F5F5F5),
    ink: Color(0xFFF5F5F5),
    inkSecondary: Color(0xA8F5F5F5),
    inkTertiary: Color(0x8AF5F5F5),
    accent: Color(0xFF116DD2),
    accentText: Color(0xFF4D9BFF),
    composer: Color(0xFF1F1F1F),
    composerRing: Color(0x1AF5F5F5),
    menu: Color(0xFF1F1F1F),
    menuHover: Color(0x0FF5F5F5),
    bubbleUser: Color(0xFF3B3B3B),
    bubbleUserInk: Color(0xFFF5F5F5),
    success: Color(0xFF00BA69),
    danger: Color(0xFFFF6463),
    warning: Color(0xFFE28600),
    notice: Color(0x0FF5F5F5),
    scrim: Color(0x80000000),
    floatShadow: [],
    popoverShadow: [BoxShadow(color: Color(0x66000000), offset: Offset(0, 10), blurRadius: 28, spreadRadius: -12)],
  );

  /// 桌面浮空窗口的 Apple Liquid Glass 清透配色（2026-11-26 维护者定）：
  /// 中性清透白玻璃 + 深曜石墨字 + 系统蓝点缀，让背后文艺复兴蓝天和金色弧光自然透色。
  static const glass = Shell(
    app: Color(0xFFF6F8FC),
    panel: Color(0x00FFFFFF),
    raised: Color(0xCCFFFFFF),
    raisedHover: Color(0x66FFFFFF),
    card: Color(0xD9F0F3F8),
    inset: Color(0x1314171F),
    control: Color(0x1F14171F),
    edge: Color(0x1214171F),
    hairline: Color(0x2914171F),
    ink: Color(0xFF14171F),
    inkSecondary: Color(0x9414171F),
    inkTertiary: Color(0x7314171F),
    accent: Color(0xFF007AFF),
    accentText: Color(0xFF0066D6),
    composer: Color(0xF5FFFFFF),
    composerRing: Color(0x1814171F),
    menu: Color(0xFAFFFFFF),
    menuHover: Color(0x0D14171F),
    bubbleUser: Color(0xFF15181F),
    bubbleUserInk: Color(0xFFF8FAFC),
    success: Color(0xFF1A9E38),
    danger: Color(0xFFE0312B),
    warning: Color(0xFFD97706),
    notice: Color(0x0D14171F),
    scrim: Color(0x2E0A1432),
    floatShadow: [BoxShadow(color: Color(0x140E183A), offset: Offset(0, 3), blurRadius: 12, spreadRadius: -2)],
    popoverShadow: [BoxShadow(color: Color(0x260E183A), offset: Offset(0, 12), blurRadius: 32, spreadRadius: -10)],
  );

  /// 兼容别名：暖象牙旧名已统一升级到清透 Liquid Glass。
  static const ivory = glass;

  @override
  Shell copyWith() => this;

  @override
  Shell lerp(ThemeExtension<Shell>? other, double t) => t < 0.5 || other is! Shell ? this : other;
}

/// 圆角：一套，从小到大。
abstract final class Radii {
  static const xs = 2.0, sm = 4.0, md = 6.0, lg = 8.0, xl = 12.0, x2 = 14.0, x3 = 16.0, x4 = 18.0;

  /// 菜单、弹出层。
  static const menu = 14.0;

  /// 气泡。
  static const bubble = 18.0;

  /// 提问和审批卡片、输入框。
  static const card = 24.0;
}

/// 动效：只用一条曲线（快出慢停）。
abstract final class Motion {
  static const curve = Cubic(0.22, 1, 0.36, 1);
  static const fast = Duration(milliseconds: 120);
  static const msg = Duration(milliseconds: 180);
  static const dialog = Duration(milliseconds: 240);
  static const scrim = Duration(milliseconds: 160);
}

/// 尺寸。
abstract final class Sizes {
  /// 宽窗口换成新外壳的最小宽度。测试里可以调大，好在宽窗口里测原来的页面。
  static double shellMin = 980.0;
  static const sidebar = 288.0;
  static const sidebarMin = 240.0;
  static const sidebarMax = 420.0;
  static const sidebarRail = 80.0;
  static const panel = 340.0;

  /// 对话栏最宽 60rem。
  static const chatColumn = 960.0;

  /// 气泡最宽 35rem。
  static const bubbleMax = 560.0;
  static const cardMax = 560.0;
}

/// 浅色、深色两套主题。字体先用系统的（macOS 苹方、Windows 微软雅黑），字重不加粗。
ThemeData shellTheme(Brightness brightness) =>
    _themeFor(brightness == Brightness.light ? Shell.light : Shell.dark, brightness);

/// 桌面浮空窗口与左上角小组件共用的 Liquid Glass 主题（深色模式下也保持清透玻璃质感）。
ThemeData glassTheme() => _themeFor(Shell.glass, Brightness.light);

/// 兼容旧名：统一指向 Liquid Glass 主题。
ThemeData ivoryTheme() => glassTheme();

ThemeData _themeFor(Shell s, Brightness brightness) {
  final base = ThemeData(
    brightness: brightness,
    useMaterial3: true,
    colorScheme: ColorScheme.fromSeed(
      seedColor: s.accent,
      brightness: brightness,
      surface: s.app,
      primary: s.accent,
      onSurface: s.ink,
      error: s.danger,
    ),
    scaffoldBackgroundColor: s.app,
    splashFactory: NoSplash.splashFactory,
    hoverColor: s.menuHover,
    highlightColor: Colors.transparent,
    dividerColor: s.edge,
    extensions: [s],
  );
  final text = base.textTheme.apply(bodyColor: s.ink, displayColor: s.ink);
  return base.copyWith(
    textTheme: text,
    hintColor: s.inkTertiary,
    dividerTheme: DividerThemeData(color: s.edge, thickness: 1, space: 1),
    cardTheme: CardThemeData(
      color: s.card,
      elevation: 0,
      margin: EdgeInsets.zero,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(Radii.x3)),
    ),
    dialogTheme: DialogThemeData(
      backgroundColor: s.menu,
      elevation: 0,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(Radii.x3)),
    ),
    popupMenuTheme: PopupMenuThemeData(
      color: s.menu,
      elevation: 6,
      shadowColor: const Color(0x331C1C1C),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(Radii.menu)),
    ),
    tooltipTheme: TooltipThemeData(
      decoration: BoxDecoration(color: s.bubbleUser, borderRadius: BorderRadius.circular(Radii.md)),
      textStyle: TextStyle(color: s.bubbleUserInk, fontSize: 12),
      waitDuration: const Duration(milliseconds: 500),
    ),
    snackBarTheme: SnackBarThemeData(
      backgroundColor: s.bubbleUser,
      contentTextStyle: TextStyle(color: s.bubbleUserInk),
      behavior: SnackBarBehavior.floating,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(Radii.xl)),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        backgroundColor: s.ink,
        foregroundColor: s.app,
        shape: const StadiumBorder(),
        elevation: 0,
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        foregroundColor: s.ink,
        side: BorderSide(color: s.composerRing),
        shape: const StadiumBorder(),
      ),
    ),
    textButtonTheme: TextButtonThemeData(style: TextButton.styleFrom(foregroundColor: s.accentText)),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: s.inset,
      isDense: true,
      contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      border: OutlineInputBorder(borderRadius: BorderRadius.circular(Radii.xl), borderSide: BorderSide.none),
      enabledBorder: OutlineInputBorder(borderRadius: BorderRadius.circular(Radii.xl), borderSide: BorderSide.none),
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(Radii.xl),
        borderSide: BorderSide(color: s.hairline),
      ),
      hintStyle: TextStyle(color: s.inkTertiary),
    ),
    switchTheme: SwitchThemeData(
      thumbColor: WidgetStateProperty.all(Colors.white),
      trackColor: WidgetStateProperty.resolveWith((st) => st.contains(WidgetState.selected) ? s.accent : s.control),
      trackOutlineColor: WidgetStateProperty.all(Colors.transparent),
    ),
    listTileTheme: ListTileThemeData(
      iconColor: s.inkSecondary,
      textColor: s.ink,
      selectedColor: s.ink,
      selectedTileColor: s.raised,
    ),
    iconTheme: IconThemeData(color: s.inkSecondary, size: 18),
    scrollbarTheme: ScrollbarThemeData(
      thickness: WidgetStateProperty.all(6),
      radius: const Radius.circular(3),
      thumbColor: WidgetStateProperty.all(s.hairline),
    ),
  );
}
