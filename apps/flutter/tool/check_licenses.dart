// 依赖许可检查：pubspec.lock 里从 pub.dev 来的每个包（含间接依赖、开发依赖），
// 许可都要和 Apache-2.0 兼容（CLAUDE.md 第 7 条）。和 Rust 那边的 cargo-deny（rust/deny.toml）对应。
//
// 用法：先 `flutter pub get`，再在 apps/flutter 下运行 `dart run tool/check_licenses.dart`。
// 认不出或不在允许清单里的，打印出来并以非零退出。Flutter SDK 自带的包（source: sdk）是 BSD-3-Clause，不逐个查。

import 'dart:convert';
import 'dart:io';

/// 允许的许可。
const allowed = {'Apache-2.0', 'MIT', 'BSD-2-Clause', 'BSD-3-Clause'};

/// 按 LICENSE 正文认许可。一个文件里有几种许可时（比如主体 BSD、附带的第三方代码 MIT）都要认出来。
Set<String> classify(String text) {
  final t = text.replaceAll(RegExp(r'\s+'), ' ');
  final found = <String>{};
  if (t.contains('Apache License') && t.contains('Version 2.0')) found.add('Apache-2.0');
  if (t.contains('Permission is hereby granted, free of charge')) found.add('MIT');
  if (t.contains('Redistribution and use in source and binary forms')) {
    found.add(
      RegExp(r'Neither the name|names of its contributors may be used', caseSensitive: false).hasMatch(t)
          ? 'BSD-3-Clause'
          : 'BSD-2-Clause',
    );
  }
  for (final (name, marker) in [
    ('GPL', 'GNU GENERAL PUBLIC LICENSE'),
    ('LGPL', 'GNU LESSER GENERAL PUBLIC LICENSE'),
    ('MPL-2.0', 'Mozilla Public License'),
    ('PolyForm', 'PolyForm'),
  ]) {
    if (t.toUpperCase().contains(marker.toUpperCase())) found.add(name);
  }
  return found;
}

Future<void> main() async {
  final lock = File('pubspec.lock').readAsStringSync();
  final config = jsonDecode(File('.dart_tool/package_config.json').readAsStringSync()) as Map<String, dynamic>;
  final roots = {
    for (final p in (config['packages'] as List).cast<Map<String, dynamic>>())
      p['name'] as String: Uri.parse(p['rootUri'] as String),
  };
  final configDir = Directory('.dart_tool').absolute.uri;

  // pubspec.lock 是 YAML，这里只要包名和来源，按缩进读就够了。
  final sources = <String, String>{};
  String? current;
  for (final line in const LineSplitter().convert(lock)) {
    final name = RegExp(r'^  ([a-z0-9_]+):$').firstMatch(line);
    if (name != null) current = name.group(1);
    final source = RegExp(r'^    source: (\w+)').firstMatch(line);
    if (source != null && current != null) sources[current] = source.group(1)!;
  }

  final problems = <String>[];
  final counts = <String, int>{};
  for (final MapEntry(key: pkg, value: source) in sources.entries) {
    if (source == 'sdk') continue;
    final root = roots[pkg];
    if (root == null) {
      problems.add('$pkg：package_config.json 里没有，先运行 flutter pub get');
      continue;
    }
    final dir = Directory.fromUri(configDir.resolveUri(root));
    final files = [
      'LICENSE',
      'LICENSE.md',
      'LICENSE.txt',
      'COPYING',
    ].map((n) => File('${dir.path}/$n')).where((f) => f.existsSync());
    if (files.isEmpty) {
      problems.add('$pkg：没有 LICENSE 文件');
      continue;
    }
    final found = classify(files.first.readAsStringSync());
    final bad = found.difference(allowed);
    if (found.isEmpty || bad.isNotEmpty) {
      problems.add('$pkg：${found.isEmpty ? '认不出许可' : '许可不在允许清单里：${bad.join('、')}'}（${files.first.path}）');
      continue;
    }
    for (final l in found) {
      counts[l] = (counts[l] ?? 0) + 1;
    }
  }

  final summary = counts.entries.map((e) => '${e.key} ${e.value} 个').join('，');
  if (problems.isEmpty) {
    stdout.writeln('依赖许可都在允许清单里（$summary）。');
    return;
  }
  stderr.writeln('依赖许可检查没通过：');
  for (final p in problems) {
    stderr.writeln('  $p');
  }
  exitCode = 1;
}
