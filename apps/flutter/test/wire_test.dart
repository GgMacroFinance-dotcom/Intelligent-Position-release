// 从 Rust 生成的协议类型（F-20，lib/src/protocol/wire.dart）：几种生成方式各挑一个，看 JSON 进出是不是和 serde 一样。

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/protocol/wire.dart';

Map<String, Object?> json(String s) => jsonDecode(s) as Map<String, Object?>;

void main() {
  test('按字段区分的几种（sealed）：认得的解成子类，认不出的原样留着', () {
    final b = Body.fromJson(json('{"kind":"tool_call","call_id":"c1","name":"search","input":{"q":"CPI"}}'));
    expect(b, isA<BodyToolCall>());
    b as BodyToolCall;
    expect((b.callId, b.name), ('c1', 'search'));
    expect(b.input, {'q': 'CPI'});
    expect(b.toJson(), {
      'kind': 'tool_call',
      'call_id': 'c1',
      'name': 'search',
      'input': {'q': 'CPI'},
    });

    final later = json('{"kind":"image","url":"x"}');
    final u = Body.fromJson(later);
    expect(u, isA<BodyUnknown>());
    expect(u.toJson(), later);

    // 只有区分字段的那几种。
    expect(ApprovalState.fromJson(json('{"state":"pending"}')), isA<ApprovalStatePending>());
    expect(const ApprovalStatePending().toJson(), {'state': 'pending'});
  });

  test('字符串和对象混着的（Fallback）', () {
    expect(Fallback.fromJson('ask'), isA<FallbackAsk>());
    expect(const FallbackAsk().toJson(), 'ask');
    final r = Fallback.fromJson({'role': 'ops'});
    expect((r as FallbackRole).role, 'ops');
    expect(r.toJson(), {'role': 'ops'});
  });

  test('几个字符串之一（enum）：认不出的落到 unknown', () {
    expect(TaskStatus.fromJson('in_progress'), TaskStatus.inProgress);
    expect(TaskStatus.inProgress.toJson(), 'in_progress');
    expect(TaskStatus.fromJson('archived'), TaskStatus.unknown);
  });

  test('可省的字段（?）是 null 时不写，可为 null 的字段照写 null', () {
    expect(const RequestHistory(threadId: 't1', limit: 50).toJson(), {
      'type': 'history',
      'thread_id': 't1',
      'limit': 50,
    });
    expect(const NewPairing(code: '123456', name: '手机', expiresMs: 1).toJson(), {
      'code': '123456',
      'name': '手机',
      'expires_ms': 1,
    });
    const slot = ModelSlot(defaultValue: 'openai/gpt-6-luna');
    expect(slot.toJson(), {'default': 'openai/gpt-6-luna', 'escalate': null});
    expect(ModelSlot.fromJson(slot.toJson()).defaultValue, 'openai/gpt-6-luna');
  });

  test('嵌套、列表、按岗位的表都能来回转', () {
    final src = json('''
      {"date":"2026-11-21","roles":{"macro":0.25},"routing_usd":0.01,
       "tokens":{"macro":{"prompt_tokens":10,"completion_tokens":2,"cached_tokens":8,"cache_write_tokens":0}},
       "routing_tokens":{"prompt_tokens":1,"completion_tokens":1,"cached_tokens":0,"cache_write_tokens":0}}''');
    final s = SpendToday.fromJson(src);
    expect(s.roles['macro'], 0.25);
    expect(s.tokens['macro']!.cachedTokens, 8);
    expect(jsonDecode(jsonEncode(s.toJson())), src);

    final frame = ServerFrame.fromJson(
      json('''
      {"type":"event","seq":7,"event":{"type":"message","message":{"id":"m1","thread_id":"t1","at_ms":5,
       "author":{"kind":"human","name":"张三","via":{"channel":"slack","user_id":"U1"}},
       "body":{"kind":"text","text":"你好"}}}}'''),
    );
    final msg = ((frame as ServerFrameEvent).event as RuntimeEventMessage).message;
    expect((msg.author as AuthorHuman).via!.channel, 'slack');
    expect((msg.body as BodyText).text, '你好');
    expect(msg.notice, isNull);
    expect(msg.toJson().containsKey('notice'), isFalse);
  });
}
