import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/screens/setup/name_step.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/app_state.dart';
import 'package:wellfed/state/settings/backups.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/ask_text.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';

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
  const AppSettingsScreen({super.key, required this.book, required this.newBackups});

  final FeederBook book;

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
      label: 'This phone',
      initial: widget.book.label,
      note: 'Feeders keep the name they were joined with.',
    );
    if (name == null || !mounted) return;

    final String trimmed = name.trim();
    final int bytes = utf8.encode(trimmed).length;
    if (bytes < 1 || bytes > NameStep.maxBytes) {
      ScaffoldMessenger.of(context).showSnackBar(const SnackBar(content: Text('A phone name is 1 to 16 bytes.')));
      return;
    }
    await widget.book.setLabel(trimmed);
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([widget.book, _backups]),
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final FeederBook book = widget.book;
      return Scaffold(
        appBar: AppBar(
          toolbarHeight: 64,
          titleSpacing: 4,
          automaticallyImplyLeading: false,
          leading: IconButton(
            tooltip: 'Back',
            // Opened from a link with nothing under it, back goes to Feeders.
            onPressed: () => Navigator.of(context).canPop() ? Navigator.pop(context) : context.go('/'),
            icon: StrokeIcon(Glyph.back, color: c.text),
          ),
          title: Text(
            'Settings',
            style: TextStyle(fontSize: 22, fontWeight: Fonts.semibold, color: c.text),
          ),
        ),
        body: ListView(
          padding: const EdgeInsets.all(Space.s4),
          children: [
            const SectionLabel('Appearance'),
            const SizedBox(height: Space.s2),
            SegmentedButton<Appearance>(
              segments: const [
                ButtonSegment(value: Appearance.system, label: Text('System')),
                ButtonSegment(value: Appearance.light, label: Text('Light')),
                ButtonSegment(value: Appearance.dark, label: Text('Dark')),
              ],
              selected: {book.appearance},
              onSelectionChanged: (chosen) => unawaited(book.setAppearance(chosen.first)),
            ),
            const SizedBox(height: Space.s4),
            const SectionLabel('This phone'),
            const SizedBox(height: Space.s2),
            Panel(
              child: Column(
                children: [
                  ListTile(
                    title: const Text('Name on feeders'),
                    subtitle: Text(book.label),
                    trailing: StrokeIcon(Glyph.chevron, size: 20, color: c.textMuted),
                    onTap: () => unawaited(_rename()),
                  ),
                  ListTile(
                    title: const Text('About Wellfed'),
                    trailing: StrokeIcon(Glyph.chevron, size: 20, color: c.textMuted),
                    onTap: () => unawaited(
                      Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => const AboutScreen())),
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(height: Space.s4),
            const SectionLabel('Firmware backups'),
            const SizedBox(height: Space.s2),
            Panel(
              child: Column(
                children: [
                  if (_backups.state.isEmpty)
                    ListTile(
                      title: Text('None yet', style: TextStyle(color: c.textMuted)),
                    ),
                  for (final (feeder, path) in _backups.state)
                    ListTile(
                      title: Text(feeder.name),
                      trailing: IconButton(
                        tooltip: "Share ${feeder.name}'s backup",
                        icon: const Icon(Icons.share_outlined),
                        onPressed: () => unawaited(_backups.share(path)),
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
