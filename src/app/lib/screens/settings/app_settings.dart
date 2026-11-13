import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/src/limits.g.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/settings/backups.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/ask_text.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/settings_list.dart';

const sourceUrl = 'https://github.com/Isaac-Duarte/wellfed';
const issuesUrl = '$sourceUrl/issues';

/// The gear that opens the app's settings, from Feeders, or from Welcome
/// before there are any.
class SettingsButton extends StatelessWidget {
  const SettingsButton({super.key});

  @override
  Widget build(BuildContext context) => IconButton(
    tooltip: 'Settings',
    icon: Icon(Icons.settings_outlined, color: context.colors.text),
    onPressed: () => unawaited(context.push('/settings')),
  );
}

/// The app's own settings (15.1).
class AppSettingsScreen extends StatefulWidget {
  const AppSettingsScreen({super.key, required this.book, required this.newBackups, this.alerts = false});

  final FeederBook book;

  /// This phone watches for alerts, so the page links to their settings.
  final bool alerts;

  /// The controller of the backups this phone keeps.
  final BackupsFlow Function() newBackups;

  @override
  State<AppSettingsScreen> createState() => _AppSettingsScreenState();
}

class _AppSettingsScreenState extends State<AppSettingsScreen> {
  late final BackupsFlow _backups = widget.newBackups();

  @override
  void initState() {
    super.initState();
    unawaited(_backups.load());
  }

  @override
  void dispose() {
    _backups.dispose();
    super.dispose();
  }

  Future<void> _rename() async {
    final String? name = await askText(
      context,
      title: 'Name on feeders',
      label: 'This $deviceName',
      initial: widget.book.label,
      note: 'Feeders keep the name they were joined with.',
    );
    if (name == null || !mounted) return;

    final String trimmed = name.trim();
    final int bytes = utf8.encode(trimmed).length;
    if (bytes < 1 || bytes > Limits.labelBytesMost) {
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text('A $deviceName name is 1 to ${Limits.labelBytesMost} bytes.')));
      return;
    }
    await widget.book.setLabel(trimmed);
  }

  Future<void> _share(String path) async {
    final String? failed = await _backups.share(path);
    if (failed == null || !mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(failed)));
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([widget.book, _backups]),
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final FeederBook book = widget.book;
      // A desktop opens Settings from the sidebar, which is also the way out.
      final bool fromSidebar = isDesktop && !Navigator.of(context).canPop();
      return Scaffold(
        appBar: AppBar(
          toolbarHeight: isDesktop ? 72 : 64,
          automaticallyImplyLeading: false,
          titleSpacing: fromSidebar ? Space.s5 : 4,
          leading: fromSidebar
              ? null
              : IconButton(
                  tooltip: 'Back',
                  // Opened from a link with nothing under it, back goes to Feeders.
                  onPressed: () => Navigator.of(context).canPop() ? Navigator.pop(context) : context.go('/'),
                  icon: StrokeIcon(Glyph.back, color: c.text),
                ),
          title: Text(
            'Settings',
            style: isDesktop
                ? TextStyle(fontSize: 24, fontWeight: Fonts.display, letterSpacing: -0.4, color: c.text)
                : TextStyle(fontSize: 22, fontWeight: Fonts.semibold, color: c.text),
          ),
        ),
        body: ListView(
          padding: const EdgeInsets.all(Space.s4),
          children: [
            // Indented as each settings section's label is.
            const Padding(
              padding: EdgeInsets.only(left: Space.s1, bottom: Space.s2),
              child: SectionLabel('Appearance'),
            ),
            SegmentedButton<Appearance>(
              segments: const [
                ButtonSegment(value: Appearance.system, label: Text('System')),
                ButtonSegment(value: Appearance.light, label: Text('Light')),
                ButtonSegment(value: Appearance.dark, label: Text('Dark')),
              ],
              selected: {book.appearance},
              onSelectionChanged: (chosen) => unawaited(book.setAppearance(chosen.first)),
            ),
            const SizedBox(height: Space.s5),
            SettingsSection(
              'This $deviceName',
              Column(
                children: [
                  SettingsLink('Name on feeders', book.label, first: true, onTap: () => unawaited(_rename())),
                  SettingsLink(
                    'About Wellfed',
                    null,
                    onTap: () => unawaited(
                      Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => const AboutScreen())),
                    ),
                  ),
                ],
              ),
            ),
            if (widget.alerts)
              SettingsSection(
                'Alerts',
                SettingsLink(
                  'Alerts at home',
                  'Notifications, instant checks and what to mute',
                  first: true,
                  onTap: () => unawaited(context.push('/alert-settings')),
                ),
              ),
            SettingsSection(
              'Firmware backups',
              last: true,
              Column(
                children: [
                  if (_backups.state.copies.isEmpty)
                    SettingsTile(
                      first: true,
                      child: Align(
                        alignment: AlignmentDirectional.centerStart,
                        child: Text(
                          _backups.state.problem ?? 'None yet',
                          style: TextStyle(fontSize: 15, color: c.textMuted),
                        ),
                      ),
                    ),
                  for (final (index, copy) in _backups.state.copies.indexed)
                    SettingsTile(
                      first: index == 0,
                      child: Row(
                        children: [
                          Expanded(
                            child: SettingsWords(
                              copy.label,
                              copy.name == null ? 'Not on this $deviceName any more' : null,
                            ),
                          ),
                          IconButton(
                            tooltip: isDesktop ? "Save ${copy.label}'s backup" : "Share ${copy.label}'s backup",
                            icon: Icon(isDesktop ? Icons.save_alt_outlined : Icons.share_outlined, color: c.textMuted),
                            onPressed: () => unawaited(_share(copy.path)),
                          ),
                        ],
                      ),
                    ),
                ],
              ),
            ),
          ],
        ),
      );
    },
  );
}

/// What this app is (15.2).
class AboutScreen extends StatelessWidget {
  const AboutScreen({super.key, this.version = appVersion});

  final String Function() version;

  void _copy(BuildContext context, String url) {
    unawaited(Clipboard.setData(ClipboardData(text: url)));
    ScaffoldMessenger.of(context).showSnackBar(const SnackBar(content: Text('Link copied.')));
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final muted = TextStyle(fontSize: Sizes.sm, height: 1.5, color: c.textMuted);
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
          'About',
          style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
        ),
      ),
      body: ListView(
        padding: const EdgeInsets.all(Space.s4),
        children: [
          Text(
            'Wellfed',
            style: TextStyle(fontSize: 22, fontWeight: Fonts.semibold, color: c.text),
          ),
          Text('App ${version()}', style: muted),
          const SizedBox(height: Space.s3),
          Text(
            'Open firmware and an app for the PETLIBRO Granary PLAF103, so it '
            'runs on your own network.',
            style: TextStyle(fontSize: 15, height: 1.5, color: c.text),
          ),
          const SizedBox(height: Space.s4),
          Panel(
            child: Column(
              children: [
                ListTile(
                  title: const Text('Source code'),
                  subtitle: const Text(sourceUrl),
                  onTap: () => _copy(context, sourceUrl),
                ),
                ListTile(
                  title: const Text('Open-source licenses'),
                  onTap: () =>
                      showLicensePage(context: context, applicationName: 'Wellfed', applicationVersion: version()),
                ),
                ListTile(
                  title: const Text('Report a problem'),
                  subtitle: const Text(issuesUrl),
                  onTap: () => _copy(context, issuesUrl),
                ),
              ],
            ),
          ),
          const SizedBox(height: Space.s4),
          Text(
            'Independent community project. Not affiliated with or endorsed '
            "by PETLIBRO. PETLIBRO and Granary are their owner's trademarks.",
            style: muted,
          ),
        ],
      ),
    );
  }
}
