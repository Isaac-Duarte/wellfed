import 'dart:io';

import 'package:path_provider/path_provider.dart';
import 'package:wellfed/src/rust/api/feeder.dart' show useDataDirectory;

/// Whether this process runs on a desktop operating system, for choosing
/// which plugin or bridge call does a job.
///
/// The real host, not `defaultTargetPlatform`: a test may set that to draw
/// the desktop layout, but the plugins underneath are still the host's.
/// Layout and wording key on `isDesktop` in `state/form_factor.dart`.
bool get onDesktopHost => Platform.isLinux || Platform.isMacOS || Platform.isWindows;

/// Whether `mobile_scanner` can read a code with this host's camera. It has
/// an Android and a macOS side, and none for Linux or Windows.
bool get hostCanScan => !onDesktopHost || Platform.isMacOS;

/// Tell the bridge where the app's stores are, so its sessions keep their
/// memo of the clock syncs beside the catch-ups'. Once a process, after
/// `RustLib.init`.
Future<void> nameDataDirectory() async => useDataDirectory(dir: (await appDataDirectory()).path);

/// The folder the app keeps its stores in: client keys, feeders, the
/// written-before-sent record and alerts.
///
/// On Windows it is `%LOCALAPPDATA%\Wellfed`, not the support folder the
/// plugin gives: that one roams between a domain's PCs, which could sync an
/// older written-before-sent record over a newer one, and it is named from
/// the runner's company and product strings, so renaming either would strand
/// every store.
///
/// Windows always sets `LOCALAPPDATA`; a launch without it fails rather than
/// open a second, empty set of stores somewhere else.
Future<Directory> appDataDirectory() async {
  if (!Platform.isWindows) return getApplicationSupportDirectory();
  final String local = Platform.environment['LOCALAPPDATA'] ?? '';
  if (local.isEmpty) {
    throw const FileSystemException('LOCALAPPDATA is not set, so the app has nowhere to keep its stores');
  }
  return Directory('$local${Platform.pathSeparator}Wellfed').create(recursive: true);
}
