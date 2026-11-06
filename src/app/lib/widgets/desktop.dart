import 'package:flutter/material.dart';
import 'package:wellfed/state/form_factor.dart';
import 'package:wellfed/theme/theme.dart';

export 'package:wellfed/state/form_factor.dart';

/// The widest a desktop page's column reads well at.
const double desktopColumn = 880;

/// The widest a step of a flow is on a desktop: a form, not a page.
const double desktopStep = 560;

/// The widest a short page of settings or fields is on a desktop: wider than
/// a step, which is a card, but narrower than a page of lists, so its fields
/// and buttons don't run the window's width.
const double desktopForm = 640;

/// The most a page opened as a dialog on a desktop takes of the window.
const Size _dialogPage = Size(560, 760);

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

/// `page` at [desktopForm] on a desktop, its header included, so the title
/// and the back button line up with the fields under them. A phone shows it
/// as it is.
class FormWidth extends StatelessWidget {
  const FormWidth({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) => isDesktop ? CenteredColumn(maxWidth: desktopForm, child: child) : child;
}

/// Open `page`, a screen with its own close button that asks one thing (a
/// meal, an invite): full screen on a phone, a dialog over the window on a
/// desktop, where a page that size reads as a form stretched across it.
///
/// The page closes itself with [Navigator.pop] either way, as the dialog is
/// its own route. A page it opens in turn goes through here too, so it
/// stacks over it rather than taking the window.
Future<T?> openModalPage<T>(BuildContext context, WidgetBuilder page) {
  if (!isDesktop) {
    return Navigator.of(context).push(MaterialPageRoute<T>(fullscreenDialog: true, builder: page));
  }

  return showDialog<T>(
    context: context,
    // A tap beside it would throw away what was typed.
    barrierDismissible: false,
    builder: (context) {
      final Size window = MediaQuery.sizeOf(context);
      return Dialog(
        backgroundColor: context.colors.background,
        surfaceTintColor: Colors.transparent,
        insetPadding: const EdgeInsets.all(Space.s6),
        clipBehavior: Clip.antiAlias,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(Radii.card),
          side: BorderSide(color: context.colors.line),
        ),
        child: SizedBox(
          width: _dialogPage.width,
          height: (window.height - 2 * Space.s6).clamp(0, _dialogPage.height),
          child: page(context),
        ),
      );
    },
  );
}
