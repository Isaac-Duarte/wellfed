import 'package:flutter/foundation.dart';

/// Whether this build runs on a desktop. The phone layout stays exactly as
/// it is on Android; only a desktop build takes the sidebar, the centered
/// columns and dialogs in place of sheets.
bool get isDesktop => switch (defaultTargetPlatform) {
  TargetPlatform.linux || TargetPlatform.macOS || TargetPlatform.windows => true,
  TargetPlatform.android || TargetPlatform.iOS || TargetPlatform.fuchsia => false,
};

/// What the app runs on, as its words name it: "this phone" on a phone,
/// "this computer" on a desktop.
String get deviceName => isDesktop ? 'computer' : 'phone';

/// Any of the owner's devices, as the words name them: "phone" on a phone,
/// "device" on a desktop, where a phone may open the feeder too.
String get anyDevice => isDesktop ? 'device' : 'phone';

/// What a computer that would not promise to stay awake asks of its owner
/// during a long run. Holding off sleep never covers a closed lid.
const String stayAwakeWords = 'Keep this computer awake, with its lid open, until this finishes.';
