import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/alerts/alert_words.dart';
import 'package:wellfed/state/alerts/alerts.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/when.dart';

/// One feeder's alerts on this phone: what the feeder says is wrong now, as
/// this phone last caught up with it, with a check now and a dismissal that
/// hides an alert here only.
class AlertsScreen extends StatefulWidget {
  const AlertsScreen({super.key, required this.name, required this.newController, this.openSettings});

  /// The feeder's name on this phone.
  final String name;

  final FeederAlertsController Function() newController;

  /// Opens the alert settings.
  final VoidCallback? openSettings;

  @override
  State<AlertsScreen> createState() => _AlertsScreenState();
}

class _AlertsScreenState extends State<AlertsScreen> {
  late final FeederAlertsController _alerts = widget.newController();

  @override
  void initState() {
    super.initState();
    unawaited(_alerts.load());
  }

  @override
  void dispose() {
    _alerts.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Scaffold(
      appBar: AppBar(
        toolbarHeight: 64,
        automaticallyImplyLeading: false,
        titleSpacing: 4,
        leading: IconButton(
          tooltip: 'Back',
          onPressed: () => Navigator.pop(context),
          icon: StrokeIcon(Glyph.back, color: c.text),
        ),
        title: Text(
          'Alerts',
          style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
        ),
        actions: [
          if (widget.openSettings case final VoidCallback open)
            IconButton(
              tooltip: 'Alert settings',
              icon: Icon(Icons.tune, color: c.text),
              onPressed: open,
            ),
        ],
      ),
      body: ListenableBuilder(
        listenable: _alerts,
        builder: (context, _) => switch (_alerts.state) {
          AlertsLoading() => const Center(child: Searching('Reading the alerts')),
          AlertsFailed(:final String message) => Padding(
            padding: const EdgeInsets.all(Space.s4),
            child: Notice(tone: Tone.fault, glyph: Glyph.warning, text: "Couldn't read the alerts: $message"),
          ),
          final AlertsShown shown => _List(shown: shown, name: widget.name, alerts: _alerts),
        },
      ),
    );
  }
}

class _List extends StatelessWidget {
  const _List({required this.shown, required this.name, required this.alerts});

  final AlertsShown shown;
  final String name;
  final FeederAlertsController alerts;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final muted = TextStyle(fontSize: Sizes.sm, height: 1.5, color: c.textMuted);
    return ListView(
      padding: const EdgeInsets.all(Space.s4),
      children: [
        Text(switch (shown.reachedMs) {
          final int at => 'Last checked ${atMomentTextMs(context, at)}.',
          null => 'Not checked yet: this phone checks while it is home.',
        }, style: muted),
        const SizedBox(height: Space.s3),
        if (shown.problem case final String problem) ...[
          Notice(tone: Tone.warning, glyph: Glyph.warning, text: problem),
          const SizedBox(height: Space.s3),
        ],
        if (shown.notices.isEmpty)
          Panel(
            child: Padding(
              padding: const EdgeInsets.all(Space.s4),
              child: Text('Nothing is wrong that $name has said.', style: TextStyle(fontSize: 15, color: c.text)),
            ),
          ),
        for (final NoticeView notice in shown.notices) ...[
          _Alert(notice: notice, name: name, onDismiss: () => unawaited(alerts.dismiss(notice))),
          const SizedBox(height: Space.s3),
        ],
        if (shown.unread > 0) ...[
          Notice(
            tone: Tone.neutral,
            glyph: Glyph.info,
            text:
                '$name has ${shown.unread} more ${shown.unread == 1 ? 'alert' : 'alerts'} this app is too old '
                'to read. Update Wellfed to see ${shown.unread == 1 ? 'it' : 'them'}.',
          ),
          const SizedBox(height: Space.s3),
        ],
        const SizedBox(height: Space.s2),
        Align(
          alignment: AlignmentDirectional.centerStart,
          child: OutlinedButton(
            onPressed: shown.checking ? null : () => unawaited(alerts.check()),
            child: Text(shown.checking ? 'Checking…' : 'Check now'),
          ),
        ),
      ],
    );
  }
}

class _Alert extends StatelessWidget {
  const _Alert({required this.notice, required this.name, required this.onDismiss});

  final NoticeView notice;
  final String name;
  final VoidCallback onDismiss;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final NoticeWords words = noticeWords(notice, name);
    final Color mark = switch (notice.level) {
      AlertLevel.urgent => c.fault,
      AlertLevel.soon => c.warning,
      AlertLevel.security => c.accent,
    };
    return Panel(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(Space.s4, Space.s3, Space.s1, Space.s3),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Padding(
              padding: const EdgeInsets.only(top: 2),
              child: StrokeIcon(notice.level == AlertLevel.urgent ? Glyph.warning : Glyph.info, size: 20, color: mark),
            ),
            const SizedBox(width: Space.s3),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    words.title,
                    style: TextStyle(fontSize: 15, fontWeight: Fonts.semibold, color: c.text),
                  ),
                  const SizedBox(height: 2),
                  Text(
                    words.body,
                    style: TextStyle(fontSize: Sizes.sm, height: 1.45, color: c.text),
                  ),
                  const SizedBox(height: 4),
                  Text(
                    [
                      '${levelName(notice.level)} · ${momentTextMs(context, notice.atMs ?? notice.seenMs)}',
                      if (notice.quiet) 'not notified',
                    ].join(' · '),
                    style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
                  ),
                ],
              ),
            ),
            IconButton(
              tooltip: 'Dismiss on this phone',
              icon: StrokeIcon(Glyph.close, size: 20, color: c.textMuted),
              onPressed: onDismiss,
            ),
          ],
        ),
      ),
    );
  }
}
