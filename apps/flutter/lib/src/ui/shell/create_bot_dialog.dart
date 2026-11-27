// 宽窗口居中浮起的「新建 Bot」极简深暮蓝 Liquid Glass 弹窗（参照 duskbot 的克制构图，方案 B + 甲，2026-11-27）：
//   - 整张 440px 弹窗采用「文艺复兴暗金夜雾 + 随选中星球轻染色」液态流光底幕（shaders/modal_veil.frag），
//     直接复用天穹共享 CelestialClock（不额外开定时器），彻底去掉每颗星球背后多余的圆形底托与角落小点：
//       1) 极简顶栏（标题“新建 Bot” + 右上角透明圆 ×）；
//       2) 居中大号实机 GPU 星球裸浮预览（下方仅标一句天体本名，零花哨定语）；
//       3) 居中无厚边框的名字输入框（回车即可直接创建）；
//       4) 居中两排（5 + 4）纯裸浮实机星球选择器（严格排除太阳，允许重复选用；未选零边框，选中仅出 1.25px 解析抗锯齿淡白细环）。
//   - 「岗位与高级设置」（已有/新建岗位、模型、日预算、沙箱工具、工作目录，对齐 ip.yaml）默认收起，
//     挂在底部左侧极轻文字开关处，点开才平滑展开，平时绝不打扰第一屏纯净感。
//   - 后端协议空缺详细文档与提案字段集中在 local_bot.dart。

import 'dart:async' show unawaited;
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import '../../protocol/client.dart' show RequestError;
import '../../protocol/models.dart';
import '../../star_map.dart';
import '../common.dart';
import 'agent_sheet.dart' show liquidSaturateMatrix;
import 'celestial_body.dart';
import 'cosmos.dart' show CosmosBackdrop;
import 'local_bot.dart';
import 'tokens.dart';

enum _RoleMode { existing, createNew }

/// 居中浮起的极简深暮蓝「新建 Bot」macOS Liquid Glass 弹窗卡片。
class CreateBotModal extends StatefulWidget {
  const CreateBotModal({
    super.key,
    required this.clock,
    required this.usedCelestialCounts,
    required this.onClose,
    required this.onCreated,
  });

  /// 与桌面天穹共享的单一轻量时钟（弹窗打开期间驱动大星球与 9 颗缩略星球的实时着色器）。
  final CelestialClock clock;

  /// 当前天穹上各星球形态已被多少员工使用（按 [CelestialStyle] 计数，仅做轻提示，允许重复选择）。
  final Map<CelestialStyle, int> usedCelestialCounts;

  final VoidCallback onClose;
  final ValueChanged<CreatedBot> onCreated;

  @override
  State<CreateBotModal> createState() => _CreateBotModalState();
}

class _CreateBotModalState extends State<CreateBotModal> {
  final _nameCtrl = TextEditingController();
  final _roleNameCtrl = TextEditingController();
  final _roleDescCtrl = TextEditingController();
  final _modelCtrl = TextEditingController();
  final _escalateCtrl = TextEditingController();
  final _budgetCtrl = TextEditingController();
  final _workspaceCtrl = TextEditingController();
  final _scrollCtrl = ScrollController();

  late Celestial _selectedCelestial;
  int _pickBounceTick = 0;

  _RoleMode _roleMode = _RoleMode.existing;
  String? _selectedRoleId;
  bool _advancedOpen = false;
  bool _tools = true;
  bool _initializedFromState = false;
  String? _error;

  /// 正在等主机建好（建在主机上时，m2-spec D-42）。
  bool _submitting = false;

  @override
  void initState() {
    super.initState();
    // 打开弹窗时默认优先选中地球（earth）。
    _selectedCelestial = selectableBotCelestials.firstWhere(
      (c) => c.id == 'earth',
      orElse: () => selectableBotCelestials.first,
    );
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_initializedFromState) return;
    _initializedFromState = true;
    final state = AppScope.read(context);
    final roles = _selectableExistingRoles(state.allRoles);
    if (roles.isNotEmpty) {
      _roleMode = _RoleMode.existing;
      _selectedRoleId = roles.first.id;
      _modelCtrl.text = roles.first.model;
    } else {
      _roleMode = _RoleMode.createNew;
      _modelCtrl.text = state.allRoles.isNotEmpty ? state.allRoles.first.model : 'openai/gpt-6-luna';
    }
  }

  @override
  void dispose() {
    _nameCtrl.dispose();
    _roleNameCtrl.dispose();
    _roleDescCtrl.dispose();
    _modelCtrl.dispose();
    _escalateCtrl.dispose();
    _budgetCtrl.dispose();
    _workspaceCtrl.dispose();
    _scrollCtrl.dispose();
    super.dispose();
  }

  /// 过滤掉协调员/幕僚长岗位，避免新建普通行星 Bot 误挂到太阳专属岗位。
  List<Role> _selectableExistingRoles(List<Role> allRoles) {
    final explicit = allRoles.any((r) => r.coordinator);
    return [
      for (final r in allRoles)
        if (!isChiefOfStaff(
          Agent(id: '_probe', role: r.id, name: r.displayName),
          r,
          explicit: explicit,
        ))
          r,
    ];
  }

  void _pickCelestial(Celestial c) {
    if (c.kind == CelestialKind.sun) return;
    setState(() {
      _selectedCelestial = c;
      _pickBounceTick++;
    });
  }

  void _toggleAdvanced() {
    final opening = !_advancedOpen;
    setState(() => _advancedOpen = opening);
    if (opening) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted || !_scrollCtrl.hasClients) return;
        _scrollCtrl.animateTo(
          _scrollCtrl.position.maxScrollExtent,
          duration: const Duration(milliseconds: 220),
          curve: Motion.curve,
        );
      });
    }
  }

  Future<void> _submit() async {
    if (_submitting) return;
    final state = AppScope.read(context);
    final name = _nameCtrl.text.trim();
    if (name.isEmpty) {
      setState(() => _error = '请填写 Bot 名字');
      return;
    }

    final existingRoles = _selectableExistingRoles(state.allRoles);
    final creatingRole = _roleMode == _RoleMode.createNew || existingRoles.isEmpty;

    double? budget;
    final budgetText = _budgetCtrl.text.trim();
    if (budgetText.isNotEmpty) {
      final parsed = double.tryParse(budgetText);
      if (parsed == null || parsed <= 0 || parsed.isNaN) {
        setState(() => _error = '每日预算需填写大于 0 的数字（美元），如 2');
        return;
      }
      budget = parsed;
    }

    final draft = CreateBotDraft(
      name: name,
      celestial: _selectedCelestial,
      existingRoleId: creatingRole ? null : (_selectedRoleId ?? existingRoles.first.id),
      newRoleName: creatingRole ? _roleNameCtrl.text.trim() : null,
      newRoleDescription: creatingRole ? _roleDescCtrl.text.trim() : null,
      modelDefault: _modelCtrl.text.trim().isEmpty ? 'openai/gpt-6-luna' : _modelCtrl.text.trim(),
      modelEscalate: _escalateCtrl.text.trim().isEmpty ? null : _escalateCtrl.text.trim(),
      budgetPerDayUsd: budget,
      tools: _tools,
      workspace: _workspaceCtrl.text.trim().isEmpty ? null : _workspaceCtrl.text.trim(),
    );

    setState(() {
      _submitting = true;
      _error = null;
    });
    try {
      final created = await state.createBot(draft);
      if (mounted) widget.onCreated(created);
    } on RequestError catch (e) {
      if (mounted) setState(() => _error = '主机没能新建：$e');
    } on ArgumentError catch (e) {
      if (mounted) setState(() => _error = '${e.message}');
    } finally {
      if (mounted) setState(() => _submitting = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final existingRoles = _selectableExistingRoles(state.allRoles);
    final effectiveMode = existingRoles.isEmpty ? _RoleMode.createNew : _roleMode;
    final selectedStyle = CelestialStyle.fromCelestial(_selectedCelestial);
    final usedCount = widget.usedCelestialCounts[selectedStyle] ?? 0;

    final trimmedName = _nameCtrl.text.trim();
    final sameNameExists =
        trimmedName.isNotEmpty && state.allAgents.any((a) => a.name.trim().toLowerCase() == trimmedName.toLowerCase());
    final canSubmit = trimmedName.isNotEmpty && !_submitting;

    // 统一深暮蓝 Liquid Glass 色板：带冷群青暗调，既衬亮色恒星/水星/金星，又让深蓝海王星清晰立体。
    const ink = Color(0xF5F7FAFF);
    const inkSecondary = Color(0xA6D6E2FA);
    const inkTertiary = Color(0x66D6E2FA);
    const hairline = Color(0x1CFFFFFF);

    final outerRadius = BorderRadius.circular(20);
    final outerShape = RoundedSuperellipseBorder(
      borderRadius: outerRadius,
      side: const BorderSide(color: Color(0x30FFFFFF), width: 0.85),
    );
    final auraTint = _celestialAuraTint(selectedStyle);

    return LayoutBuilder(
      builder: (context, box) {
        // 严格对齐 duskbot 440px 的克制弹窗宽度，并将横向起点锁定到整像素，杜绝窗口缩放时的 .5 亚像素抖动。
        final dialogW = math.min(440.0, math.max(320.0, box.maxWidth - 32)).floorToDouble();
        final dialogMaxH = math.min(680.0, math.max(360.0, box.maxHeight - 32)).floorToDouble();
        final dialogLeft = ((box.maxWidth - dialogW) / 2).floorToDouble();
        return Stack(
          children: [
            // 轻薄深蓝半透幕布：点空白关闭。
            Positioned.fill(
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTap: widget.onClose,
                child: const ColoredBox(color: Color(0x3D071229)),
              ),
            ),
            Positioned(
              left: dialogLeft,
              top: 0,
              bottom: 0,
              width: dialogW,
              child: Center(
                child: ConstrainedBox(
                  constraints: BoxConstraints(maxWidth: dialogW, maxHeight: dialogMaxH),
                  child: DecoratedBox(
                    key: const ValueKey('create-bot-modal'),
                    decoration: ShapeDecoration(
                      shape: RoundedSuperellipseBorder(borderRadius: outerRadius),
                      shadows: const [
                        BoxShadow(color: Color(0x66050D22), offset: Offset(0, 22), blurRadius: 54, spreadRadius: -14),
                        BoxShadow(color: Color(0x2E050D22), offset: Offset(0, 4), blurRadius: 14, spreadRadius: -2),
                      ],
                    ),
                    child: ClipRSuperellipse(
                      borderRadius: outerRadius,
                      child: BackdropFilter(
                        filter: ui.ImageFilter.compose(
                          outer: const ui.ColorFilter.matrix(liquidSaturateMatrix),
                          inner: ui.ImageFilter.blur(sigmaX: 34, sigmaY: 34),
                        ),
                        child: Stack(
                          fit: StackFit.passthrough,
                          children: [
                            // 文艺复兴暗金夜雾 + 随选中星球轻染色的隐蔽液态流动底幕（复用共享时钟，零额外定时器）
                            Positioned.fill(
                              child: _ModalVeilBackdrop(clock: widget.clock, accentColor: auraTint),
                            ),
                            DecoratedBox(
                              position: DecorationPosition.foreground,
                              decoration: ShapeDecoration(shape: outerShape),
                              child: Material(
                                type: MaterialType.transparency,
                                child: Stack(
                                  children: [
                                    Column(
                                      mainAxisSize: MainAxisSize.min,
                                      crossAxisAlignment: CrossAxisAlignment.stretch,
                                      children: [
                                        // 1. 极简顶栏：只放“新建 Bot”标题（右上角关闭按钮绝对定位对齐 duskbot）。
                                        const Padding(
                                          padding: EdgeInsets.fromLTRB(18, 16, 48, 6),
                                          child: Text(
                                            '新建 Bot',
                                            style: TextStyle(
                                              fontSize: 14,
                                              fontWeight: FontWeight.w600,
                                              color: ink,
                                              letterSpacing: -0.15,
                                              height: 1.4,
                                            ),
                                          ),
                                        ),

                                        // 2. 居中极简主体：裸浮大星球 + 天体本名 + 居中起名 + 两排纯裸浮选星（+ 折叠高级区）
                                        Flexible(
                                          child: SingleChildScrollView(
                                            controller: _scrollCtrl,
                                            padding: const EdgeInsets.fromLTRB(20, 2, 20, 14),
                                            child: Column(
                                              crossAxisAlignment: CrossAxisAlignment.stretch,
                                              children: [
                                                // 2a. 居中大星球直接裸浮在深暮蓝空间里（彻底去掉圆框）
                                                Center(
                                                  child: _HeroCelestialPreview(
                                                    style: selectedStyle,
                                                    bounceTick: _pickBounceTick,
                                                    clock: widget.clock,
                                                  ),
                                                ),
                                                const SizedBox(height: 4),
                                                // 大球下方只标一句极轻本名（已占用时顺带轻提示，零花哨定语）
                                                Text(
                                                  usedCount > 0
                                                      ? '${selectedStyle.bodyName} · 在轨 $usedCount 颗'
                                                      : selectedStyle.bodyName,
                                                  key: const ValueKey('create-bot-selected-label'),
                                                  textAlign: TextAlign.center,
                                                  style: const TextStyle(
                                                    fontSize: 12,
                                                    fontWeight: FontWeight.w500,
                                                    color: inkSecondary,
                                                    height: 1.3,
                                                  ),
                                                ),
                                                const SizedBox(height: 10),

                                                // 2b. 居中无厚重边框的名字输入框（回车直接创建）
                                                TextField(
                                                  key: const ValueKey('create-bot-name-input'),
                                                  controller: _nameCtrl,
                                                  autofocus: true,
                                                  maxLength: 80,
                                                  textAlign: TextAlign.center,
                                                  cursorColor: ink,
                                                  style: const TextStyle(
                                                    fontSize: 14.5,
                                                    fontWeight: FontWeight.w500,
                                                    color: ink,
                                                    letterSpacing: 0.1,
                                                  ),
                                                  onChanged: (_) => setState(() => _error = null),
                                                  onSubmitted: (_) {
                                                    if (canSubmit) unawaited(_submit());
                                                  },
                                                  decoration: InputDecoration(
                                                    counterText: '',
                                                    isDense: true,
                                                    hintText: '给 Bot 起个名字',
                                                    hintStyle: const TextStyle(
                                                      fontSize: 13.5,
                                                      fontWeight: FontWeight.w400,
                                                      color: inkTertiary,
                                                    ),
                                                    filled: true,
                                                    fillColor: Colors.transparent,
                                                    hoverColor: const Color(0x0DFFFFFF),
                                                    contentPadding: const EdgeInsets.symmetric(
                                                      horizontal: 12,
                                                      vertical: 9,
                                                    ),
                                                    border: OutlineInputBorder(
                                                      borderRadius: BorderRadius.circular(10),
                                                      borderSide: BorderSide.none,
                                                    ),
                                                    enabledBorder: OutlineInputBorder(
                                                      borderRadius: BorderRadius.circular(10),
                                                      borderSide: BorderSide.none,
                                                    ),
                                                    focusedBorder: OutlineInputBorder(
                                                      borderRadius: BorderRadius.circular(10),
                                                      borderSide: const BorderSide(
                                                        color: Color(0x2EFFFFFF),
                                                        width: 0.8,
                                                      ),
                                                    ),
                                                  ),
                                                ),
                                                if (sameNameExists)
                                                  const Padding(
                                                    padding: EdgeInsets.only(top: 2, bottom: 2),
                                                    child: Text(
                                                      '已有同名员工，仍可继续创建',
                                                      textAlign: TextAlign.center,
                                                      style: TextStyle(fontSize: 11.5, color: inkSecondary),
                                                    ),
                                                  ),
                                                const SizedBox(height: 12),

                                                // 2c. 居中两排（5 + 4）纯裸浮实机星球（无圆底、无角点，仅选中淡白细环）
                                                _CelestialPickerRows(
                                                  selected: _selectedCelestial,
                                                  usedCounts: widget.usedCelestialCounts,
                                                  clock: widget.clock,
                                                  onSelect: _pickCelestial,
                                                ),

                                                // 2d. 可折叠「岗位与高级设置」（由底部左侧文字开关展开，平时完全隐藏）
                                                if (_advancedOpen) ...[
                                                  const SizedBox(height: 16),
                                                  const ColoredBox(color: hairline, child: SizedBox(height: 0.5)),
                                                  const SizedBox(height: 14),
                                                  _RoleAndAdvancedSection(
                                                    existingRoles: existingRoles,
                                                    roleMode: effectiveMode,
                                                    selectedRoleId: _selectedRoleId,
                                                    roleNameCtrl: _roleNameCtrl,
                                                    roleDescCtrl: _roleDescCtrl,
                                                    modelCtrl: _modelCtrl,
                                                    escalateCtrl: _escalateCtrl,
                                                    budgetCtrl: _budgetCtrl,
                                                    workspaceCtrl: _workspaceCtrl,
                                                    tools: _tools,
                                                    onRoleModeChanged: (m) => setState(() {
                                                      _roleMode = m;
                                                      _error = null;
                                                    }),
                                                    onSelectedRoleChanged: (id) => setState(() {
                                                      _selectedRoleId = id;
                                                      final match = existingRoles.where((r) => r.id == id);
                                                      if (match.isNotEmpty) _modelCtrl.text = match.first.model;
                                                    }),
                                                    onToolsChanged: (v) => setState(() => _tools = v),
                                                  ),
                                                ],

                                                if (_error != null) ...[
                                                  const SizedBox(height: 10),
                                                  Text(
                                                    _error!,
                                                    textAlign: TextAlign.center,
                                                    style: const TextStyle(
                                                      fontSize: 12,
                                                      color: Color(0xFFFF8680),
                                                      fontWeight: FontWeight.w500,
                                                    ),
                                                  ),
                                                ],
                                              ],
                                            ),
                                          ),
                                        ),

                                        // 3. 极简底栏：左“岗位与高级设置”轻文字开关，右两枚 32px 全圆角胶囊按钮（取消 / 创建）。
                                        Padding(
                                          padding: const EdgeInsets.fromLTRB(16, 10, 14, 13),
                                          child: Row(
                                            children: [
                                              InkWell(
                                                key: const ValueKey('create-bot-advanced-toggle'),
                                                onTap: _toggleAdvanced,
                                                borderRadius: BorderRadius.circular(8),
                                                hoverColor: const Color(0x14FFFFFF),
                                                child: Padding(
                                                  padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 5),
                                                  child: Row(
                                                    mainAxisSize: MainAxisSize.min,
                                                    children: [
                                                      Text(
                                                        '岗位与高级设置',
                                                        style: TextStyle(
                                                          fontSize: 12.5,
                                                          color: _advancedOpen ? ink : inkSecondary,
                                                          fontWeight: _advancedOpen ? FontWeight.w500 : FontWeight.w400,
                                                        ),
                                                      ),
                                                      const SizedBox(width: 2),
                                                      Icon(
                                                        _advancedOpen
                                                            ? Icons.keyboard_arrow_up_rounded
                                                            : Icons.keyboard_arrow_down_rounded,
                                                        size: 15,
                                                        color: _advancedOpen ? ink : inkSecondary,
                                                      ),
                                                    ],
                                                  ),
                                                ),
                                              ),
                                              const Spacer(),
                                              _PillActionButton(
                                                label: '取消',
                                                background: const Color(0x16FFFFFF),
                                                hoverBackground: const Color(0x26FFFFFF),
                                                foreground: ink,
                                                onTap: widget.onClose,
                                              ),
                                              const SizedBox(width: 8),
                                              _PillActionButton(
                                                key: const ValueKey('create-bot-submit'),
                                                label: '创建',
                                                background: canSubmit
                                                    ? const Color(0xFFF6F9FF)
                                                    : const Color(0x1AFFFFFF),
                                                hoverBackground: const Color(0xE0FFFFFF),
                                                foreground: canSubmit
                                                    ? const Color(0xFF101829)
                                                    : const Color(0x4DD6E2FA),
                                                onTap: canSubmit ? () => unawaited(_submit()) : null,
                                              ),
                                            ],
                                          ),
                                        ),
                                      ],
                                    ),

                                    // 右上角轻量关闭按钮（悬停淡白圆底，对齐 duskbot）。
                                    Positioned(
                                      right: 10,
                                      top: 10,
                                      child: Tooltip(
                                        message: '关闭',
                                        child: Material(
                                          color: Colors.transparent,
                                          shape: const CircleBorder(),
                                          child: InkWell(
                                            customBorder: const CircleBorder(),
                                            hoverColor: const Color(0x18FFFFFF),
                                            onTap: widget.onClose,
                                            child: const SizedBox.square(
                                              dimension: 32,
                                              child: Icon(Icons.close_rounded, size: 16, color: inkSecondary),
                                            ),
                                          ),
                                        ),
                                      ),
                                    ),
                                  ],
                                ),
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ],
        );
      },
    );
  }
}

/// 各天体在深邃暗金夜雾玻璃上的柔和氛围色（克制宝石调，随选中星球 620ms 丝滑染晕面板上方）。
Color _celestialAuraTint(CelestialStyle style) => switch (style) {
  CelestialStyle.ringed => const Color(0xFFE5BA72),
  CelestialStyle.banded => const Color(0xFFD88450),
  CelestialStyle.oceanic => const Color(0xFF47A4DB),
  CelestialStyle.terracotta => const Color(0xFFDA6848),
  CelestialStyle.lunar => const Color(0xFFB7C8E4),
  CelestialStyle.ice => const Color(0xFF56CBD8),
  CelestialStyle.deepSea => const Color(0xFF447CE8),
  CelestialStyle.gilded => const Color(0xFFE6A989),
  CelestialStyle.star => const Color(0xFFC8DCFF),
  CelestialStyle.sun => const Color(0xFFE6AD48),
};

Future<ui.FragmentProgram>? _veilProgram;
Future<ui.FragmentProgram> _loadVeilProgram() =>
    _veilProgram ??= ui.FragmentProgram.fromAsset('shaders/modal_veil.frag');

/// 弹窗内部液态流光底幕：GPU 片段着色器驱动文艺复兴暗金夜雾 + 随选中星球 620ms 平滑染色，
/// 复用共享 [CelestialClock]（无新定时器、零逐帧 widget 重建），单测或着色器未就绪时自动平滑回退。
class _ModalVeilBackdrop extends StatefulWidget {
  const _ModalVeilBackdrop({required this.clock, required this.accentColor});

  final CelestialClock clock;
  final Color accentColor;

  @override
  State<_ModalVeilBackdrop> createState() => _ModalVeilBackdropState();
}

class _ModalVeilBackdropState extends State<_ModalVeilBackdrop> with SingleTickerProviderStateMixin {
  ui.FragmentShader? _shader;
  bool _loading = false;

  late Color _fromAccent = widget.accentColor;
  late Color _toAccent = widget.accentColor;
  late final AnimationController _accentCtrl = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 620),
    value: 1.0,
  );

  Color get _currentAccent =>
      Color.lerp(_fromAccent, _toAccent, Curves.easeInOutCubic.transform(_accentCtrl.value)) ?? _toAccent;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (CosmosBackdrop.animate && !_loading) {
      _loading = true;
      _load();
    }
  }

  @override
  void didUpdateWidget(_ModalVeilBackdrop old) {
    super.didUpdateWidget(old);
    if (old.accentColor != widget.accentColor) {
      _fromAccent = _currentAccent;
      _toAccent = widget.accentColor;
      _accentCtrl.forward(from: 0.0);
    }
  }

  Future<void> _load() async {
    try {
      final program = await _loadVeilProgram();
      if (!mounted) return;
      setState(() => _shader = program.fragmentShader());
    } catch (e) {
      debugPrint('弹窗流光着色器加载失败，回退为静谧夜雾渐变：$e');
    }
  }

  @override
  void dispose() {
    _accentCtrl.dispose();
    _shader?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return IgnorePointer(
      child: RepaintBoundary(
        child: CustomPaint(
          painter: _ModalVeilPainter(
            clock: widget.clock,
            accentAnim: _accentCtrl,
            accentOf: () => _currentAccent,
            shader: _shader,
          ),
        ),
      ),
    );
  }
}

class _ModalVeilPainter extends CustomPainter {
  _ModalVeilPainter({required this.clock, required this.accentAnim, required this.accentOf, required this.shader})
    : super(repaint: Listenable.merge([clock, accentAnim]));

  final CelestialClock clock;
  final Animation<double> accentAnim;
  final ValueGetter<Color> accentOf;
  final ui.FragmentShader? shader;

  @override
  void paint(Canvas canvas, Size size) {
    if (size.isEmpty) return;
    final accent = accentOf();
    final heroY = (102.0 / size.height).clamp(0.14, 0.36);
    final s = shader;
    if (s != null) {
      s
        ..setFloat(0, size.width)
        ..setFloat(1, size.height)
        ..setFloat(2, clock.value)
        ..setFloat(3, accent.r)
        ..setFloat(4, accent.g)
        ..setFloat(5, accent.b)
        ..setFloat(6, heroY);
      canvas.drawRect(Offset.zero & size, Paint()..shader = s);
      return;
    }

    // 回退绘制（单测或着色器首帧异步加载期间）：保持一致的深暮蓝暗金夜雾 + 顶部柔和星晕。
    final rect = Offset.zero & size;
    canvas.drawRect(
      rect,
      Paint()
        ..shader = ui.Gradient.linear(
          rect.topLeft,
          rect.bottomRight,
          const [Color(0xDE131E38), Color(0xE60E162B), Color(0xEB0A1020)],
          const [0.0, 0.54, 1.0],
        ),
    );
    final heroCenter = Offset(size.width * 0.5, size.height * heroY);
    final auraR = size.width * 0.44;
    canvas.drawRect(
      rect,
      Paint()
        ..shader = ui.Gradient.radial(
          heroCenter,
          auraR,
          [accent.withValues(alpha: 0.16), accent.withValues(alpha: 0.05), accent.withValues(alpha: 0.0)],
          const [0.0, 0.55, 1.0],
        ),
    );
  }

  @override
  bool shouldRepaint(_ModalVeilPainter old) =>
      old.shader != shader || old.clock != clock || old.accentAnim != accentAnim;
}

/// 居中实机 GPU 大星球裸浮预览：不加任何圆框，星球背后的柔光晕由底层 [_ModalVeilBackdrop] 统一流动渲染。
class _HeroCelestialPreview extends StatelessWidget {
  const _HeroCelestialPreview({required this.style, required this.bounceTick, required this.clock});

  final CelestialStyle style;
  final int bounceTick;
  final CelestialClock clock;

  @override
  Widget build(BuildContext context) {
    final radius = switch (style) {
      CelestialStyle.ringed => 26.0,
      CelestialStyle.star => 27.0,
      CelestialStyle.ice => 29.0,
      _ => 34.0,
    };

    return SizedBox(
      width: 136,
      height: 112,
      child: Center(
        child: TweenAnimationBuilder<double>(
          key: ValueKey('hero-bounce-$bounceTick-${style.name}'),
          tween: Tween(begin: 0.88, end: 1.0),
          duration: const Duration(milliseconds: 220),
          curve: Curves.easeOutBack,
          builder: (_, s, child) => Transform.scale(scale: s, child: child),
          child: CelestialBody(
            key: ValueKey('create-bot-hero-${style.name}'),
            radius: radius,
            style: style,
            seed: 0.23,
            selected: true,
            clock: clock,
            semanticsLabel: style.bodyName,
            onTap: () {},
          ),
        ),
      ),
    );
  }
}

/// 居中两排（5 + 4）纯裸浮实机星球选择器：无圆底、无文字、无角落小点（太阳严格排除）。
class _CelestialPickerRows extends StatelessWidget {
  const _CelestialPickerRows({
    required this.selected,
    required this.usedCounts,
    required this.clock,
    required this.onSelect,
  });

  final Celestial selected;
  final Map<CelestialStyle, int> usedCounts;
  final CelestialClock clock;
  final ValueChanged<Celestial> onSelect;

  @override
  Widget build(BuildContext context) {
    final row1 = selectableBotCelestials.take(5).toList();
    final row2 = selectableBotCelestials.skip(5).toList();

    Widget buildRow(List<Celestial> items, int indexOffset) => Row(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        for (var i = 0; i < items.length; i++) ...[
          if (i > 0) const SizedBox(width: 10),
          _CelestialTile(
            celestial: items[i],
            seed: (indexOffset + i + 1) * 0.17,
            selected: items[i].id == selected.id,
            usedCount: usedCounts[CelestialStyle.fromCelestial(items[i])] ?? 0,
            clock: clock,
            onTap: () => onSelect(items[i]),
          ),
        ],
      ],
    );

    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [buildRow(row1, 0), const SizedBox(height: 8), buildRow(row2, row1.length)],
    );
  }
}

/// 单颗裸浮星球按钮：未选中零边框纯星球；悬停或选中时仅在周围亮出解析抗锯齿淡白细环。
class _CelestialTile extends StatefulWidget {
  const _CelestialTile({
    required this.celestial,
    required this.seed,
    required this.selected,
    required this.usedCount,
    required this.clock,
    required this.onTap,
  });

  final Celestial celestial;
  final double seed;
  final bool selected;
  final int usedCount;
  final CelestialClock clock;
  final VoidCallback onTap;

  @override
  State<_CelestialTile> createState() => _CelestialTileState();
}

class _CelestialTileState extends State<_CelestialTile> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final st = CelestialStyle.fromCelestial(widget.celestial);
    final thumbR = switch (st) {
      CelestialStyle.ringed => 11.5,
      CelestialStyle.star => 12.0,
      CelestialStyle.ice => 13.0,
      _ => 15.0,
    };

    final ringColor = widget.selected
        ? const Color(0x66EAF2FF)
        : _hovered
        ? const Color(0x28EAF2FF)
        : const Color(0x00EAF2FF);

    return Tooltip(
      message: widget.usedCount > 0 ? '${st.bodyName} · 在轨 ${widget.usedCount} 颗' : st.bodyName,
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        onEnter: (_) => setState(() => _hovered = true),
        onExit: (_) => setState(() => _hovered = false),
        child: GestureDetector(
          key: ValueKey('create-bot-celestial-${st.name}'),
          behavior: HitTestBehavior.opaque,
          onTap: widget.onTap,
          child: SizedBox.square(
            dimension: 58,
            child: Stack(
              alignment: Alignment.center,
              clipBehavior: Clip.none,
              children: [
                // 仅悬停/选中时出现的片元级解析抗锯齿淡白细环（平时完全透明，无任何多余圆底）
                TweenAnimationBuilder<Color?>(
                  tween: ColorTween(end: ringColor),
                  duration: Motion.fast,
                  builder: (_, c, _) => _SmoothFocusRing(
                    diameter: 54,
                    strokeWidth: widget.selected ? 1.25 : 1.0,
                    color: c ?? ringColor,
                    fillAlpha: widget.selected ? 0.08 : (_hovered ? 0.04 : 0.0),
                  ),
                ),
                // 直接裸浮在深暮蓝空间中的实机 GPU 星球
                IgnorePointer(
                  child: CelestialBody(
                    radius: thumbR,
                    style: st,
                    seed: widget.seed,
                    selected: widget.selected || _hovered,
                    clock: widget.clock,
                    semanticsLabel: st.bodyName,
                    onTap: () {},
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// 片元级解析抗锯齿细环：用径向平滑环带绘制选中/悬停细环，在任意 DPR 与窗口宽度下都绝无锯齿。
class _SmoothFocusRing extends StatelessWidget {
  const _SmoothFocusRing({
    required this.diameter,
    required this.strokeWidth,
    required this.color,
    required this.fillAlpha,
  });

  final double diameter;
  final double strokeWidth;
  final Color color;
  final double fillAlpha;

  @override
  Widget build(BuildContext context) {
    if (color.a <= 0.002 && fillAlpha <= 0.002) return const SizedBox.shrink();
    return IgnorePointer(
      child: CustomPaint(
        size: Size.square(diameter + 6),
        painter: _SmoothFocusRingPainter(
          diameter: diameter,
          strokeWidth: strokeWidth,
          color: color,
          fillAlpha: fillAlpha,
        ),
      ),
    );
  }
}

class _SmoothFocusRingPainter extends CustomPainter {
  const _SmoothFocusRingPainter({
    required this.diameter,
    required this.strokeWidth,
    required this.color,
    required this.fillAlpha,
  });

  final double diameter;
  final double strokeWidth;
  final Color color;
  final double fillAlpha;

  @override
  void paint(Canvas canvas, Size size) {
    final c = size.center(Offset.zero);
    final rMid = diameter / 2;

    if (fillAlpha > 0.002) {
      final fillOuter = rMid + 0.8;
      final fillPaint = Paint()
        ..shader = ui.Gradient.radial(
          c,
          fillOuter,
          [
            const Color(0xFFFFFFFF).withValues(alpha: fillAlpha),
            const Color(0xFFFFFFFF).withValues(alpha: fillAlpha * 0.65),
            const Color(0x00FFFFFF),
          ],
          [0.0, ((rMid - 1.2) / fillOuter).clamp(0.0, 0.98), 1.0],
        );
      canvas.drawRect(Rect.fromCircle(center: c, radius: fillOuter), fillPaint);
    }

    if (color.a > 0.002) {
      final half = strokeWidth / 2;
      const aa = 0.85;
      final outer = rMid + half + aa + 0.5;
      final clear = color.withValues(alpha: 0);
      final s0 = ((rMid - half - aa) / outer).clamp(0.0, 0.96);
      final s1 = ((rMid - half + 0.15) / outer).clamp(s0 + 0.002, 0.98);
      final s2 = ((rMid + half - 0.15) / outer).clamp(s1 + 0.002, 0.99);
      final s3 = ((rMid + half + aa) / outer).clamp(s2 + 0.002, 1.0);
      final ringPaint = Paint()..shader = ui.Gradient.radial(c, outer, [clear, color, color, clear], [s0, s1, s2, s3]);
      canvas.drawRect(Rect.fromCircle(center: c, radius: outer), ringPaint);
    }
  }

  @override
  bool shouldRepaint(_SmoothFocusRingPainter old) =>
      old.diameter != diameter || old.strokeWidth != strokeWidth || old.color != color || old.fillAlpha != fillAlpha;
}

/// 底部 32px 全圆角胶囊按钮（对齐 duskbot KIT_BUTTON）。
class _PillActionButton extends StatelessWidget {
  const _PillActionButton({
    super.key,
    required this.label,
    required this.background,
    required this.hoverBackground,
    required this.foreground,
    required this.onTap,
  });

  final String label;
  final Color background;
  final Color hoverBackground;
  final Color foreground;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: background,
      shape: const StadiumBorder(),
      child: InkWell(
        customBorder: const StadiumBorder(),
        hoverColor: onTap == null ? Colors.transparent : hoverBackground,
        onTap: onTap,
        child: SizedBox(
          height: 32,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 14),
            child: Center(
              child: Text(
                label,
                style: TextStyle(fontSize: 13.5, fontWeight: FontWeight.w500, color: foreground, height: 1.2),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// 折叠在左下角入口里的「岗位与高级设置」（对齐 ip.yaml 的 Role & AgentConfig 字段，深暮蓝玻璃配色）。
class _RoleAndAdvancedSection extends StatelessWidget {
  const _RoleAndAdvancedSection({
    required this.existingRoles,
    required this.roleMode,
    required this.selectedRoleId,
    required this.roleNameCtrl,
    required this.roleDescCtrl,
    required this.modelCtrl,
    required this.escalateCtrl,
    required this.budgetCtrl,
    required this.workspaceCtrl,
    required this.tools,
    required this.onRoleModeChanged,
    required this.onSelectedRoleChanged,
    required this.onToolsChanged,
  });

  final List<Role> existingRoles;
  final _RoleMode roleMode;
  final String? selectedRoleId;
  final TextEditingController roleNameCtrl;
  final TextEditingController roleDescCtrl;
  final TextEditingController modelCtrl;
  final TextEditingController escalateCtrl;
  final TextEditingController budgetCtrl;
  final TextEditingController workspaceCtrl;
  final bool tools;
  final ValueChanged<_RoleMode> onRoleModeChanged;
  final ValueChanged<String> onSelectedRoleChanged;
  final ValueChanged<bool> onToolsChanged;

  @override
  Widget build(BuildContext context) {
    const ink = Color(0xF5F7FAFF);
    const inkSecondary = Color(0xA6D6E2FA);
    const inkTertiary = Color(0x66D6E2FA);
    final createsNewRole = roleMode == _RoleMode.createNew || existingRoles.isEmpty;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            const Text(
              '归属岗位',
              style: TextStyle(fontSize: 12, fontWeight: FontWeight.w600, color: ink),
            ),
            const Spacer(),
            _MiniSegment(
              value: createsNewRole ? _RoleMode.createNew : _RoleMode.existing,
              existingEnabled: existingRoles.isNotEmpty,
              onChanged: onRoleModeChanged,
            ),
          ],
        ),
        const SizedBox(height: 8),
        if (!createsNewRole)
          _RoleDropdown(
            roles: existingRoles,
            selectedId: selectedRoleId ?? existingRoles.first.id,
            onChanged: onSelectedRoleChanged,
          )
        else ...[
          _CompactField(
            key: const ValueKey('create-bot-role-name-input'),
            controller: roleNameCtrl,
            hintText: '岗位名称（留空默认同 Bot 名）',
          ),
          const SizedBox(height: 8),
          _CompactField(
            key: const ValueKey('create-bot-role-desc-input'),
            controller: roleDescCtrl,
            maxLines: 2,
            hintText: '职责说明（对应 ip.yaml description，参与自动路由）',
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text('默认模型', style: TextStyle(fontSize: 11, color: inkSecondary)),
                    const SizedBox(height: 4),
                    _CompactField(
                      key: const ValueKey('create-bot-model-input'),
                      controller: modelCtrl,
                      hintText: 'openai/gpt-6-luna',
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text('升级模型', style: TextStyle(fontSize: 11, color: inkSecondary)),
                    const SizedBox(height: 4),
                    _CompactField(
                      key: const ValueKey('create-bot-escalate-input'),
                      controller: escalateCtrl,
                      hintText: '选填',
                    ),
                  ],
                ),
              ),
            ],
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text('每日预算（美元）', style: TextStyle(fontSize: 11, color: inkSecondary)),
                    const SizedBox(height: 4),
                    _CompactField(
                      key: const ValueKey('create-bot-budget-input'),
                      controller: budgetCtrl,
                      keyboardType: const TextInputType.numberWithOptions(decimal: true),
                      hintText: '选填，如 2',
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Text('工作目录', style: TextStyle(fontSize: 11, color: inkSecondary)),
                    const SizedBox(height: 4),
                    _CompactField(
                      key: const ValueKey('create-bot-workspace-input'),
                      controller: workspaceCtrl,
                      hintText: '留空自动分配',
                    ),
                  ],
                ),
              ),
            ],
          ),
        ],
        if (!createsNewRole) ...[
          const SizedBox(height: 8),
          const Text('工作目录', style: TextStyle(fontSize: 11, color: inkSecondary)),
          const SizedBox(height: 4),
          _CompactField(
            key: const ValueKey('create-bot-workspace-input'),
            controller: workspaceCtrl,
            hintText: '留空默认 workspaces/<id>（模型与预算沿用所选岗位）',
          ),
        ],
        const SizedBox(height: 8),
        Row(
          children: [
            const Expanded(
              child: Text('启用沙箱工具（shell / 文件读写）', style: TextStyle(fontSize: 12, color: ink)),
            ),
            Switch.adaptive(
              key: const ValueKey('create-bot-tools-switch'),
              value: tools,
              activeTrackColor: const Color(0xFF4F8FF0),
              onChanged: onToolsChanged,
            ),
          ],
        ),
        const SizedBox(height: 4),
        const Text(
          '当前主机暂未开放 create_role / create_agent 与 celestial_style 持久化接口，提交后先在本机星图生效演示。',
          style: TextStyle(fontSize: 10.5, color: inkTertiary, height: 1.45),
        ),
      ],
    );
  }
}

class _MiniSegment extends StatelessWidget {
  const _MiniSegment({required this.value, required this.existingEnabled, required this.onChanged});

  final _RoleMode value;
  final bool existingEnabled;
  final ValueChanged<_RoleMode> onChanged;

  @override
  Widget build(BuildContext context) {
    Widget tab(_RoleMode mode, String label, bool enabled, Key key) {
      final active = value == mode;
      return InkWell(
        key: key,
        onTap: enabled ? () => onChanged(mode) : null,
        borderRadius: BorderRadius.circular(7),
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 3.5),
          decoration: BoxDecoration(
            color: active ? const Color(0x28FFFFFF) : Colors.transparent,
            borderRadius: BorderRadius.circular(7),
          ),
          child: Text(
            label,
            style: TextStyle(
              fontSize: 11.5,
              fontWeight: active ? FontWeight.w600 : FontWeight.w400,
              color: !enabled
                  ? const Color(0x45D6E2FA)
                  : active
                  ? const Color(0xFFF7FAFF)
                  : const Color(0x99D6E2FA),
            ),
          ),
        ),
      );
    }

    return Container(
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(color: const Color(0x14FFFFFF), borderRadius: BorderRadius.circular(9)),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          tab(_RoleMode.existing, '已有岗位', existingEnabled, const ValueKey('create-bot-role-mode-existing')),
          tab(_RoleMode.createNew, '新建岗位', true, const ValueKey('create-bot-role-mode-new')),
        ],
      ),
    );
  }
}

class _RoleDropdown extends StatelessWidget {
  const _RoleDropdown({required this.roles, required this.selectedId, required this.onChanged});

  final List<Role> roles;
  final String selectedId;
  final ValueChanged<String> onChanged;

  @override
  Widget build(BuildContext context) {
    final safeId = roles.any((r) => r.id == selectedId) ? selectedId : roles.first.id;
    return Container(
      height: 36,
      padding: const EdgeInsets.symmetric(horizontal: 11),
      decoration: ShapeDecoration(
        color: const Color(0x14FFFFFF),
        shape: RoundedSuperellipseBorder(
          borderRadius: BorderRadius.circular(10),
          side: const BorderSide(color: Color(0x24FFFFFF), width: 0.7),
        ),
      ),
      child: DropdownButtonHideUnderline(
        child: DropdownButton<String>(
          key: const ValueKey('create-bot-role-dropdown'),
          isExpanded: true,
          value: safeId,
          iconEnabledColor: const Color(0xA6D6E2FA),
          dropdownColor: const Color(0xF5182540),
          style: const TextStyle(fontSize: 12.5, color: Color(0xF5F7FAFF)),
          items: [
            for (final r in roles)
              DropdownMenuItem(
                value: r.id,
                child: Row(
                  children: [
                    Flexible(child: Text(r.displayName, overflow: TextOverflow.ellipsis)),
                    const SizedBox(width: 8),
                    Text(
                      r.model,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(fontSize: 10.5, fontFamily: 'monospace', color: Color(0x7AD6E2FA)),
                    ),
                  ],
                ),
              ),
          ],
          onChanged: (v) {
            if (v != null) onChanged(v);
          },
        ),
      ),
    );
  }
}

class _CompactField extends StatelessWidget {
  const _CompactField({
    super.key,
    required this.controller,
    required this.hintText,
    this.maxLines = 1,
    this.keyboardType,
  });

  final TextEditingController controller;
  final String hintText;
  final int maxLines;
  final TextInputType? keyboardType;

  @override
  Widget build(BuildContext context) {
    return TextField(
      controller: controller,
      maxLines: maxLines,
      keyboardType: keyboardType,
      cursorColor: const Color(0xFFF7FAFF),
      style: const TextStyle(fontSize: 12.5, color: Color(0xF5F7FAFF)),
      decoration: InputDecoration(
        isDense: true,
        hintText: hintText,
        hintStyle: const TextStyle(fontSize: 12, color: Color(0x66D6E2FA)),
        filled: true,
        fillColor: const Color(0x14FFFFFF),
        contentPadding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8.5),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: const BorderSide(color: Color(0x24FFFFFF), width: 0.7),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(10),
          borderSide: const BorderSide(color: Color(0x66D6E2FA), width: 1.0),
        ),
      ),
    );
  }
}
