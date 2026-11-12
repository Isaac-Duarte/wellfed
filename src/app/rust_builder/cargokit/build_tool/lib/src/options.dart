/// This is copied from Cargokit (which is the official way to use it currently)
/// Details: https://fzyzcjy.github.io/flutter_rust_bridge/manual/integrate/builtin

import 'dart:io';

import 'package:collection/collection.dart';
import 'package:logging/logging.dart';
import 'package:path/path.dart' as path;
import 'package:source_span/source_span.dart';
import 'package:yaml/yaml.dart';

import 'builder.dart';
import 'environment.dart';

final _log = Logger('options');

/// A class for exceptions that have source span information attached.
class SourceSpanException implements Exception {
  // This is a getter so that subclasses can override it.
  /// A message describing the exception.
  String get message => _message;
  final String _message;

  // This is a getter so that subclasses can override it.
  /// The span associated with this exception.
  ///
  /// This may be `null` if the source location can't be determined.
  SourceSpan? get span => _span;
  final SourceSpan? _span;

  SourceSpanException(this._message, this._span);

  /// Returns a string representation of `this`.
  ///
  /// [color] may either be a [String], a [bool], or `null`. If it's a string,
  /// it indicates an ANSI terminal color escape that should be used to
  /// highlight the span's text. If it's `true`, it indicates that the text
  /// should be highlighted using the default color. If it's `false` or `null`,
  /// it indicates that the text shouldn't be highlighted.
  @override
  String toString({Object? color}) {
    if (span == null) return message;
    return 'Error on ${span!.message(message, color: color)}';
  }
}

class CargoBuildOptions {
  /// A rustup channel (`stable`, `beta`, `nightly`) or an exact version.
  /// Wellfed: an exact version, so the app builds with the toolchain
  /// rust-toolchain.toml pins rather than whatever `stable` is locally.
  final String toolchain;
  final List<String> flags;

  CargoBuildOptions({
    required this.toolchain,
    required this.flags,
  });

  static String _toolchainFromNode(YamlNode node) {
    if (node case YamlScalar(value: String name) when name.isNotEmpty) {
      return name;
    }
    throw SourceSpanException(
        'A toolchain is a rustup channel or version, such as stable or 1.99.1.',
        node.span);
  }

  static CargoBuildOptions parse(YamlNode node) {
    if (node is! YamlMap) {
      throw SourceSpanException('Cargo options must be a map', node.span);
    }
    String toolchain = 'stable';
    List<String> flags = [];
    for (final MapEntry(:key, :value) in node.nodes.entries) {
      if (key case YamlScalar(value: 'toolchain')) {
        toolchain = _toolchainFromNode(value);
      } else if (key case YamlScalar(value: 'extra_flags')) {
        if (value case YamlList(nodes: List<YamlNode> list)) {
          if (list.every((element) {
            if (element case YamlScalar(value: String _)) {
              return true;
            }
            return false;
          })) {
            flags = list.map((e) => e.value as String).toList();
            continue;
          }
        }
        throw SourceSpanException(
            'Extra flags must be a list of strings', value.span);
      } else {
        throw SourceSpanException(
            'Unknown cargo option type. Must be "toolchain" or "extra_flags".',
            key.span);
      }
    }
    return CargoBuildOptions(toolchain: toolchain, flags: flags);
  }
}

/// Cargokit options specified for Rust crate.
class CargokitCrateOptions {
  CargokitCrateOptions({
    this.cargo = const {},
  });

  final Map<BuildConfiguration, CargoBuildOptions> cargo;

  static CargokitCrateOptions parse(YamlNode node) {
    if (node is! YamlMap) {
      throw SourceSpanException('Cargokit options must be a map', node.span);
    }
    final options = <BuildConfiguration, CargoBuildOptions>{};

    for (final entry in node.nodes.entries) {
      if (entry
          case MapEntry(
            key: YamlScalar(value: 'cargo'),
            value: YamlNode node,
          )) {
        if (node is! YamlMap) {
          throw SourceSpanException('Cargo options must be a map', node.span);
        }
        for (final MapEntry(:YamlNode key, :value) in node.nodes.entries) {
          if (key case YamlScalar(value: String name)) {
            final configuration = BuildConfiguration.values
                .firstWhereOrNull((element) => element.name == name);
            if (configuration != null) {
              options[configuration] = CargoBuildOptions.parse(value);
              continue;
            }
          }
          throw SourceSpanException(
              'Unknown build configuration. Must be one of ${BuildConfiguration.values.map((e) => e.name)}.',
              key.span);
        }
      } else {
        throw SourceSpanException(
            'Unknown cargokit option type. Must be "cargo".', entry.key.span);
      }
    }
    return CargokitCrateOptions(cargo: options);
  }

  static CargokitCrateOptions load({
    required String manifestDir,
  }) {
    final uri = Uri.file(path.join(manifestDir, "cargokit.yaml"));
    final file = File.fromUri(uri);
    if (file.existsSync()) {
      final contents = loadYamlNode(file.readAsStringSync(), sourceUrl: uri);
      return parse(contents);
    } else {
      return CargokitCrateOptions();
    }
  }
}

class CargokitUserOptions {
  CargokitUserOptions({
    required this.verboseLogging,
  });

  CargokitUserOptions._() : verboseLogging = false;

  static CargokitUserOptions parse(YamlNode node) {
    if (node is! YamlMap) {
      throw SourceSpanException('Cargokit options must be a map', node.span);
    }
    bool verboseLogging = false;

    for (final entry in node.nodes.entries) {
      if (entry.key case YamlScalar(value: 'verbose_logging')) {
        if (entry.value case YamlScalar(value: bool value)) {
          verboseLogging = value;
          continue;
        }
        throw SourceSpanException(
            'Invalid value for "verbose_logging". Must be a boolean.',
            entry.value.span);
      } else {
        throw SourceSpanException(
            'Unknown cargokit option type. Must be "verbose_logging".',
            entry.key.span);
      }
    }
    return CargokitUserOptions(verboseLogging: verboseLogging);
  }

  static CargokitUserOptions load() {
    String fileName = "cargokit_options.yaml";
    var userProjectDir = Directory(Environment.rootProjectDir);

    while (userProjectDir.parent.path != userProjectDir.path) {
      final configFile = File(path.join(userProjectDir.path, fileName));
      if (configFile.existsSync()) {
        final contents = loadYamlNode(
          configFile.readAsStringSync(),
          sourceUrl: configFile.uri,
        );
        final res = parse(contents);
        if (res.verboseLogging) {
          _log.info('Found user options file at ${configFile.path}');
        }
        return res;
      }
      userProjectDir = userProjectDir.parent;
    }
    return CargokitUserOptions._();
  }

  final bool verboseLogging;
}
