import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/src/rust/api/alerts.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/alerts/alert_settings.dart';
import 'package:wellfed/state/alerts/alert_words.dart';
import 'package:wellfed/state/form_factor.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/settings_list.dart';

/// Alerts on this phone or computer: how they work and where, notifications,
/// "instant while home" on a phone, and what to mute.
class AlertSettingsScreen extends StatefulWidget {
  const AlertSettingsScreen({super.key, required this.newController});

  final AlertSettingsController Function() newController;

  @override
  State<AlertSettingsScreen> createState() => _AlertSettingsScreenState();
}

class _AlertSettingsScreenState extends State<AlertSettingsScreen> {
  late final AlertSettingsController _settings = widget.newController();

  // Back from the system's settings, notifications may be on now.
  late final AppLifecycleListener _lifecycle;

  @override
  void initState() {
    super.initState();
    _lifecycle = AppLifecycleListener(onResume: () => unawaited(_settings.resumed()));
    unawaited(_settings.load());
  }

  @override
  void dispose() {
    _lifecycle.dispose();
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
            if (isDesktop)
              const Notice(
                tone: Tone.neutral,
                glyph: Glyph.info,
                lead: 'Alerts work at home, while Wellfed is open.',
                text:
                    'This computer hears from a feeder on the same network only while Wellfed is open, so your '
                    'phone stays the main way to hear. The feeder decides what is wrong. This computer decides one '
                    "thing itself: that it can't reach a feeder.",
              )
            else
              const Notice(
                tone: Tone.neutral,
                glyph: Glyph.info,
                lead: 'Alerts work at home.',
                text:
                    "This phone hears from a feeder on the same Wi-Fi, so you won't get alerts away from home. "
                    "The feeder decides what is wrong. This phone decides one thing itself: that it can't reach "
                    'a feeder.',
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
        if (shown.watching case PhoneWatching(instant: true, running: false)) ...[
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
        if (!shown.notificationsAllowed && notificationAskLabel(shown.notificationAsk) == null) ...[
          Notice(
            tone: Tone.warning,
            glyph: Glyph.warning,
            lead: 'Notifications are off.',
            text: notificationsOffWords(shown.notificationAsk),
          ),
          const SizedBox(height: Space.s4),
        ],
        SettingsSection(
          'On this $deviceName',
          Column(
            children: [
              if (!shown.notificationsAllowed)
                if (notificationAskLabel(shown.notificationAsk) case final String label)
                  SettingsLink(
                    label,
                    notificationsOffWords(shown.notificationAsk),
                    first: true,
                    onTap: () => unawaited(settings.allowNotifications()),
                  ),
              switch (shown.watching) {
                PhoneWatching(:final bool instant) => SettingsTile(
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
                    value: instant,
                    onChanged: (on) => unawaited(settings.setInstant(on: on)),
                  ),
                ),
                OpenWatching() => SettingsTile(
                  first: shown.notificationsAllowed || notificationAskLabel(shown.notificationAsk) == null,
                  child: ListTile(
                    contentPadding: EdgeInsets.zero,
                    title: Text('Every minute while open', style: TextStyle(fontSize: 15, color: c.text)),
                    subtitle: Text(
                      "Wellfed checks each feeder every minute while it's open, and posts what's new. Closing "
                      'Wellfed stops the checks, and a notification it posted comes off once Wellfed is open again '
                      'to see the alert end.',
                      style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
                    ),
                  ),
                ),
              },
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
          "A muted alert still shows in its feeder's alert list. Dismissing an alert hides it on this "
          '$deviceName only; the feeder keeps it until it is over.',
          style: TextStyle(fontSize: Sizes.sm, height: 1.5, color: c.textMuted),
        ),
      ],
    );
  }
}
