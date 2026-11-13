import 'package:flutter/material.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/store_notice.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/layout.dart';

/// What the start found of the app's files that did not read: each was kept
/// under another name and replaced with an empty one. Shown before Home
/// until the owner has read it; with the record of writes among them, until
/// the owner has looked in the bowls, since no feed goes before then.
class SetAsideScreen extends StatelessWidget {
  const SetAsideScreen({super.key, required this.notice, required this.onRead});

  final StoreNotice notice;

  /// The owner read it, and checked the bowls when it asked.
  final VoidCallback onRead;

  @override
  Widget build(BuildContext context) => StepScaffold(
    leading: Leading.none,
    title: "Some of this $deviceName's data couldn't be read",
    body:
        'Wellfed kept it and started over with that part empty. What lets your feeders recognize '
        'this $deviceName, and any saved copies of the original firmware, are stored apart and are untouched.',
    content: [
      for (final SetAside file in notice.files) ...[
        Notice(
          tone: file.kind == StoreKind.pending ? Tone.warning : Tone.neutral,
          lead: _lead(file.kind),
          text: _words(file.kind),
        ),
        const SizedBox(height: Space.s2),
        Text(
          'Kept as ${_name(file.movedTo)}',
          style: TextStyle(fontSize: Sizes.sm, color: context.colors.textMuted),
        ),
        const SizedBox(height: Space.s4),
      ],
    ],
    actions: [FilledButton(onPressed: onRead, child: Text(notice.holdsFeeds ? "I've checked the bowls" : 'Continue'))],
  );

  static String _lead(StoreKind kind) => switch (kind) {
    StoreKind.app => 'Your feeders.',
    StoreKind.pending => 'Feeding is paused.',
    StoreKind.alerts => 'Alerts.',
  };

  static String _words(StoreKind kind) => switch (kind) {
    StoreKind.app =>
      "This $deviceName's list of feeders starts empty, but your feeders still know "
          'this $deviceName. To put one back on the list, stand at the feeder and use Add a feeder, then '
          'Recover a feeder. Or ask someone '
          'who uses the feeder for an invite.',
    StoreKind.pending =>
      "The record of feeds and changes that hadn't been confirmed is gone, so a feed sent "
          "just before may have run. Look in every feeder's bowl before you feed again.",
    StoreKind.alerts => 'Muted and dismissed alerts start over.',
  };

  /// The kept file's own name: the folder is the app's private storage.
  static String _name(String path) => path.split(RegExp(r'[/\\]')).last;
}
