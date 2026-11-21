// 连上主机后的主界面：待办、星图、对话、任务、岗位，“我”放在顶栏右上角（宽屏在导航栏底部）。
// 窄屏导航在底部，宽屏在左边。

import 'package:flutter/material.dart';

import '../protocol/client.dart';
import 'chat_page.dart';
import 'common.dart';
import 'inbox_page.dart';
import 'me_page.dart';
import 'roles_page.dart';
import 'star_map_page.dart';
import 'tasks_page.dart';

/// 导航挪到左边的最小宽度，和网页控制台的窄屏断点一样。
const double kWideNav = 720;

/// 各页在导航里的位置。底部导航最多放 5 个，“我”用得少，放到顶栏。
const int kTabInbox = 0, kTabStars = 1, kTabChat = 2, kTabTasks = 3, kTabRoles = 4;

class HomePage extends StatefulWidget {
  const HomePage({super.key});

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  /// 先打开待办（手机首页，apps-plan Q-6）。
  int _tab = kTabInbox;

  bool _threadOpen = false;

  void _openThread(String threadId) {
    final state = AppScope.read(context);
    state.openThread(threadId);
    _showChat();
  }

  /// 去对话页看现在打开着的对话（星图气泡里点“在对话页打开”）。
  void _showChat() {
    setState(() {
      _tab = kTabChat;
      _threadOpen = true;
    });
  }

  void _newChat() {
    AppScope.read(context).startNewChat();
    _showChat();
  }

  /// 岗位页点“和 X 对话”：开一个发给这位员工的新对话。
  void _chatWith(String agentId) {
    final state = AppScope.read(context);
    state.startNewChat();
    state.chooseAgent(agentId);
    _showChat();
  }

  void _openMe() => Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => Scaffold(
        appBar: AppBar(title: const Text('我')),
        body: const MePage(),
      ),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final wide = MediaQuery.sizeOf(context).width >= kWideNav;
    final badge = state.inboxCount;
    Widget inboxIcon(IconData icon) => Badge(isLabelVisible: badge > 0, label: Text('$badge'), child: Icon(icon));
    final pages = [
      InboxPage(onOpenThread: _openThread),
      StarMapPage(onOpenChat: _showChat),
      ChatPage(
        active: _tab == kTabChat,
        threadOpen: _threadOpen,
        onOpenThread: _openThread,
        onNewChat: _newChat,
        onShowDetail: _showChat,
        onBack: () => setState(() => _threadOpen = false),
      ),
      TasksPage(onOpenThread: _openThread),
      RolesPage(onChat: _chatWith),
    ];
    const titles = ['待办', '星图', '对话', '任务', '岗位'];
    final icons = [
      (inboxIcon(Icons.inbox_outlined), inboxIcon(Icons.inbox)),
      (const Icon(Icons.auto_awesome_outlined), const Icon(Icons.auto_awesome)),
      (const Icon(Icons.chat_bubble_outline), const Icon(Icons.chat_bubble)),
      (const Icon(Icons.task_alt_outlined), const Icon(Icons.task_alt)),
      (const Icon(Icons.groups_outlined), const Icon(Icons.groups)),
    ];
    final me = IconButton(tooltip: '我', onPressed: _openMe, icon: const Icon(Icons.account_circle_outlined));
    final body = Column(
      children: [
        const ConnectionBanner(),
        Expanded(
          child: IndexedStack(
            index: _tab,
            // 看不见的页不跑动画：星图的星星不闪、不公转，也不耗电。
            children: [for (final (i, p) in pages.indexed) TickerMode(enabled: i == _tab, child: p)],
          ),
        ),
      ],
    );
    final phoneDetail = !wide && _tab == kTabChat && _threadOpen;
    return Scaffold(
      appBar: wide || _tab == kTabStars || phoneDetail ? null : AppBar(title: Text(titles[_tab]), actions: [me]),
      body: SafeArea(
        bottom: wide || phoneDetail,
        child: Row(
          children: [
            if (wide) ...[
              NavigationRail(
                selectedIndex: _tab,
                labelType: NavigationRailLabelType.all,
                onDestinationSelected: (i) => setState(() => _tab = i),
                trailing: Expanded(
                  child: Align(
                    alignment: Alignment.bottomCenter,
                    child: Padding(padding: const EdgeInsets.only(bottom: 12), child: me),
                  ),
                ),
                destinations: [
                  for (final (i, (icon, selected)) in icons.indexed)
                    NavigationRailDestination(icon: icon, selectedIcon: selected, label: Text(titles[i])),
                ],
              ),
              const VerticalDivider(width: 1),
            ],
            Expanded(child: body),
          ],
        ),
      ),
      floatingActionButton: !wide && !phoneDetail && _tab == kTabChat
          ? FloatingActionButton(tooltip: '新对话', onPressed: _newChat, child: const Icon(Icons.edit))
          : null,
      bottomNavigationBar: wide || phoneDetail
          ? null
          : NavigationBar(
              selectedIndex: _tab,
              onDestinationSelected: (i) => setState(() => _tab = i),
              destinations: [
                for (final (i, (icon, selected)) in icons.indexed)
                  NavigationDestination(icon: icon, selectedIcon: selected, label: titles[i]),
              ],
            ),
    );
  }
}

/// 没连上、被拒绝、协议版本不一致时顶上的一行。
class ConnectionBanner extends StatelessWidget {
  const ConnectionBanner({super.key});

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final scheme = Theme.of(context).colorScheme;
    final (String? text, bool bad) = switch (state) {
      _ when state.rejected != null => (state.rejected!.message, true),
      _ when state.outdated => ('主机的协议版本是 ${state.serverProtocol}，这个 App 说的是 $kProtocol：请把 App 和主机升到同一版', true),
      _ when state.status == ConnStatus.connecting => ('正在连接 ${state.host?.address.text ?? ''}…', false),
      _ when state.status == ConnStatus.closed => ('连接断开了，正在重连…', false),
      _ when state.loadError != null => (state.loadError, true),
      _ => (null, false),
    };
    if (text == null) return const SizedBox.shrink();
    return Material(
      color: bad ? scheme.errorContainer : scheme.secondaryContainer,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
        child: Row(
          children: [
            Expanded(
              child: Text(text, style: TextStyle(color: bad ? scheme.onErrorContainer : scheme.onSecondaryContainer)),
            ),
            if (state.rejected != null)
              TextButton(onPressed: () => AppScope.read(context).forgetHost(), child: const Text('重新配对')),
          ],
        ),
      ),
    );
  }
}
