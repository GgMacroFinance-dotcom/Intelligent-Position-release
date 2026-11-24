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
ThemeData shellTheme(Brightness brightness) {
  final s = brightness == Brightness.light ? Shell.light : Shell.dark;
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
    listTileTheme: ListTileThemeData(iconColor: s.inkSecondary, textColor: s.ink),
    iconTheme: IconThemeData(color: s.inkSecondary, size: 18),
    scrollbarTheme: ScrollbarThemeData(
      thickness: WidgetStateProperty.all(6),
      radius: const Radius.circular(3),
      thumbColor: WidgetStateProperty.all(s.hairline),
    ),
  );
}
