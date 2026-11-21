// 连上主机后的主界面：待办、对话、我。窄屏导航在底部，宽屏在左边。

import 'package:flutter/material.dart';

import '../protocol/client.dart';
import 'chat_page.dart';
import 'common.dart';
import 'inbox_page.dart';
import 'me_page.dart';

/// 导航挪到左边的最小宽度，和网页控制台的窄屏断点一样。
const double kWideNav = 720;

class HomePage extends StatefulWidget {
  const HomePage({super.key});

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  /// 0 待办（手机首页，apps-plan Q-6），1 对话，2 我。
  int _tab = 0;

  bool _narrowChat(BuildContext context) => MediaQuery.sizeOf(context).width < kWideChat;

  void _openThread(String threadId) {
    final state = AppScope.read(context);
    state.openThread(threadId);
    setState(() => _tab = 1);
    if (_narrowChat(context)) _pushThread();
  }

  void _newChat() {
    AppScope.read(context).startNewChat();
    if (_narrowChat(context)) _pushThread();
  }

  void _pushThread() => Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => const ThreadScreen()));

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final wide = MediaQuery.sizeOf(context).width >= kWideNav;
    final badge = state.inboxCount;
    Widget inboxIcon(IconData icon) => Badge(isLabelVisible: badge > 0, label: Text('$badge'), child: Icon(icon));
    final pages = [
      InboxPage(onOpenThread: _openThread),
      ChatPage(onOpenThread: _openThread, onNewChat: _newChat),
      const MePage(),
    ];
    const titles = ['待办', '对话', '我'];
    final body = Column(
      children: [
        const ConnectionBanner(),
        Expanded(
          child: IndexedStack(index: _tab, children: pages),
        ),
      ],
    );
    if (wide) {
      return Scaffold(
        body: SafeArea(
          child: Row(
            children: [
              NavigationRail(
                selectedIndex: _tab,
                labelType: NavigationRailLabelType.all,
                onDestinationSelected: (i) => setState(() => _tab = i),
                destinations: [
                  NavigationRailDestination(
                    icon: inboxIcon(Icons.inbox_outlined),
                    selectedIcon: inboxIcon(Icons.inbox),
                    label: Text(titles[0]),
                  ),
                  NavigationRailDestination(
                    icon: const Icon(Icons.chat_bubble_outline),
                    selectedIcon: const Icon(Icons.chat_bubble),
                    label: Text(titles[1]),
                  ),
                  NavigationRailDestination(
                    icon: const Icon(Icons.person_outline),
                    selectedIcon: const Icon(Icons.person),
                    label: Text(titles[2]),
                  ),
                ],
              ),
              const VerticalDivider(width: 1),
              Expanded(child: body),
            ],
          ),
        ),
      );
    }
    return Scaffold(
      appBar: AppBar(title: Text(titles[_tab])),
      body: body,
      floatingActionButton: _tab == 1
          ? FloatingActionButton(tooltip: '新对话', onPressed: _newChat, child: const Icon(Icons.edit))
          : null,
      bottomNavigationBar: NavigationBar(
        selectedIndex: _tab,
        onDestinationSelected: (i) => setState(() => _tab = i),
        destinations: [
          NavigationDestination(
            icon: inboxIcon(Icons.inbox_outlined),
            selectedIcon: inboxIcon(Icons.inbox),
            label: titles[0],
          ),
          NavigationDestination(
            icon: const Icon(Icons.chat_bubble_outline),
            selectedIcon: const Icon(Icons.chat_bubble),
            label: titles[1],
          ),
          NavigationDestination(
            icon: const Icon(Icons.person_outline),
            selectedIcon: const Icon(Icons.person),
            label: titles[2],
          ),
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
