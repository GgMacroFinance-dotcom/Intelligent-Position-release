// 从 Rust 生成协议的 Dart 类型（backlog F-20）。
//
// Rust 那边的协议类型用 ts-rs 导出成 `rust/web/src/types/*.ts`（`cargo test` 时导出，rust CI 检查已提交）。
// ts-rs 的输出很规整，这里读那些 .ts，生成 `lib/src/protocol/wire.dart`：
//   - `type X = string` 之类：typedef；
//   - 几个字符串之一：enum，认不出的值落到 `unknown`；
//   - 对象：不可变的类，带 `fromJson`、`toJson`；
//   - 按某个字段（`type`、`kind`、`state`……）区分的几种对象：sealed 类，每种一个子类，认不出的落到 `XUnknown`；
//   - `number` 一律是 `num`：TS 里分不出整数和小数，由用的地方决定。
// 这样 Rust 的类型一改：rust CI 要求重新导出 .ts，flutter CI 要求重新生成 wire.dart，
// 用到改了的字段的 Dart 代码就编不过。
//
// 用法（在 apps/flutter 下）：
//   dart run tool/gen_protocol.dart          重新生成
//   dart run tool/gen_protocol.dart --check  只检查生成结果是不是最新的，不是就以非零退出（CI 用）

import 'dart:io';

const tsDir = '../../rust/web/src/types';
const outPath = 'lib/src/protocol/wire.dart';

/// 不生成的：`Envelope` 是 `{id} & Request`，请求直接用 `Request`、id 由客户端加；`JsonValue` 就是 `Object?`。
const skip = {'Envelope', 'JsonValue'};

/// Dart 的保留字不能当字段名。
const reserved = {
  'default', 'in', 'is', 'new', 'null', 'class', 'enum', 'extends', 'final', 'for', 'if', 'return', 'switch', //
  'var', 'void', 'while', 'with', 'do', 'else', 'case', 'catch', 'const', 'continue', 'try', 'true', 'false',
};

void main(List<String> args) {
  final check = args.contains('--check');
  final dir = Directory(tsDir);
  if (!dir.existsSync()) {
    stderr.writeln('找不到 $tsDir：要在 apps/flutter 下运行');
    exit(2);
  }
  final files = dir.listSync().whereType<File>().where((f) => f.path.endsWith('.ts')).toList()
    ..sort((a, b) => a.path.compareTo(b.path));
  final decls = <Decl>[];
  for (final f in files) {
    final name = f.uri.pathSegments.last.replaceAll('.ts', '');
    if (skip.contains(name)) continue;
    decls.add(parseFile(name, f.readAsStringSync()));
  }
  final out = format(generate(decls));
  final file = File(outPath);
  if (check) {
    if (!file.existsSync() || file.readAsStringSync() != out) {
      stderr.writeln('$outPath 不是最新的：在 apps/flutter 下运行 dart run tool/gen_protocol.dart，再提交。');
      exit(1);
    }
    stdout.writeln('$outPath 是最新的（${decls.length} 个类型）。');
    return;
  }
  file.writeAsStringSync(out);
  stdout.writeln('生成了 $outPath（${decls.length} 个类型）。');
}

/// 用 `dart format`（行宽 120，和 analysis_options.yaml 一样）排版，这样生成的文件和手写的一样过格式检查。
String format(String src) {
  final tmp = Directory.systemTemp.createTempSync('gen_protocol');
  try {
    final f = File('${tmp.path}/wire.dart')..writeAsStringSync(src);
    final r = Process.runSync(Platform.resolvedExecutable, ['format', '--page-width', '120', f.path]);
    if (r.exitCode != 0) throw StateError('dart format 失败：${r.stderr}');
    return f.readAsStringSync();
  } finally {
    tmp.deleteSync(recursive: true);
  }
}

// ---------- 解析 ts-rs 的输出 ----------

/// 一个导出的类型：`export type Name = <type>;`，前面可以有一段 `/** */` 说明。
class Decl {
  Decl(this.name, this.doc, this.type);
  final String name;
  final String? doc;
  final TsType type;
}

sealed class TsType {}

class TsPrim extends TsType {
  TsPrim(this.name);

  /// string、number、boolean、null。
  final String name;
}

class TsLit extends TsType {
  TsLit(this.value);
  final String value;
}

class TsRef extends TsType {
  TsRef(this.name);
  final String name;
}

class TsArray extends TsType {
  TsArray(this.item);
  final TsType item;
}

/// `{ [key in K]: V }`
class TsMap extends TsType {
  TsMap(this.value);
  final TsType value;
}

class TsField {
  TsField(this.key, this.type, {required this.optional, this.doc});
  final String key;
  final TsType type;
  final bool optional;
  final String? doc;
}

class TsObject extends TsType {
  TsObject(this.fields);
  final List<TsField> fields;
}

class TsUnion extends TsType {
  TsUnion(this.members, this.docs);
  final List<TsType> members;

  /// 每种的说明（ts-rs 把 Rust 枚举每一项的文档注释放在那一项前面）。
  final List<String?> docs;
}

Decl parseFile(String name, String src) {
  final m = RegExp(r'(/\*\*((?:(?!\*/)[\s\S])*)\*/\s*)?export type (\w+) = ([\s\S]*);\s*$').firstMatch(src);
  if (m == null || m.group(3) != name) throw FormatException('$name.ts 不是 ts-rs 的格式');
  final p = _Parser(m.group(4)!);
  final t = p.type();
  p.end();
  return Decl(name, _doc(m.group(2)), t);
}

String? _doc(String? raw) {
  if (raw == null) return null;
  final lines = raw.split('\n').map((l) => l.replaceFirst(RegExp(r'^\s*\*?\s?'), '').trimRight()).toList();
  while (lines.isNotEmpty && lines.first.isEmpty) {
    lines.removeAt(0);
  }
  while (lines.isNotEmpty && lines.last.isEmpty) {
    lines.removeLast();
  }
  return lines.isEmpty ? null : lines.join('\n');
}

class _Parser {
  _Parser(String src) : _t = _tokens(src);

  final List<String> _t;
  var _i = 0;

  static List<String> _tokens(String s) {
    final re = RegExp(r'/\*\*(?:(?!\*/)[\s\S])*\*/|"(?:[^"\\]|\\.)*"|[A-Za-z_][A-Za-z0-9_]*|[{}\[\]()<>|&:,?;]|\s+');
    final out = <String>[];
    var pos = 0;
    for (final m in re.allMatches(s)) {
      if (m.start != pos) throw FormatException('认不出：${s.substring(pos, m.start)}');
      pos = m.end;
      final tok = m.group(0)!;
      if (tok.trim().isNotEmpty) out.add(tok);
    }
    if (pos != s.length) throw FormatException('认不出：${s.substring(pos)}');
    return out;
  }

  String get _peek => _i < _t.length ? _t[_i] : '';
  String _next() => _t[_i++];
  void _expect(String s) {
    final got = _next();
    if (got != s) throw FormatException('要 $s，看到 $got');
  }

  void end() {
    if (_i != _t.length) throw FormatException('多出来：${_t.sublist(_i).join(' ')}');
  }

  String? _comment() => _peek.startsWith('/**') ? _doc(_next().substring(3, _t[_i - 1].length - 2)) : null;

  TsType type() {
    final members = <TsType>[];
    final docs = <String?>[];
    do {
      docs.add(_comment());
      members.add(_primary());
      if (_peek == '&') throw const FormatException('不支持 &（交叉类型）');
    } while (_peek == '|' && _next() == '|');
    return members.length == 1 && docs.single == null ? members.single : TsUnion(members, docs);
  }

  TsType _primary() {
    final tok = _next();
    if (tok == '(') {
      final t = type();
      _expect(')');
      return t;
    }
    if (tok == '{') return _object();
    if (tok.startsWith('"')) return TsLit(tok.substring(1, tok.length - 1));
    if (const {'string', 'number', 'boolean', 'null', 'bigint'}.contains(tok)) {
      return TsPrim(tok == 'bigint' ? 'number' : tok);
    }
    if (tok == 'Array') {
      _expect('<');
      final item = type();
      _expect('>');
      return TsArray(item);
    }
    if (RegExp(r'^[A-Za-z_]\w*$').hasMatch(tok)) return TsRef(tok);
    throw FormatException('认不出类型：$tok');
  }

  TsType _object() {
    if (_peek == '[') {
      // { [key in K]: V }
      _next();
      _next(); // key
      _expect('in');
      type();
      _expect(']');
      if (_peek == '?') _next();
      _expect(':');
      final v = type();
      if (_peek == ',' || _peek == ';') _next();
      _expect('}');
      return TsMap(v);
    }
    final fields = <TsField>[];
    while (true) {
      final doc = _comment();
      if (_peek == '}') {
        _next();
        break;
      }
      var key = _next();
      if (key.startsWith('"')) key = key.substring(1, key.length - 1);
      var optional = false;
      if (_peek == '?') {
        _next();
        optional = true;
      }
      _expect(':');
      final t = type();
      fields.add(TsField(key, t, optional: optional, doc: doc));
      if (_peek == ',' || _peek == ';') _next();
    }
    return TsObject(fields);
  }
}

// ---------- 生成 Dart ----------

/// 每个导出类型生成成什么。
sealed class Kind {}

class AliasKind extends Kind {
  AliasKind(this.type);
  final TsType type;
}

class EnumKind extends Kind {
  EnumKind(this.values, this.docs);
  final List<String> values;
  final List<String?> docs;
}

class ClassKind extends Kind {
  ClassKind(this.fields);
  final List<TsField> fields;
}

/// 一种变体：按 `tag` 字段的值认，或者整个就是一个字符串（`Fallback` 的 `"ask"`）。
class Variant {
  Variant(this.dartName, this.doc, {this.tagValue, this.literal, this.fields = const []});
  final String dartName;
  final String? doc;
  final String? tagValue;
  final String? literal;
  final List<TsField> fields;
}

class SealedKind extends Kind {
  SealedKind(this.tag, this.variants);

  /// 区分用的字段名；只有字符串和单字段对象时为 null。
  final String? tag;
  final List<Variant> variants;
}

Kind classify(Decl d) {
  final t = d.type;
  if (t is TsUnion && t.members.every((m) => m is TsLit)) {
    return EnumKind([for (final m in t.members) (m as TsLit).value], t.docs);
  }
  if (t is TsObject) return ClassKind(t.fields);
  if (t is TsUnion && t.members.every((m) => m is TsObject || m is TsLit)) {
    final objs = t.members.whereType<TsObject>().toList();
    // 找所有对象都有、而且都是字符串字面量的那个字段。
    String? tag;
    if (objs.isNotEmpty) {
      for (final f in objs.first.fields) {
        if (f.type is TsLit && objs.every((o) => o.fields.any((g) => g.key == f.key && g.type is TsLit))) {
          tag = f.key;
          break;
        }
      }
    }
    final variants = <Variant>[];
    for (final (i, m) in t.members.indexed) {
      final doc = t.docs[i];
      if (m is TsLit) {
        variants.add(Variant('${d.name}${pascal(m.value)}', doc, literal: m.value));
      } else if (m is TsObject && tag != null) {
        final v = (m.fields.firstWhere((f) => f.key == tag).type as TsLit).value;
        variants.add(
          Variant(
            '${d.name}${pascal(v)}',
            doc,
            tagValue: v,
            fields: [
              for (final f in m.fields)
                if (f.key != tag) f,
            ],
          ),
        );
      } else if (m is TsObject && m.fields.length == 1) {
        variants.add(Variant('${d.name}${pascal(m.fields.single.key)}', doc, fields: m.fields));
      } else {
        throw FormatException('${d.name}：认不出怎么区分几种情况');
      }
    }
    return SealedKind(tag, variants);
  }
  return AliasKind(t);
}

String pascal(String s) =>
    s.split(RegExp(r'[_\-\s]+')).where((p) => p.isNotEmpty).map((p) => p[0].toUpperCase() + p.substring(1)).join();

String camel(String s) {
  final p = pascal(s);
  final c = p.isEmpty ? s : p[0].toLowerCase() + p.substring(1);
  return reserved.contains(c) ? '${c}Value' : c;
}

class Gen {
  Gen(List<Decl> decls)
    : kinds = {for (final d in decls) d.name: classify(d)},
      decls = {for (final d in decls) d.name: d};

  final Map<String, Kind> kinds;
  final Map<String, Decl> decls;
  final out = StringBuffer();

  /// 别名一路解到底（`ThreadId` → string）；不是别名时原样返回。
  TsType resolve(TsType t) {
    if (t is TsRef && kinds[t.name] is AliasKind) return resolve((kinds[t.name] as AliasKind).type);
    return t;
  }

  /// `T | null` 拆成 (T, true)。
  (TsType, bool) nullable(TsType t) {
    if (t is TsUnion && t.members.length == 2 && t.members.any((m) => m is TsPrim && m.name == 'null')) {
      return (t.members.firstWhere((m) => !(m is TsPrim && m.name == 'null')), true);
    }
    return (t, false);
  }

  String dartType(TsType t) {
    final (inner, isNull) = nullable(t);
    final base = switch (inner) {
      TsPrim(name: 'string') => 'String',
      TsPrim(name: 'number') => 'num',
      TsPrim(name: 'boolean') => 'bool',
      TsPrim() => 'Object?',
      TsRef(name: 'JsonValue') => 'Object?',
      TsRef(:final name) => name,
      TsArray(:final item) => 'List<${dartType(item)}>',
      TsMap(:final value) => 'Map<String, ${dartType(value)}>',
      TsLit() => 'String',
      TsObject() || TsUnion() => throw FormatException('字段里不支持内嵌的对象或联合类型：$inner'),
    };
    return isNull && !base.endsWith('?') ? '$base?' : base;
  }

  bool isNullable(TsType t) => dartType(t).endsWith('?');

  /// 把 JSON 值 `e` 解成类型 `t`。
  String decode(TsType t, String e) {
    final (inner, isNull) = nullable(t);
    final body = _decode(inner, e);
    return isNull && body != e ? '$e == null ? null : $body' : body;
  }

  String _decode(TsType t, String e) {
    final r = resolve(t);
    return switch (r) {
      TsPrim(name: 'string') || TsLit() => '$e as String',
      TsPrim(name: 'number') => '$e as num',
      TsPrim(name: 'boolean') => '$e as bool',
      TsPrim() => e,
      TsRef(name: 'JsonValue') => e,
      TsRef(:final name) when kinds[name] is EnumKind => '$name.fromJson($e)',
      TsRef(:final name)
          when kinds[name] is SealedKind && (kinds[name] as SealedKind).variants.any((v) => v.literal != null) =>
        '$name.fromJson($e)',
      TsRef(:final name) => '$name.fromJson($e as Map<String, Object?>)',
      TsArray(:final item) => '[for (final x in $e as List<Object?>) ${_paren(decode(item, 'x'))}]',
      TsMap(:final value) =>
        '{for (final x in ($e as Map<String, Object?>).entries) x.key: ${_paren(decode(value, 'x.value'))}}',
      _ => throw FormatException('不支持：$r'),
    };
  }

  /// 三元表达式放进别的表达式里要加括号。
  String _paren(String s) => s.contains(' ? ') ? '($s)' : s;

  /// 把 Dart 值 `e`（已知不为 null）转回 JSON。
  String encode(TsType t, String e) {
    final (inner, _) = nullable(t);
    final r = resolve(inner);
    return switch (r) {
      TsPrim() || TsLit() || TsRef(name: 'JsonValue') => e,
      TsRef() => '$e.toJson()',
      TsArray(:final item) => _isPlain(item) ? e : '[for (final x in $e) ${encodeMaybeNull(item, 'x')}]',
      TsMap(:final value) =>
        _isPlain(value) ? e : '{for (final x in $e.entries) x.key: ${encodeMaybeNull(value, 'x.value')}}',
      _ => throw FormatException('不支持：$r'),
    };
  }

  String encodeMaybeNull(TsType t, String e) {
    final enc = encode(t, e);
    if (enc == e || !isNullable(t)) return enc;
    return '$e == null ? null : ${enc.replaceFirst('$e.', '$e!.')}';
  }

  bool _isPlain(TsType t) {
    final r = resolve(nullable(t).$1);
    return r is TsPrim || r is TsLit || (r is TsRef && r.name == 'JsonValue');
  }

  void doc(String? d, String indent) {
    if (d == null) return;
    for (final l in d.split('\n')) {
      out.writeln(l.isEmpty ? '$indent///' : '$indent/// $l');
    }
  }

  String run() {
    out.writeln('// 由 tool/gen_protocol.dart 从 rust/web/src/types/*.ts（ts-rs 从 Rust 导出的）生成，不要手改。');
    out.writeln('// 改了 Rust 的协议类型：先在 rust/ 下 cargo test 重新导出 .ts，再在 apps/flutter 下');
    out.writeln('// dart run tool/gen_protocol.dart。`number` 一律是 `num`，整数还是小数由用的地方决定。');
    out.writeln();
    out.writeln('// ignore_for_file: unnecessary_cast');
    for (final name in decls.keys) {
      out.writeln();
      final d = decls[name]!;
      switch (kinds[name]!) {
        case AliasKind(:final type):
          doc(d.doc, '');
          out.writeln('typedef $name = ${dartType(type)};');
        case EnumKind(:final values, :final docs):
          _enum(name, d.doc, values, docs);
        case ClassKind(:final fields):
          doc(d.doc, '');
          _class(name, null, fields, const {});
        case final SealedKind s:
          _sealed(name, d.doc, s);
      }
    }
    return out.toString();
  }

  void _enum(String name, String? docText, List<String> values, List<String?> docs) {
    doc(docText, '');
    out.writeln('enum $name {');
    for (final (i, v) in values.indexed) {
      doc(docs[i], '  ');
      out.writeln("  ${camel(v)}('$v'),");
    }
    out.writeln('');
    out.writeln('  /// 这个版本还不认识的值（主机比 App 新）。');
    out.writeln("  unknown('');");
    out.writeln();
    out.writeln('  const $name(this.wire);');
    out.writeln();
    out.writeln('  /// JSON 里的写法。');
    out.writeln('  final String wire;');
    out.writeln();
    out.writeln(
      '  static $name fromJson(Object? v) => values.firstWhere((e) => e != unknown && e.wire == v, orElse: () => unknown);',
    );
    out.writeln();
    out.writeln('  String toJson() => wire;');
    out.writeln('}');
  }

  /// 一个类：`base` 不为 null 时是 sealed 的子类，`extra` 是 toJson 时固定带上的字段（区分用的字段）。
  void _class(String name, String? base, List<TsField> fields, Map<String, String> extra) {
    out.writeln(base == null ? 'final class $name {' : 'final class $name extends $base {');
    final params = [
      for (final f in fields)
        isNullable(f.type) || f.optional ? 'this.${camel(f.key)}' : 'required this.${camel(f.key)}',
    ];
    out.writeln(params.isEmpty ? '  const $name();' : '  const $name({${params.join(', ')}});');
    out.writeln();
    if (fields.isEmpty) {
      out.writeln('  factory $name.fromJson(Map<String, Object?> j) => const $name();');
    } else {
      out.writeln('  factory $name.fromJson(Map<String, Object?> j) => $name(');
      for (final f in fields) {
        final e = "j['${f.key}']";
        final v = f.optional && !isNullable(f.type)
            ? '$e == null ? null : ${_paren(decode(f.type, e))}'
            : decode(f.type, e);
        out.writeln('    ${camel(f.key)}: $v,');
      }
      out.writeln('  );');
    }
    for (final f in fields) {
      out.writeln();
      doc(f.doc, '  ');
      var t = dartType(f.type);
      if (f.optional && !t.endsWith('?')) t = '$t?';
      out.writeln('  final $t ${camel(f.key)};');
    }
    out.writeln();
    if (base != null) out.writeln('  @override');
    final entries = [for (final e in extra.entries) "'${e.key}': '${e.value}'", for (final f in fields) _entry(f)];
    out.writeln('  Map<String, Object?> toJson() => {${entries.join(', ')}};');
    out.writeln('}');
  }

  String _entry(TsField f) {
    final n = camel(f.key);
    final optional = f.optional || isNullable(f.type);
    if (!optional) return "'${f.key}': ${encode(f.type, n)}";
    final enc = encode(f.type, n);
    // 可省的字段（`?`）是 null 时不写；`| null` 的字段照写 null，和 serde 一样。
    if (f.optional) return "'${f.key}': ?${enc == n ? n : '$n?.${enc.substring(n.length + 1)}'}";
    return "'${f.key}': ${enc == n ? n : '$n?.${enc.substring(n.length + 1)}'}";
  }

  void _sealed(String name, String? docText, SealedKind s) {
    final hasLiteral = s.variants.any((v) => v.literal != null);
    doc(docText, '');
    out.writeln('sealed class $name {');
    out.writeln('  const $name();');
    out.writeln();
    out.writeln(
      hasLiteral ? '  factory $name.fromJson(Object? j) {' : '  factory $name.fromJson(Map<String, Object?> j) {',
    );
    if (hasLiteral) {
      for (final v in s.variants.where((v) => v.literal != null)) {
        out.writeln("    if (j == '${v.literal}') return const ${v.dartName}();");
      }
      out.writeln('    if (j is! Map<String, Object?>) return ${name}Unknown(j);');
    }
    if (s.tag != null) {
      out.writeln("    return switch (j['${s.tag}']) {");
      for (final v in s.variants.where((v) => v.tagValue != null)) {
        out.writeln("      '${v.tagValue}' => ${v.dartName}.fromJson(j),");
      }
      out.writeln('      _ => ${name}Unknown(j),');
      out.writeln('    };');
    } else {
      for (final v in s.variants.where((v) => v.literal == null)) {
        out.writeln("    if (j.containsKey('${v.fields.single.key}')) return ${v.dartName}.fromJson(j);");
      }
      out.writeln('    return ${name}Unknown(j);');
    }
    out.writeln('  }');
    out.writeln();
    out.writeln('  Object? toJson();');
    out.writeln('}');
    for (final v in s.variants) {
      out.writeln();
      doc(v.doc, '');
      if (v.literal != null) {
        out.writeln('final class ${v.dartName} extends $name {');
        out.writeln('  const ${v.dartName}();');
        out.writeln();
        out.writeln('  @override');
        out.writeln("  Object? toJson() => '${v.literal}';");
        out.writeln('}');
      } else {
        _class(v.dartName, name, v.fields, {if (v.tagValue != null) s.tag!: v.tagValue!});
      }
    }
    out.writeln();
    out.writeln('/// 这个版本还不认识的$name（主机比 App 新），原样留着。');
    out.writeln('final class ${name}Unknown extends $name {');
    out.writeln('  const ${name}Unknown(this.json);');
    out.writeln();
    out.writeln('  final Object? json;');
    out.writeln();
    out.writeln('  @override');
    out.writeln('  Object? toJson() => json;');
    out.writeln('}');
  }
}

String generate(List<Decl> decls) => Gen(decls).run();
