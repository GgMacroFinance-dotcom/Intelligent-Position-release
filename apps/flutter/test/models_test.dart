// 协议类型的解析。样例照 rust/web/src/types/ 里 ts-rs 导出的类型写。

import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/protocol/models.dart';

import 'fake_host.dart' as fake;

void main() {
  test('消息的几种内容和作者', () {
    final m = Message.fromJson(fake.message('m1', 't1', agent: 'macro-1', text: '# CPI'));
    expect(m.author, isA<AgentAuthor>().having((a) => a.agentId, 'agentId', 'macro-1'));
    expect(m.body, isA<TextBody>().having((b) => b.text, 'text', '# CPI'));

    Body body(Map<String, dynamic> j) => Body.fromJson(j);
    expect(
      body({
        'kind': 'tool_call',
        'call_id': 'c1',
        'name': 'get_series',
        'input': {'id': 'CPIAUCSL'},
      }),
      isA<ToolCallBody>().having((b) => b.name, 'name', 'get_series'),
    );
    expect(
      body({'kind': 'tool_result', 'call_id': 'c1', 'ok': false, 'output': '超时'}),
      isA<ToolResultBody>().having((b) => b.ok, 'ok', false),
    );
    expect(body({'kind': 'summary', 'text': '前情', 'covers_through': 'm0'}), isA<SummaryBody>());
    expect(body({'kind': 'image'}), isA<UnknownBody>());

    final human = Author.fromJson({
      'kind': 'human',
      'name': '张三',
      'via': {'channel': 'slack', 'user_id': 'U1'},
    });
    expect(human, isA<HumanAuthor>().having((a) => a.channel, 'channel', 'slack'));
    expect(Author.fromJson({'kind': 'system'}), isA<SystemAuthor>());
  });

  test('一轮结束的事件带用量和费用', () {
    final e = RuntimeEvent.fromJson({
      'type': 'turn_finished',
      'agent_id': 'macro-1',
      'thread_id': 't1',
      'steps': 3,
      'usage': {'prompt_tokens': 1000, 'completion_tokens': 200, 'cached_tokens': 800, 'cache_write_tokens': 0},
      'cost_usd': 0.0123,
      'elapsed_ms': 4200,
    });
    expect(
      e,
      isA<TurnFinished>().having((t) => t.usage.cachedTokens, 'cached', 800).having((t) => t.costUsd, 'cost', 0.0123),
    );
    // 旧数据里没有 cache_write_tokens。
    expect(Usage.fromJson({'prompt_tokens': 1, 'completion_tokens': 2, 'cached_tokens': 0}).cacheWriteTokens, 0);
  });

  test('审批卡：待决定和已选定', () {
    final a = Approval.fromJson(fake.approval('a1', kind: 'agent_question'));
    expect(a.pending, isTrue);
    expect(a.isAgentQuestion, isTrue);
    expect(a.subjectAgentId, 'macro-1');
    expect(a.options.map((o) => o.label), ['宏观研究员', '协调员']);
    final r = Approval.fromJson(fake.approval('a2', pending: false));
    expect(r.resolution?.by, '张三的手机');
    expect(
      Approval.fromJson({
        ...fake.approval('a3'),
        'subject': {'kind': 'question'},
      }).subjectThreadId,
      isNull,
    );
  });

  test('任务、设备和 who_am_i', () {
    final t = Task.fromJson({
      'id': 'k1',
      'title': '写 CPI 周报',
      'description': '',
      'status': 'blocked',
      'role': 'macro',
      'assignee': 'macro-1',
      'thread_id': 't2',
      'origin_thread_id': null,
      'result': '取不到数据',
      'depth': 1,
      'created_by': {'kind': 'system'},
      'created_at_ms': 1,
      'updated_at_ms': 2,
      'due_ms': null,
    });
    expect(t.status, 'blocked');
    expect(t.result, '取不到数据');
    expect(WhoAmI.fromJson({'device': null, 'scope': 'admin'}).scope, Scope.admin);
    expect(Scope.parse('daily').label, '日常');
  });

  test('认不出的事件和帧不抛错', () {
    expect(RuntimeEvent.fromJson({'type': 'brand_new'}), isA<UnknownEvent>());
    expect(ServerFrame.fromJson({'type': 'brand_new'}), isA<UnknownFrame>());
  });
}
