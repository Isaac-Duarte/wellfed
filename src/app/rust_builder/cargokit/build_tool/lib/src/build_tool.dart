/// This is copied from Cargokit (which is the official way to use it currently)
/// Details: https://fzyzcjy.github.io/flutter_rust_bridge/manual/integrate/builtin

import 'dart:io';

import 'package:args/command_runner.dart';
import 'package:logging/logging.dart';

import 'android_environment.dart';
import 'build_cmake.dart';
import 'build_gradle.dart';
import 'build_pod.dart';
import 'logging.dart';
import 'options.dart';
import 'util.dart';

final log = Logger('build_tool');

abstract class BuildCommand extends Command {
  Future<void> runBuildCommand(CargokitUserOptions options);

  @override
  Future<void> run() async {
    final options = CargokitUserOptions.load();

    if (options.verboseLogging ||
        Platform.environment['CARGOKIT_VERBOSE'] == '1') {
      enableVerboseLogging();
    }

    await runBuildCommand(options);
  }
}

class BuildPodCommand extends BuildCommand {
  @override
  final name = 'build-pod';

  @override
  final description = 'Build cocoa pod library';

  @override
  Future<void> runBuildCommand(CargokitUserOptions options) async {
    final build = BuildPod(userOptions: options);
    await build.build();
  }
}

class BuildGradleCommand extends BuildCommand {
  @override
  final name = 'build-gradle';

  @override
  final description = 'Build android library';

  @override
  Future<void> runBuildCommand(CargokitUserOptions options) async {
    final build = BuildGradle(userOptions: options);
    await build.build();
  }
}

class BuildCMakeCommand extends BuildCommand {
  @override
  final name = 'build-cmake';

  @override
  final description = 'Build CMake library';

  @override
  Future<void> runBuildCommand(CargokitUserOptions options) async {
    final build = BuildCMake(userOptions: options);
    await build.build();
  }
}

Future<void> runMain(List<String> args) async {
  try {
    // Init logging before options are loaded
    initLogging();

    if (Platform.environment['_CARGOKIT_NDK_LINK_TARGET'] != null) {
      return AndroidEnvironment.clangLinkerWrapper(args);
    }

    final runner = CommandRunner('build_tool', 'Cargokit built_tool')
      ..addCommand(BuildPodCommand())
      ..addCommand(BuildGradleCommand())
      ..addCommand(BuildCMakeCommand());

    await runner.run(args);
  } on ArgumentError catch (e) {
    stderr.writeln(e.toString());
    exit(1);
  } catch (e, s) {
    log.severe(kDoubleSeparator);
    log.severe('Cargokit BuildTool failed with error:');
    log.severe(kSeparator);
    log.severe(e);
    // This tells user to install Rust, there's no need to pollute the log with
    // stack trace.
    if (e is! RustupNotFoundException) {
      log.severe(kSeparator);
      log.severe(s);
      log.severe(kSeparator);
      log.severe('BuildTool arguments: $args');
    }
    log.severe(kDoubleSeparator);
    exit(1);
  }
}
