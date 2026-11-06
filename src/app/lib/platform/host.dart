import 'dart:io';

/// Whether this process runs on a desktop operating system, for choosing
/// which plugin or bridge call does a job.
///
/// The real host, not `defaultTargetPlatform`: a test may set that to draw
/// the desktop layout, but the plugins underneath are still the host's.
/// Layout and wording key on `isDesktop` in `state/form_factor.dart`.
bool get onDesktopHost => Platform.isLinux || Platform.isMacOS || Platform.isWindows;
