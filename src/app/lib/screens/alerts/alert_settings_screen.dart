import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/alerts/alert_settings.dart';
import 'package:wellfed/state/alerts/alert_words.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/settings_list.dart';

/// Alerts on this phone: how they work and where, notifications, "instant
/// while home", and what to mute.
class AlertSettingsScreen extends StatefulWidget {
  const AlertSettingsScreen({super.key, required this.newController});

  final AlertSettingsController Function() newController;

  @override
  State<AlertSettingsScreen> createState() => _AlertSettingsScreenState();
}

class _AlertSettingsScreenState extends State<AlertSettingsScreen> {
  late final AlertSettingsController _settings = widget.newController();

  @override
  void initState() {
    super.initState();
    unawaited(_settings.load());
  }

  @override
  void dispose() {
    _settings.dispose();
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
      ),
      body: ListenableBuilder(
        listenable: _settings,
        builder: (context, _) => ListView(
          padding: const EdgeInsets.all(Space.s4),
          children: [
            const Notice(
              tone: Tone.neutral,
              glyph: Glyph.info,
              lead: 'Alerts work only at home.',
              text:
                  'Without a home server or a relay, this phone hears from a feeder only while it is on the '
                  "same Wi-Fi. Away from home you won't get alerts, and nothing tells you that a feeder "
                  'stopped. The feeder decides what is wrong; this phone only says so, and decides one thing '
                  "itself: that it can't reach a feeder while it is home.",
            ),
            const SizedBox(height: Space.s5),
            switch (_settings.state) {
              AlertSettingsLoading() => const Searching('Reading the settings'),
              AlertSettingsFailed(:final String message) => Notice(
                tone: Tone.fault,
                glyph: Glyph.warning,
                text: "Couldn't read the alert settings: $message",
              ),
              final AlertSettingsShown shown => _Choices(shown: shown, settings: _settings),
            },
          ],
        ),
      ),
    );
  }
}

class _Choices extends StatelessWidget {
  const _Choices({required this.shown, required this.settings});

  final AlertSettingsShown shown;
  final AlertSettingsController settings;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (shown.problem case final String problem) ...[
          Notice(tone: Tone.warning, glyph: Glyph.warning, text: problem),
          const SizedBox(height: Space.s4),
        ],
        if (shown.instant && !shown.instantRunning) ...[
          const Notice(
            tone: Tone.warning,
            glyph: Glyph.warning,
            lead: "Instant isn't running.",
            text:
                "Android stopped it and won't start it again, so alerts come about every 15 minutes. If "
                "Wellfed's battery use is Restricted in Android's app settings, set it to Optimized or "
                'Unrestricted, then turn instant off and on here.',
          ),
          const SizedBox(height: Space.s4),
        ],
        SettingsSection(
          'On this phone',
          Column(
            children: [
              if (!shown.notificationsAllowed)
                SettingsLink(
                  'Allow notifications',
                  'Android is keeping alerts off this phone.',
                  first: true,
                  onTap: () => unawaited(settings.allowNotifications()),
                ),
              SettingsTile(
                first: shown.notificationsAllowed,
                child: SwitchListTile(
                  contentPadding: EdgeInsets.zero,
                  title: Text('Instant while home', style: TextStyle(fontSize: 15, color: c.text)),
                  subtitle: Text(
                    'Checks every minute while this phone is on the Wi-Fi, with a notice that stays while it '
                    'runs. It uses more battery. Off, Android checks about every 15 minutes, and less '
                    'often while the phone sleeps. It stops when the phone restarts or Android stops it, '
                    'and starts again when you open Wellfed.',
                    style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
                  ),
                  value: shown.instant,
                  onChanged: (on) => unawaited(settings.setInstant(on: on)),
                ),
              ),
            ],
          ),
        ),
        SettingsSection(
          'Levels',
          Column(
            children: [
              for (final (index, level) in AlertLevel.values.indexed)
                SettingsTile(
                  first: index == 0,
                  child: SwitchListTile(
                    contentPadding: EdgeInsets.zero,
                    title: Text(levelName(level), style: TextStyle(fontSize: 15, color: c.text)),
                    value: !shown.mutedLevels.contains(level),
                    onChanged: (on) => unawaited(settings.muteLevel(level, muted: !on)),
                  ),
                ),
            ],
          ),
        ),
        SettingsSection(
          'Each alert',
          last: true,
          Column(
            children: [
              for (final (index, KindLevel kind) in shown.kinds.indexed)
                SettingsTile(
                  first: index == 0,
                  child: SwitchListTile(
                    contentPadding: EdgeInsets.zero,
                    title: Text(kindName(kind.kind), style: TextStyle(fontSize: 15, color: c.text)),
                    subtitle: shown.mutedLevels.contains(kind.level)
                        ? Text(
                            'Off with ${levelName(kind.level)}',
                            style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
                          )
                        : null,
                    value: !shown.muted(kind),
                    // A level that is off decides for each alert in it.
                    onChanged: shown.mutedLevels.contains(kind.level)
                        ? null
                        : (on) => unawaited(settings.muteKind(kind.kind, muted: !on)),
                  ),
                ),
            ],
          ),
        ),
        const SizedBox(height: Space.s3),
        Text(
          "A muted alert still shows in its feeder's alert list. Dismissing an alert hides it on this phone "
          'only; the feeder keeps it until it is over.',
          style: TextStyle(fontSize: Sizes.sm, height: 1.5, color: c.textMuted),
        ),
      ],
    );
  }
}
