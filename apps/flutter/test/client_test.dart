import 'dart:async';
import 'dart:io';

import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/protocol/client.dart';
import 'package:ip_app/src/protocol/models.dart';

import 'fake_host.dart';

IpClient clientFor(FakeHost h, {String? token = 'ipd_secret'}) =>
    IpClient(wsUri: Uri.parse('wss://ip.example.com/ws'), token: token, connector: h.connect);

void main() {
  test('收到 hello 才算连上，令牌放在请求头里不放地址', () async {
    final h = FakeHost();
    final c = clientFor(h);
    final statuses = <ConnStatus>[];
    c.statusChanges.listen(statuses.add);
    c.connect();
    expect(c.status, ConnStatus.connecting);
    await pumpEventQueue();
    expect(c.status, ConnStatus.open);
    expect(statuses, [ConnStatus.connecting, ConnStatus.open]);
    final (uri, headers) = h.connects.single;
    expect(uri.toString(), 'wss://ip.example.com/ws');
    expect(headers['Authorization'], 'Bearer ipd_secret');
    expect(c.epoch, 'e1');
    await c.dispose();
  });

  test('请求和回复按 id 配对，错误回复变成 RequestError', () async {
    final h = FakeHost()
      ..handlers['list_agents'] = (_) => [
        {'id': 'macro-1', 'role': 'macro', 'name': '宏观研究员'},
      ];
    final c = clientFor(h)..connect();
    await pumpEventQueue();
    final agents = await c.listAgents();
    expect(agents.single.name, '宏观研究员');
    await expectLater(c.listRoles(), throwsA(isA<RequestError>()));
    expect(h.requests.map((r) => r['id']), [1, 2]);
    await c.dispose();
  });

  test('send 只带给了的字段', () async {
    final h = FakeHost()
      ..handlers['send'] = (req) => {
        'message': message('m9', 't9', human: '张三的手机', text: req['text'] as String),
        'agent_id': 'macro-1',
        'decision': {
          'kind': 'routed',
          'by': 'rule',
          'rule_id': 'r1',
          'role': 'macro',
          'agent': null,
          'escalate': false,
          'reason': '命中规则 r1',
        },
        'approval_id': null,
      };
    final c = clientFor(h)..connect();
    await pumpEventQueue();
    final res = await c.send('CPI 怎么样');
    expect(h.requests.single, {'id': 1, 'type': 'send', 'text': 'CPI 怎么样'});
    expect(res.agentId, 'macro-1');
    expect(res.routedReason, '命中规则 r1');
    await c.send('接着说', agentId: 'macro-1', threadId: 't9');
    expect(h.requests.last['agent_id'], 'macro-1');
    expect(h.requests.last['thread_id'], 't9');
    await c.dispose();
  });

  test('改任务状态、列报告、读报告的请求', () async {
    final task = {
      'id': 'k1',
      'title': 't',
      'description': '',
      'status': 'in_progress',
      'created_by': {'kind': 'system'},
      'created_at_ms': 1,
      'updated_at_ms': 2,
    };
    final h = FakeHost()
      ..handlers.addAll({
        'update_task': (_) => task,
        'list_reports': (_) => [
          {'agent_id': 'macro-1', 'path': 'reports/a.md', 'bytes': 3, 'modified_ms': 1},
        ],
        'read_report': (req) => {
          'agent_id': req['agent_id'],
          'path': req['path'],
          'text': '# A',
          'modified_ms': 1,
          'truncated': false,
        },
      });
    final c = clientFor(h)..connect();
    await pumpEventQueue();
    expect((await c.updateTask('k1', status: 'in_progress')).status, 'in_progress');
    expect(h.requests.last, {'id': 1, 'type': 'update_task', 'task_id': 'k1', 'status': 'in_progress'});
    expect((await c.listReports()).single.agentId, 'macro-1');
    expect(h.requests.last, {'id': 2, 'type': 'list_reports', 'limit': 200});
    expect((await c.readReport('macro-1', 'reports/a.md')).text, '# A');
    expect(h.requests.last['path'], 'reports/a.md');
    await c.dispose();
  });

  test('事件按序号记下，断开后带 since 和 epoch 重连', () async {
    final h = FakeHost();
    final c = clientFor(h)..connect();
    final events = <RuntimeEvent>[];
    c.events.listen(events.add);
    await pumpEventQueue();
    h.event({'type': 'turn_started', 'agent_id': 'macro-1', 'thread_id': 't1'});
    h.event({'type': 'delta', 'agent_id': 'macro-1', 'thread_id': 't1', 'text': 'CPI'});
    await pumpEventQueue();
    expect(c.seq, 2);
    expect(events, [isA<TurnStarted>(), isA<Delta>()]);

    h.resume = true;
    await h.last.drop();
    await pumpEventQueue();
    expect(c.status, ConnStatus.closed);
    await Future<void>.delayed(const Duration(milliseconds: 600));
    await pumpEventQueue();
    expect(c.status, ConnStatus.open);
    expect(c.resumed, isTrue);
    expect(h.connects.last.$1.queryParameters, {'since': '2', 'epoch': 'e1'});
    await c.dispose();
  });

  test('断开时没回的请求都失败', () async {
    final h = FakeHost()..silent.add('list_threads');
    final c = clientFor(h)..connect();
    await pumpEventQueue();
    final pending = expectLater(c.listThreads(), throwsA(isA<RequestError>()));
    await h.last.drop();
    await pending;
    await c.dispose();
  });

  test('重连间隔从 500 毫秒起翻倍，最多 8 秒；连上后从头算', () {
    fakeAsync((async) {
      final h = FakeHost();
      final starts = <Duration>[];
      // 先连不上 6 次。
      var failures = 6;
      final real = h.connect;
      Future<FrameSocket> flaky(Uri u, Map<String, String> hd) {
        if (failures-- > 0) return Future.error(const SocketException('断网'));
        return real(u, hd);
      }

      final c = IpClient(wsUri: Uri.parse('wss://x/ws'), token: 't', connector: flaky);
      c.statusChanges.listen((s) {
        if (s == ConnStatus.connecting) starts.add(async.elapsed);
      });
      c.connect();
      async.elapse(const Duration(seconds: 40));
      final gaps = [for (var i = 1; i < starts.length; i++) (starts[i] - starts[i - 1]).inMilliseconds];
      expect(gaps, [500, 1000, 2000, 4000, 8000, 8000]);
      expect(c.status, ConnStatus.open);
      // 连上后断开，又从 500 毫秒起。
      h.last.drop();
      async.flushMicrotasks();
      final before = starts.length;
      async.elapse(const Duration(milliseconds: 499));
      expect(starts.length, before);
      async.elapse(const Duration(milliseconds: 2));
      expect(starts.length, before + 1);
      c.close();
    });
  });

  test('主机回 401 时不再重连，告诉人要重新配对', () async {
    final h = FakeHost()..failNext = const WebSocketException('not upgraded', 401);
    final c = clientFor(h)..connect();
    await pumpEventQueue();
    expect(c.status, ConnStatus.closed);
    expect(c.rejected?.status, 401);
    await Future<void>.delayed(const Duration(milliseconds: 700));
    expect(h.connects, hasLength(1));
    await c.dispose();
  });

  test('alert、dismiss、lagged 和认不出的帧', () async {
    final h = FakeHost();
    final c = clientFor(h)..connect();
    final alerts = <Alert>[];
    final tags = <String>[];
    var lagged = 0;
    c.alerts.listen(alerts.add);
    c.dismissals.listen(tags.add);
    c.lagged.listen((_) => lagged++);
    await pumpEventQueue();
    h.last
      ..push({
        'type': 'alert',
        'alert': {
          'kind': 'approval',
          'level': 'needs_me',
          'title': '这条消息该交给谁？',
          'body': '',
          'tag': 'approval:a1',
          'at_ms': 1,
        },
      })
      ..push({'type': 'dismiss', 'tag': 'approval:a1'})
      ..push({'type': 'lagged', 'skipped': 3})
      ..push({'type': 'something_new', 'x': 1})
      ..push({
        'type': 'event',
        'seq': 1,
        'event': {'type': 'brand_new_event'},
      });
    await pumpEventQueue();
    expect(alerts.single.tag, 'approval:a1');
    expect(tags, ['approval:a1']);
    expect(lagged, 1);
    expect(c.status, ConnStatus.open);
    expect(c.seq, 1);
    await c.dispose();
  });

  test('协议版本不一致时标出来', () async {
    final h = FakeHost()..autoHello = false;
    final c = clientFor(h)..connect();
    await pumpEventQueue();
    h.last.push({'type': 'hello', 'protocol': kProtocol + 1, 'epoch': 'e', 'seq': 0, 'resumed': false});
    await pumpEventQueue();
    expect(c.outdated, isTrue);
    await c.dispose();
  });

  test('没令牌时不带 Authorization', () async {
    final h = FakeHost();
    final c = clientFor(h, token: null)..connect();
    await pumpEventQueue();
    expect(h.connects.single.$2, isEmpty);
    await c.dispose();
  });

  test('连着的时候 reconnectNow 什么都不做；断着时马上连', () async {
    final h = FakeHost();
    final c = IpClient(
      wsUri: Uri.parse('wss://x/ws'),
      token: 't',
      connector: h.connect,
      firstRetry: const Duration(seconds: 30),
    )..connect();
    await pumpEventQueue();
    c.reconnectNow();
    expect(h.connects, hasLength(1));
    await h.last.drop();
    await pumpEventQueue();
    expect(c.status, ConnStatus.closed);
    c.reconnectNow();
    await pumpEventQueue();
    expect(h.connects, hasLength(2));
    expect(c.status, ConnStatus.open);
    unawaited(c.dispose());
  });
}
