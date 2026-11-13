import 'dart:ui' show AppExitResponse;

import 'package:flutter/material.dart';
import 'package:wellfed/src/rust/api/alerts.dart' show AlertHold;

/// Asks before a computer's window closes on a run: an install, an update
/// or a restore. Closing ends the process, which stops the run wherever it
/// is, so the owner hears that first. Pops `true` to close.
class CloseRunDialog extends StatelessWidget {
  const CloseRunDialog({super.key, required this.runs});

  /// The runs going now, oldest first.
  final Set<AlertHold> runs;

  static String _what(AlertHold run) => switch (run) {
    AlertHold.install => 'An install',
    AlertHold.update => 'A firmware update',
    AlertHold.restore => 'A restore of the original firmware',
  };

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Close Wellfed?'),
    content: Text(
      '${runs.isEmpty ? 'A run' : _what(runs.last)} is still going. Closing stops it partway, and '
      'may leave the feeder mid-step. Nothing it sent goes again by itself, so '
      'open Wellfed afterwards to see where the feeder got to.',
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Keep going')),
      TextButton(onPressed: () => Navigator.pop(context, true), child: const Text('Close anyway')),
    ],
  );
}

/// Answers a computer's exit requests (its window closing, or the app
/// quitting): at once with no run going, and through [CloseRunDialog] while
/// one is. A second request while the dialog is up is refused rather than
/// stacking a second dialog.
class CloseRunGuard {
  /// A guard over the runs `runs` names, asking over `context`'s navigator.
  CloseRunGuard({required this.runs, required this.context});

  /// The runs going now, oldest first.
  final Set<AlertHold> Function() runs;

  /// Where the dialog opens, once the app has a navigator.
  final BuildContext? Function() context;

  bool _asking = false;

  /// Whether the app may exit now.
  Future<AppExitResponse> exitRequested() async {
    final Set<AlertHold> going = runs();
    final BuildContext? at = context();
    if (going.isEmpty || at == null) return AppExitResponse.exit;
    if (_asking) return AppExitResponse.cancel;
    _asking = true;
    try {
      final bool? close = await showDialog<bool>(
        context: at,
        builder: (_) => CloseRunDialog(runs: going),
      );
      return close ?? false ? AppExitResponse.exit : AppExitResponse.cancel;
    } finally {
      _asking = false;
    }
  }
}
