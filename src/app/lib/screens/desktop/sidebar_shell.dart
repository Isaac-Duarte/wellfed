import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_svg/flutter_svg.dart';

import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';

/// Where the sidebar sends the window. The shell names a place, and the app
/// turns it into a route, so this file knows no paths.
sealed class Place {
  const Place();
}

final class OverviewPlace extends Place {
  const OverviewPlace();
}

final class FeederPlace extends Place {
  const FeederPlace(this.identity);

  final String identity;
}

final class AddPlace extends Place {
  const AddPlace();
}

final class SettingsPlace extends Place {
  const SettingsPlace();
}

/// The desktop window: the feeders down a sidebar, and the page beside it.
/// Before there is a feeder there is nothing to list, so the page (Welcome,
/// then a way to add one) takes the whole window, and Welcome brings its own
/// panel where the sidebar will be.
class SidebarShell extends StatelessWidget {
  const SidebarShell({
    super.key,
    required this.fleet,
    required this.at,
    required this.go,
    required this.back,
    required this.child,
  });

  final FeederFleet fleet;

  /// The place showing, for the sidebar's highlight; `null` inside a flow.
  final Place? at;
  final void Function(Place place) go;

  /// Step back a page, if there is one to go back to.
  final VoidCallback back;
  final Widget child;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: fleet,
    builder: (context, _) {
      final List<FeederController> feeders = fleet.all.toList();
      // Welcome is the window itself before there is a feeder, with its own
      // panel where the sidebar will be; every other page reads as a column.
      final bool welcome = feeders.isEmpty && at is OverviewPlace;
      final Widget page = ColoredBox(
        color: context.colors.background,
        child: welcome ? child : CenteredColumn(child: child),
      );
      // The sidebar sits outside every page's Scaffold, so it brings its own
      // Material for the theme's text and ink.
      return CallbackShortcuts(
        bindings: _shortcuts(feeders),
        child: Focus(
          autofocus: true,
          child: feeders.isEmpty
              ? page
              : Material(
                  color: context.colors.background,
                  child: Row(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      _Sidebar(feeders: feeders, at: at, go: go),
                      Expanded(child: page),
                    ],
                  ),
                ),
        ),
      );
    },
  );

  /// Ctrl+1 to Ctrl+9 open the feeders in the sidebar's order, Ctrl+0 the
  /// overview (Welcome before there is a feeder); Ctrl+N adds one, Ctrl+,
  /// opens settings and Escape steps back. On a Mac each is ⌘ in place of
  /// Ctrl, as in every Mac app, except ⌘,: the app menu's Settings… owns it
  /// there, so one press never opens settings twice.
  Map<ShortcutActivator, VoidCallback> _shortcuts(List<FeederController> feeders) => {
    const SingleActivator(LogicalKeyboardKey.escape): back,
    if (defaultTargetPlatform != TargetPlatform.macOS)
      _command(LogicalKeyboardKey.comma): () => go(const SettingsPlace()),
    _command(LogicalKeyboardKey.keyN): () => go(const AddPlace()),
    _command(LogicalKeyboardKey.digit0): () => go(const OverviewPlace()),
    for (final (i, feeder) in feeders.take(_digits.length).indexed)
      _command(_digits[i]): () => go(FeederPlace(feeder.state.known.identity)),
  };

  /// `key` with the platform's command modifier.
  static SingleActivator _command(LogicalKeyboardKey key) => defaultTargetPlatform == TargetPlatform.macOS
      ? SingleActivator(key, meta: true)
      : SingleActivator(key, control: true);

  static const List<LogicalKeyboardKey> _digits = [
    LogicalKeyboardKey.digit1,
    LogicalKeyboardKey.digit2,
    LogicalKeyboardKey.digit3,
    LogicalKeyboardKey.digit4,
    LogicalKeyboardKey.digit5,
    LogicalKeyboardKey.digit6,
    LogicalKeyboardKey.digit7,
    LogicalKeyboardKey.digit8,
    LogicalKeyboardKey.digit9,
  ];
}

class _Sidebar extends StatelessWidget {
  const _Sidebar({required this.feeders, required this.at, required this.go});

  static const width = 264.0;

  final List<FeederController> feeders;
  final Place? at;
  final void Function(Place place) go;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final Place? at = this.at;
    return Container(
      width: width,
      decoration: BoxDecoration(
        color: c.surface,
        border: Border(right: BorderSide(color: c.line)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const _Brand(),
          Expanded(
            child: ListView(
              padding: const EdgeInsets.symmetric(horizontal: Space.s3),
              children: [
                _Item(
                  label: 'Overview',
                  leading: _Glyph(Icons.space_dashboard_outlined, selected: at is OverviewPlace),
                  selected: at is OverviewPlace,
                  onTap: () => go(const OverviewPlace()),
                ),
                const _Heading('Feeders'),
                for (final feeder in feeders)
                  _FeederItem(
                    feeder: feeder,
                    selected: at is FeederPlace && at.identity == feeder.state.known.identity,
                    onTap: () => go(FeederPlace(feeder.state.known.identity)),
                  ),
              ],
            ),
          ),
          Divider(color: c.line),
          Padding(
            padding: const EdgeInsets.all(Space.s3),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                _Item(
                  label: 'Add a feeder',
                  leading: StrokeIcon(Glyph.add, size: 20, color: at is AddPlace ? c.accent : c.textMuted),
                  selected: at is AddPlace,
                  onTap: () => go(const AddPlace()),
                ),
                _Item(
                  label: 'Settings',
                  leading: _Glyph(Icons.settings_outlined, selected: at is SettingsPlace),
                  selected: at is SettingsPlace,
                  onTap: () => go(const SettingsPlace()),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _Brand extends StatelessWidget {
  const _Brand();

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(20, 20, 20, Space.s4),
    child: Row(
      children: [
        SvgPicture.asset(
          context.isDark ? 'assets/brand/mark.svg' : 'assets/brand/mark-on-light.svg',
          width: 28,
          height: 28,
          excludeFromSemantics: true,
        ),
        const SizedBox(width: 10),
        Text(
          'Wellfed',
          style: TextStyle(fontSize: 18, fontWeight: Fonts.display, letterSpacing: -0.3, color: context.colors.text),
        ),
      ],
    ),
  );
}

class _Heading extends StatelessWidget {
  const _Heading(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(Space.s3, 20, Space.s3, Space.s2),
    child: Text(
      text.toUpperCase(),
      style: TextStyle(
        fontSize: Sizes.xs,
        fontWeight: Fonts.semibold,
        letterSpacing: 0.6,
        color: context.colors.textMuted,
      ),
    ),
  );
}

class _Glyph extends StatelessWidget {
  const _Glyph(this.icon, {required this.selected});

  final IconData icon;
  final bool selected;

  @override
  Widget build(BuildContext context) =>
      Icon(icon, size: 20, color: selected ? context.colors.accent : context.colors.textMuted);
}

/// One row of the sidebar: highlighted in the accent's soft tint when its
/// page is showing.
class _Item extends StatelessWidget {
  const _Item({required this.label, required this.leading, required this.selected, required this.onTap, this.trailing});

  final String label;
  final Widget leading;
  final Widget? trailing;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 1),
      child: Semantics(
        selected: selected,
        button: true,
        child: Material(
          color: selected ? c.accentSoft : Colors.transparent,
          borderRadius: BorderRadius.circular(10),
          clipBehavior: Clip.antiAlias,
          child: InkWell(
            onTap: onTap,
            hoverColor: c.sunken,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: Space.s3, vertical: 10),
              child: Row(
                children: [
                  SizedBox.square(dimension: 20, child: Center(child: leading)),
                  const SizedBox(width: Space.s3),
                  Expanded(
                    child: Text(
                      label,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: Sizes.base,
                        fontWeight: selected ? Fonts.semibold : Fonts.medium,
                        color: selected ? c.accent : c.text,
                      ),
                    ),
                  ),
                  if (trailing case final trailing?) ...[const SizedBox(width: Space.s2), trailing],
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A feeder in the sidebar: whether it answers, and whether its hopper
/// is low, at a glance.
class _FeederItem extends StatelessWidget {
  const _FeederItem({required this.feeder, required this.selected, required this.onTap});

  final FeederController feeder;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: feeder,
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final FeederState state = feeder.state;
      final Color dot = switch (state.connection) {
        Connection.online => c.ok,
        Connection.offline => c.fault,
        Connection.otherBuild || Connection.otherVersion => c.warning,
        Connection.looking || Connection.connecting => c.lineStrong,
      };
      return _Item(
        label: state.known.name,
        selected: selected,
        onTap: onTap,
        leading: Container(
          width: 8,
          height: 8,
          decoration: BoxDecoration(color: dot, shape: BoxShape.circle),
        ),
        trailing: switch (state.connection) {
          Connection.offline => Text(
            'Offline',
            style: TextStyle(fontSize: Sizes.xs, color: c.textMuted),
          ),
          Connection.otherBuild => Text(
            'Different build',
            style: TextStyle(fontSize: Sizes.xs, color: c.textMuted),
          ),
          Connection.otherVersion => Text(
            'Needs another app',
            style: TextStyle(fontSize: Sizes.xs, color: c.textMuted),
          ),
          _ => null,
        },
      );
    },
  );
}
