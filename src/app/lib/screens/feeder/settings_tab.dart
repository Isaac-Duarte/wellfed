import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:wellfed/screens/feeder/settings_owner.dart';
import 'package:wellfed/screens/feeder/settings_rows.dart';
import 'package:wellfed/screens/feeder/settings_window.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/ask_text.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/pending.dart';
import 'package:wellfed/widgets/settings_list.dart';

/// Change one setting, against the generation the screen shows. A change
/// asked while another is out waits in Rust and goes on that one's generation.
Future<FeederReply> put(FeederController feeder, SettingsView settings, SettingsChanges changes) =>
    feeder.ask(FeederRequest.putSettings(base: settings.generation, changes: changes));

/// The feeder's settings (10.1), for a member what a member may change (10.3).
/// Each change goes back against the generation last read, so one made
/// elsewhere meanwhile is refused rather than overwritten. A control moves on
/// tap and shows the feeder's answer: see [Pending].
class SettingsTab extends StatelessWidget {
  const SettingsTab({super.key, required this.feeder});

  final FeederController feeder;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: feeder,
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final SettingsView? settings = feeder.state.card?.settings;
      final owner = feeder.state.role == ClientRole.owner;
      final online = feeder.state.connection == Connection.online;

      return ListView(
        // Clear of the gesture bar, so the last row is never under it.
        padding: EdgeInsets.fromLTRB(Space.s4, Space.s4, Space.s4, Space.s6 + MediaQuery.paddingOf(context).bottom),
        children: [
          if (settings == null)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: Space.s5),
              child: Text(
                online ? 'Reading the settings…' : "Can't read the settings right now.",
                textAlign: TextAlign.center,
                style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
              ),
            )
          else ...[
            if (!owner) ...[
              Text.rich(
                TextSpan(
                  style: TextStyle(fontSize: Sizes.base, height: 1.5, color: c.textMuted),
                  children: [
                    const TextSpan(text: "You're a "),
                    TextSpan(
                      text: 'member',
                      style: TextStyle(fontWeight: Fonts.semibold, color: c.text),
                    ),
                    TextSpan(
                      text:
                          ' of ${feeder.state.known.name}. You can change lights, '
                          'sounds and reminders.',
                    ),
                  ],
                ),
              ),
              const SizedBox(height: Space.s4),
            ],
            SettingsSection(
              'Lights and sound',
              Column(
                children: [
                  SettingsWindowRow(
                    feeder: feeder,
                    settings: settings,
                    online: online,
                    name: 'Lights',
                    on: settings.lights,
                    window: settings.lightsWindow,
                    question: 'When can the lights be on?',
                    toggle: (on) => SettingsChanges(lights: on),
                    windowed: (window) => SettingsChanges(lightsWindow: window),
                    first: true,
                  ),
                  SettingsWindowRow(
                    feeder: feeder,
                    settings: settings,
                    online: online,
                    name: 'Sounds',
                    on: settings.sound,
                    window: settings.soundWindow,
                    question: 'When can sounds play?',
                    toggle: (on) => SettingsChanges(sound: on),
                    windowed: (window) => SettingsChanges(soundWindow: window),
                  ),
                  SettingsVolumeRow(feeder: feeder, settings: settings, online: online),
                ],
              ),
            ),
            SettingsSection(
              'Reminders',
              Column(
                children: [
                  SettingsReminderRow(feeder: feeder, settings: settings, online: online),
                  SettingsDesiccantRow(feeder: feeder, settings: settings, online: online),
                ],
              ),
            ),
            if (owner) SettingsOwnerSection(feeder: feeder) else const SettingsOwnerOnly(),
          ],
          // What this phone keeps about the feeder: it works offline too.
          SettingsSection(
            'On this phone',
            Column(
              children: [
                SettingsTile(
                  first: true,
                  onTap: () => unawaited(_rename(context, feeder)),
                  child: Row(
                    children: [
                      Expanded(child: SettingsWords('Name', feeder.state.known.name)),
                      StrokeIcon(Glyph.chevron, size: 20, color: c.textMuted),
                    ],
                  ),
                ),
                SettingsTile(
                  first: false,
                  onTap: () => unawaited(_forget(context, feeder)),
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: Text(
                      'Remove from this phone',
                      style: TextStyle(fontSize: 15, fontWeight: Fonts.medium, color: c.fault),
                    ),
                  ),
                ),
              ],
            ),
            last: true,
          ),
        ],
      );
    },
  );

  /// Ask for a new name. It is this phone's name for the feeder, so it needs
  /// no session and works offline.
  static Future<void> _rename(BuildContext context, FeederController feeder) async {
    final String? name = await askText(
      context,
      title: 'Rename ${feeder.state.known.name}',
      label: 'Name',
      initial: feeder.state.known.name,
      action: 'Rename',
    );
    if (name == null || !context.mounted) return;

    final bool saved = await feeder.rename(name);
    if (!saved && context.mounted) {
      ScaffoldMessenger.of(context).showSnackBar(const SnackBar(content: Text("Couldn't save the name. Try again.")));
    }
  }

  /// This phone forgets the feeder. The feeder's list stays as it is, so an
  /// owner removes the key from People; a member cannot remove itself.
  static Future<void> _forget(BuildContext context, FeederController feeder) async {
    final String name = feeder.state.known.name;
    final bool? sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Remove $name from this phone?'),
        content: const Text(
          'It keeps feeding on its schedule. This phone stays on its People '
          'list until an owner removes it.',
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
          FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('Remove')),
        ],
      ),
    );
    if (sure != true || !context.mounted) return;
    Navigator.of(context).popUntil((route) => route.isFirst);
    await feeder.forgetHere();
  }
}
