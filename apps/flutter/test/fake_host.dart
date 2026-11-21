// 测试用的假主机：替代 WebSocket，按请求类型回复，能推事件、断开连接。

import 'dart:async';
import 'dart:convert';

import 'package:ip_app/src/protocol/client.dart';

typedef Json = Map<String, dynamic>;

class FakeSocket implements FrameSocket {
  FakeSocket(this.host);

  final FakeHost host;
  final _in = StreamController<dynamic>();
  bool closed = false;

  @override
  Stream<dynamic> get frames => _in.stream;

  @override
  void send(String text) {
    final req = jsonDecode(text) as Json;
    host.requests.add(req);
    if (host.silent.contains(req['type'])) return;
    final handler = host.handlers[req['type']];
    if (handler == null) {
      push({'type': 'reply', 'id': req['id'], 'ok': false, 'error': '假主机不认 ${req['type']}'});
      return;
    }
    try {
      push({'type': 'reply', 'id': req['id'], 'ok': true, 'data': handler(req)});
    } catch (e) {
      push({'type': 'reply', 'id': req['id'], 'ok': false, 'error': '$e'});
    }
  }

  void push(Json frame) {
    if (!closed) _in.add(jsonEncode(frame));
  }

  /// 主机那边断开。
  Future<void> drop() async {
    closed = true;
    await _in.close();
  }

  bool closeCalled = false;

  @override
  Future<void> close() {
    closeCalled = true;
    return host.closeHook?.call() ?? drop();
  }
}

class FakeHost {
  /// 每次连接的地址和请求头。
  final connects = <(Uri, Map<String, String>)>[];
  final sockets = <FakeSocket>[];
  final requests = <Json>[];
  final handlers = <String, Object? Function(Json req)>{};

  /// 这些类型的请求不回复。
  final silent = <String>{};

  /// 客户端关连接时调用；给了就用它的结果代替真的断开（模拟不回关闭帧的主机）。
  Future<void> Function()? closeHook;

  /// 下一次连接要抛的错；null 时正常连上。
  Object? failNext;

  /// 连上后发的 hello。
  String epoch = 'e1';
  int seq = 0;
  bool resume = false;
  bool autoHello = true;

  FakeSocket get last => sockets.last;

  Future<FrameSocket> connect(Uri uri, Map<String, String> headers) async {
    connects.add((uri, headers));
    final err = failNext;
    if (err != null) {
      failNext = null;
      throw err;
    }
    final s = FakeSocket(this);
    sockets.add(s);
    if (autoHello) {
      scheduleMicrotask(
        () => s.push({'type': 'hello', 'protocol': kProtocol, 'epoch': epoch, 'seq': seq, 'resumed': resume}),
      );
    }
    return s;
  }

  /// 推一个事件，序号自动加一。
  void event(Json event) => last.push({'type': 'event', 'seq': ++seq, 'event': event});
}

Json message(String id, String thread, {String? agent, String? human, String text = '你好', int at = 1000}) => {
  'id': id,
  'thread_id': thread,
  'at_ms': at,
  'author': agent != null
      ? {'kind': 'agent', 'agent_id': agent}
      : human != null
      ? {'kind': 'human', 'name': human}
      : {'kind': 'system'},
  'body': {'kind': 'text', 'text': text},
};

Json approval(String id, {String kind = 'route_message', bool pending = true, String thread = 't1'}) => {
  'id': id,
  'question': '这条消息该交给谁？',
  'options': [
    {'label': '宏观研究员', 'consequence': '交给宏观岗位'},
    {'label': '协调员', 'consequence': '交给协调员'},
  ],
  'recommended': 0,
  'reason': '语义匹配把握不够',
  'subject': kind == 'agent_question'
      ? {'kind': 'agent_question', 'agent_id': 'macro-1', 'thread_id': thread}
      : {'kind': 'route_message', 'message_id': 'm1', 'thread_id': thread, 'text': '最近 CPI 怎么样', 'roles': []},
  'state': pending ? {'state': 'pending'} : {'state': 'resolved', 'choice': 0, 'by': '张三的手机', 'at_ms': 2000},
  'created_at_ms': 1000,
};

/// 一套能让 App 加载起来的回复。
void installDefaults(FakeHost h) {
  h.handlers.addAll({
    'who_am_i': (_) => {
      'device': {'id': 'd1', 'name': '张三的手机', 'scope': 'daily', 'paired_at_ms': 1, 'last_seen_ms': 1, 'online': true},
      'scope': 'daily',
    },
    'list_roles': (_) => [
      {
        'id': 'macro',
        'name': '宏观研究',
        'description': '宏观研报',
        'model': {'default': 'fake/m', 'escalate': null},
        'budget_per_day_usd': null,
      },
    ],
    'list_agents': (_) => [
      {'id': 'macro-1', 'role': 'macro', 'name': '宏观研究员'},
    ],
    'list_threads': (_) => [
      {'thread_id': 't1', 'last_at_ms': 1000, 'message_count': 1, 'preview': '最近 CPI 怎么样'},
    ],
    'list_rules': (_) => {
      'spec': {'rules': <Object>[]},
      'from_console': false,
    },
    'list_approvals': (_) => [approval('a1')],
    'list_tasks': (_) => <Object>[],
    'list_reports': (_) => <Object>[],
    'spend_today': (_) => {
      'date': '2026-11-21',
      'roles': {'macro': 0.0123},
      'routing_usd': 0.0004,
      'tokens': {
        'macro': {'prompt_tokens': 10000, 'completion_tokens': 800, 'cached_tokens': 8000, 'cache_write_tokens': 0},
      },
      'routing_tokens': {'prompt_tokens': 500, 'completion_tokens': 20, 'cached_tokens': 0, 'cache_write_tokens': 0},
    },
    'history': (req) => [message('m1', req['thread_id'] as String, human: '张三的手机', text: '最近 CPI 怎么样')],
  });
}
