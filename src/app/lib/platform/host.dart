import 'dart:io';

import 'package:path_provider/path_provider.dart';

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

/// The folder the app keeps its stores in: client keys, feeders, the
/// written-before-sent record and alerts.
///
/// On Windows it is `%LOCALAPPDATA%\Wellfed`, not the support folder the
/// plugin gives: that one roams between a domain's PCs, which could sync an
/// older written-before-sent record over a newer one, and it is named from
/// the runner's company and product strings, so renaming either would strand
/// every store.
Future<Directory> appDataDirectory() async {
  final String? local = Platform.isWindows ? Platform.environment['LOCALAPPDATA'] : null;
  if (local == null) return getApplicationSupportDirectory();
  return Directory('$local${Platform.pathSeparator}Wellfed').create(recursive: true);
}
