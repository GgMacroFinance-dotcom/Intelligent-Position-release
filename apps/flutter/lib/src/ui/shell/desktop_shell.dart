// 桌面外壳（设计重构第一步，照 dusk bot 的外层框架）：左栏、中间、右栏三栏。
//   左栏：固定入口（待办、星图、任务、全部对话）和按岗位分组的员工；可拖宽，拖到最窄收起成头像列。
//   中间：待办、星图、对话、任务、岗位之一。对话的顶上浮着一个员工名字的胶囊，点了打开右栏。
//   右栏：员工面板（概况、任务、岗位），点员工名字的胶囊或左栏员工旁的按钮打开。
// 窗口窄于 Sizes.shellMin 时不用这个外壳，退回原来的导航（手机和窄窗口，home_page.dart）。
// 星图页本身先不动（维护者后面一步步说）。

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../app_state.dart';
import '../../protocol/models.dart';
import '../chat_page.dart';
import '../common.dart';
import '../home_page.dart' show ConnectionBanner;
import '../inbox_page.dart';
import '../roles_page.dart';
import '../star_map_page.dart';
import '../tasks_page.dart';
import 'avatar.dart';
import 'panel.dart';
import 'search.dart';
import 'settings.dart';
import 'sidebar.dart';
import 'tokens.dart';

class DesktopShell extends StatefulWidget {
  const DesktopShell({super.key, this.initialView = ShellView.inbox, this.onViewChanged});

  /// 打开时显示哪一页（从窄屏切过来时，接着显示原来那一页）。
  final ShellView initialView;

  /// 换了一页时告诉外面，窗口变窄退回原来的导航时接着显示这一页。
  final void Function(ShellView view)? onViewChanged;

  @override
  State<DesktopShell> createState() => _DesktopShellState();
}

class _DesktopShellState extends State<DesktopShell> {
  late ShellView _view = widget.initialView;
  double _sidebar = Sizes.sidebar;

  /// 拖着左栏边缘时的宽度（松手后才吸附到收起或展开）。
  double? _dragging;

  /// 右栏开着的员工；为空时右栏收起。
  String? _panelAgent;

  /// 对话里“全部对话”的列表是否展开在中间（点左栏“全部对话”）。
  bool _threadList = false;

  /// 打开一个对话。从“对话”列表里点的，列表留着（列表和对话并排，features.txt C-2）。
  void _openThread(String threadId, {bool keepList = false}) {
    AppScope.read(context).openThread(threadId);
    setState(() {
      _view = ShellView.chat;
      _threadList = keepList && _threadList;
    });
  }

  void _openAgent(Agent agent, String? threadId) {
    final state = AppScope.read(context);
    if (threadId != null) {
      state.openThread(threadId);
    } else {
      state.startNewChat();
      state.chooseAgent(agent.id);
    }
    setState(() {
      _view = ShellView.chat;
      _threadList = false;
      if (_panelAgent != null) _panelAgent = agent.id;
    });
  }

  void _newChat() {
    AppScope.read(context).startNewChat();
    setState(() {
      _view = ShellView.chat;
      _threadList = false;
    });
  }

  void _chatWith(String agentId) {
    final state = AppScope.read(context);
    state.startNewChat();
    state.chooseAgent(agentId);
    setState(() {
      _view = ShellView.chat;
      _threadList = false;
    });
  }

  void _search() => showMessageSearch(context, onOpen: _openThread);

  void _settings() => showSettings(context);

  /// 点了系统通知、或者“管理”各页里点了“看对话”：先关掉弹窗，去那个对话；没有对话的去待办。
  void _handlePendingOpen() {
    final state = AppScope.read(context);
    if (state.pendingOpen == null) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      final open = state.takePendingOpen();
      if (open == null) return;
      Navigator.of(context).popUntil((r) => r.isFirst);
      final (threadId,) = open;
      if (threadId == null) {
        setState(() => _view = ShellView.inbox);
      } else {
        _openThread(threadId);
      }
    });
  }

  ShellView? _reported;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    _handlePendingOpen();
    if (_reported != _view) {
      _reported = _view;
      final v = _view;
      WidgetsBinding.instance.addPostFrameCallback((_) => widget.onViewChanged?.call(v));
    }
    final width = MediaQuery.sizeOf(context).width;
    final sidebar = (_dragging ?? _sidebar).clamp(Sizes.sidebarRail, Sizes.sidebarMax);
    // 对话至少留 420：窗口不够宽时右栏盖在对话上面。
    final panelOpen = _panelAgent != null && state.agents.any((a) => a.id == _panelAgent);
    final panelFits = width - sidebar - Sizes.panel >= 420;
    final selectedAgent = _view == ShellView.chat && !_threadList ? state.currentAgent : null;
    final center = Column(
      children: [
        const ConnectionBanner(),
        Expanded(child: _center(state)),
      ],
    );
    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.keyK, meta: true): _search,
        const SingleActivator(LogicalKeyboardKey.keyK, control: true): _search,
        const SingleActivator(LogicalKeyboardKey.keyN, meta: true): _newChat,
        const SingleActivator(LogicalKeyboardKey.keyN, control: true): _newChat,
        const SingleActivator(LogicalKeyboardKey.comma, meta: true): _settings,
        const SingleActivator(LogicalKeyboardKey.escape): () {
          if (_panelAgent != null) setState(() => _panelAgent = null);
        },
      },
      child: Focus(
        autofocus: true,
        child: Scaffold(
          backgroundColor: s.panel,
          body: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              SizedBox(
                width: sidebar,
                child: ShellSidebar(
                  width: sidebar,
                  view: _view,
                  selectedAgent: selectedAgent,
                  onView: (v) => setState(() {
                    _view = v;
                    _threadList = false;
                  }),
                  onAgent: _openAgent,
                  onNewChat: _newChat,
                  onSearch: _search,
                  onSettings: _settings,
                  onAllThreads: () => setState(() {
                    _view = ShellView.chat;
                    _threadList = true;
                  }),
                ),
              ),
              _ResizeEdge(
                onDrag: (dx) => setState(() => _dragging = (_dragging ?? _sidebar) + dx),
                onEnd: () => setState(() {
                  final w = _dragging ?? _sidebar;
                  // 拖到比最窄还窄一半时收起成头像列，否则停在拖到的宽度。
                  _sidebar = w < Sizes.sidebarMin - 60 ? Sizes.sidebarRail : w.clamp(Sizes.sidebarMin, Sizes.sidebarMax);
                  _dragging = null;
                }),
                onDoubleTap: () => setState(() {
                  _sidebar = _sidebar <= Sizes.sidebarRail + 1 ? Sizes.sidebar : Sizes.sidebarRail;
                }),
              ),
              Expanded(
                child: Padding(
                  // 中间的对话区是一块圆角的白纸，浮在左右栏的灰底上（照 dusk 的“frame”）。
                  padding: EdgeInsets.fromLTRB(0, 8, panelOpen && panelFits ? 0 : 8, 8),
                  child: ClipRRect(
                    borderRadius: BorderRadius.circular(Radii.xl),
                    // Material 而不是 ColoredBox：里面的列表（ListTile）要在它上面画选中和点按效果。
                    child: Material(
                      color: s.app,
                      child: Stack(
                        children: [
                          Positioned.fill(child: center),
                          if (panelOpen && !panelFits) ...[
                            Positioned.fill(
                              child: GestureDetector(
                                onTap: () => setState(() => _panelAgent = null),
                                child: ColoredBox(color: s.scrim),
                              ),
                            ),
                            Positioned(
                              top: 0,
                              bottom: 0,
                              right: 0,
                              width: Sizes.panel,
                              child: _panel(),
                            ),
                          ],
                        ],
                      ),
                    ),
                  ),
                ),
              ),
              AnimatedSize(
                duration: Motion.dialog,
                curve: Motion.curve,
                child: panelOpen && panelFits ? SizedBox(width: Sizes.panel, child: _panel()) : const SizedBox.shrink(),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _panel() => AgentPanel(
    key: ValueKey('panel-$_panelAgent'),
    agentId: _panelAgent!,
    onClose: () => setState(() => _panelAgent = null),
    onOpenThread: _openThread,
  );

  Widget _center(AppState state) {
    // 各页都挂着（IndexedStack），切换不丢滚动位置和草稿；看不见的页不跑动画（星图不闪不转）。
    final pages = <ShellView, Widget>{
      ShellView.inbox: _Page(title: '待办', child: InboxPage(onOpenThread: _openThread)),
      ShellView.stars: StarMapPage(onOpenChat: () => setState(() => _view = ShellView.chat)),
      ShellView.chat: _ChatColumn(
        listOpen: _threadList,
        onOpenThread: _openThread,
        onListOpen: (id) => _openThread(id, keepList: true),
        onNewChat: () {
          AppScope.read(context).startNewChat();
          setState(() => _view = ShellView.chat);
        },
        onAgentPill: (id) => setState(() => _panelAgent = _panelAgent == id ? null : id),
      ),
      ShellView.tasks: _Page(title: '任务', child: TasksPage(onOpenThread: _openThread)),
      ShellView.roles: _Page(title: '岗位与员工', child: RolesPage(onChat: _chatWith)),
    };
    return IndexedStack(
      index: ShellView.values.indexOf(_view),
      children: [
        for (final v in ShellView.values) TickerMode(enabled: v == _view, child: pages[v]!),
      ],
    );
  }
}

/// 左栏的右边缘：拖动改宽度，双击收起或展开。
class _ResizeEdge extends StatefulWidget {
  const _ResizeEdge({required this.onDrag, required this.onEnd, required this.onDoubleTap});

  final void Function(double dx) onDrag;
  final VoidCallback onEnd;
  final VoidCallback onDoubleTap;

  @override
  State<_ResizeEdge> createState() => _ResizeEdgeState();
}

class _ResizeEdgeState extends State<_ResizeEdge> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return MouseRegion(
      cursor: SystemMouseCursors.resizeColumn,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onHorizontalDragUpdate: (d) => widget.onDrag(d.delta.dx),
        onHorizontalDragEnd: (_) => widget.onEnd(),
        onDoubleTap: widget.onDoubleTap,
        child: SizedBox(
          width: 8,
          child: Center(
            child: AnimatedContainer(
              duration: Motion.fast,
              width: 2,
              height: 36,
              decoration: BoxDecoration(
                color: _hover ? s.hairline : Colors.transparent,
                borderRadius: BorderRadius.circular(1),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// 中间一页的标题（待办、任务、岗位）：大字，左上角，不用顶栏。
class _Page extends StatelessWidget {
  const _Page({required this.title, required this.child});

  final String title;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(28, 22, 28, 6),
          child: Text(title, style: TextStyle(color: s.ink, fontSize: 20, fontWeight: FontWeight.w600)),
        ),
        Expanded(
          child: Align(
            alignment: Alignment.topCenter,
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: Sizes.chatColumn),
              child: Padding(padding: const EdgeInsets.symmetric(horizontal: 16), child: child),
            ),
          ),
        ),
      ],
    );
  }
}

/// 中间的对话：顶上浮着员工名字的胶囊，对话栏最宽 60rem 居中。
class _ChatColumn extends StatelessWidget {
  const _ChatColumn({
    required this.listOpen,
    required this.onOpenThread,
    required this.onListOpen,
    required this.onNewChat,
    required this.onAgentPill,
  });

  /// 左栏点了“对话”：对话列表放在中间左边，和对话并排。
  final bool listOpen;
  final void Function(String threadId) onOpenThread;
  final void Function(String threadId) onListOpen;
  final VoidCallback onNewChat;
  final void Function(String agentId) onAgentPill;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    final thread = _thread(context);
    if (!listOpen) return thread;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SizedBox(
          width: 300,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(20, 18, 16, 8),
                child: Text('对话', style: TextStyle(color: s.ink, fontSize: 17, fontWeight: FontWeight.w600)),
              ),
              Expanded(child: ThreadList(onOpen: onListOpen, onNew: onNewChat)),
            ],
          ),
        ),
        VerticalDivider(width: 1, color: s.edge),
        Expanded(child: thread),
      ],
    );
  }

  Widget _thread(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final agent = state.currentAgent;
    final a = agent == null ? null : state.agents.where((x) => x.id == agent).firstOrNull;
    return Stack(
      children: [
        Positioned.fill(
          child: Padding(
            padding: const EdgeInsets.only(top: 52),
            child: Center(
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: Sizes.chatColumn + 32),
                child: const ThreadView(),
              ),
            ),
          ),
        ),
        // 顶上的淡出：消息滚到胶囊下面时变淡，不和胶囊挤在一起。
        Positioned(
          top: 0,
          left: 0,
          right: 0,
          height: 64,
          child: IgnorePointer(
            child: DecoratedBox(
              decoration: BoxDecoration(
                gradient: LinearGradient(
                  begin: Alignment.topCenter,
                  end: Alignment.bottomCenter,
                  colors: [s.app, s.app.withValues(alpha: 0)],
                ),
              ),
            ),
          ),
        ),
        Positioned(
          top: 12,
          left: 0,
          right: 0,
          child: Center(
            child: _HeadPill(
              agent: a,
              newChat: state.threadId == null,
              onTap: a == null ? null : () => onAgentPill(a.id),
            ),
          ),
        ),
      ],
    );
  }
}

/// 对话顶上的名字胶囊（照 dusk 的 chat head pill）：头像、名字；点了开关右栏。
class _HeadPill extends StatelessWidget {
  const _HeadPill({required this.agent, required this.newChat, this.onTap});

  final Agent? agent;
  final bool newChat;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    final a = agent;
    final label = a?.name ?? (newChat ? '新对话 · 自动分派' : '对话');
    return Material(
      color: s.composer,
      shape: StadiumBorder(side: BorderSide(color: s.edge, width: 0.5)),
      elevation: 0,
      child: InkWell(
        customBorder: const StadiumBorder(),
        onTap: onTap,
        child: DecoratedBox(
          decoration: BoxDecoration(borderRadius: BorderRadius.circular(999), boxShadow: s.floatShadow),
          child: Padding(
            padding: EdgeInsets.fromLTRB(a == null ? 14 : 6, 6, 14, 6),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (a != null) ...[AgentAvatar(id: a.id, name: a.name, size: 24), const SizedBox(width: 8)],
                Text(label, style: TextStyle(color: s.ink, fontSize: 14, fontWeight: FontWeight.w500)),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
