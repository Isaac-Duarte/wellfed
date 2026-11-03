import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import 'package:wellfed/theme/theme.dart';

/// Whether this build runs on a desktop. The phone layout stays exactly as
/// it is on Android; only a desktop build takes the sidebar, the centered
/// columns and dialogs in place of sheets.
bool get isDesktop => switch (defaultTargetPlatform) {
  TargetPlatform.linux || TargetPlatform.macOS || TargetPlatform.windows => true,
  TargetPlatform.android || TargetPlatform.iOS || TargetPlatform.fuchsia => false,
};

/// The widest a desktop page's column reads well at.
const double desktopColumn = 880;

/// The widest a step of a flow is on a desktop: a form, not a page.
const double desktopStep = 560;

/// `child` centered at no more than [maxWidth], filling the height.
class CenteredColumn extends StatelessWidget {
  const CenteredColumn({super.key, required this.child, this.maxWidth = desktopColumn});

  final Widget child;
  final double maxWidth;

  @override
  Widget build(BuildContext context) => Align(
    alignment: Alignment.topCenter,
    child: ConstrainedBox(
      constraints: BoxConstraints(maxWidth: maxWidth),
      child: child,
    ),
  );
}

/// A sheet on a phone, a dialog on a desktop: a bottom sheet across a wide
/// window reads as a strip of page rather than a question.
///
/// `builder`'s content brings its own sides and bottom padding, as a sheet's
/// does under its drag handle; the dialog adds [dialogTop] for the top a
/// handle would take.
Future<T?> showPanel<T>({
  required BuildContext context,
  required WidgetBuilder builder,
  Color? barrierColor,
  bool scrollControlled = false,
  double dialogTop = Space.s6,
}) {
  if (!isDesktop) {
    return showModalBottomSheet<T>(
      context: context,
      isScrollControlled: scrollControlled,
      barrierColor: barrierColor,
      builder: builder,
    );
  }

  return showDialog<T>(
    context: context,
    barrierColor: barrierColor,
    builder: (context) => Dialog(
      backgroundColor: context.colors.surface,
      surfaceTintColor: Colors.transparent,
      insetPadding: const EdgeInsets.all(Space.s6),
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(Radii.card),
        side: BorderSide(color: context.colors.line),
      ),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 480),
        child: Padding(
          padding: EdgeInsets.only(top: dialogTop),
          child: builder(context),
        ),
      ),
    ),
  );
}

/// What the app runs on, as its words name it: "this phone" on a phone,
/// "this computer" on a desktop.
String get deviceName => isDesktop ? 'computer' : 'phone';

/// Any of the owner's devices, as the words name them: "phone" on a phone,
/// "device" on a desktop, where a phone may open the feeder too.
String get anyDevice => isDesktop ? 'device' : 'phone';
