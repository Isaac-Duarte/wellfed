import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/state/wifi/networks.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/icons.dart';

/// The network the feeder should join, picked from what this phone hears.
/// The feeder's radio is 2.4 GHz only, so a network heard only on 5 GHz shows
/// but can't be picked. A hidden network, or a phone that lists nothing,
/// falls back to typing the name.
///
/// The name lives in `controller`, so the screen reads it as it read the
/// text field this replaces.
class NetworkPicker extends StatefulWidget {
  const NetworkPicker({super.key, required this.controller, required this.scan, required this.current, this.onChanged});

  final TextEditingController controller;

  /// The networks this phone hears.
  final Future<NetworkScan> Function() scan;

  /// The Wi-Fi this phone is on, when Android will say.
  final Future<String?> Function() current;

  final VoidCallback? onChanged;

  @override
  State<NetworkPicker> createState() => _NetworkPickerState();
}

/// The list as the sheet shows it.
@immutable
class _Heard {
  const _Heard({this.scanning = false, this.choices = const [], this.problem});

  final bool scanning;
  final List<NetworkChoice> choices;

  /// Why there is no list, if there isn't one.
  final String? problem;
}

class _NetworkPickerState extends State<NetworkPicker> {
  final ValueNotifier<_Heard> _heard = ValueNotifier(const _Heard(scanning: true));
  String? _current;

  /// The name is typed: a hidden network, or no list.
  bool _typing = false;

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  @override
  void dispose() {
    _heard.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    final String? current = await widget.current();
    if (!mounted) return;
    _current = current == null ? null : unquotedNetwork(current);
    final String? here = _current;
    if (here != null && here.isNotEmpty && widget.controller.text.isEmpty) {
      widget.controller.text = here;
      widget.onChanged?.call();
    }
    await _scan();
  }

  Future<void> _scan() async {
    _heard.value = _Heard(scanning: true, choices: _heard.value.choices);
    if (mounted) setState(() {});
    final NetworkScan scan = await widget.scan();
    if (!mounted) return;
    final _Heard heard = switch (scan) {
      NetworksSeen(:final networks) when networks.isEmpty && _current == null => const _Heard(
        problem: 'No networks found nearby. Type its name.',
      ),
      NetworksSeen(:final networks) => _Heard(choices: networkChoices(networks, current: _current)),
      NetworksUnread(:final problem) => _Heard(problem: _problemWords(problem)),
    };
    _heard.value = heard;
    setState(() {
      // With no list, the name can only be typed.
      if (heard.problem != null && widget.controller.text.isEmpty) _typing = true;
    });
  }

  Future<void> _open() async {
    final String? picked = await showModalBottomSheet<String>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      builder: (context) => ValueListenableBuilder(
        valueListenable: _heard,
        builder: (context, heard, _) =>
            _Sheet(heard: heard, selected: widget.controller.text, onScan: () => unawaited(_scan())),
      ),
    );
    if (!mounted || picked == null) return;
    setState(() {
      if (picked == _typeIt) {
        _typing = true;
        widget.controller.clear();
      } else {
        widget.controller.text = picked;
      }
    });
    widget.onChanged?.call();
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final _Heard heard = _heard.value;
    final String name = widget.controller.text;
    final bool listed = heard.choices.isNotEmpty;

    if (_typing) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          TextField(
            controller: widget.controller,
            autofocus: listed,
            autocorrect: false,
            textInputAction: TextInputAction.next,
            decoration: const InputDecoration(hintText: 'Network name'),
            onChanged: (_) => widget.onChanged?.call(),
          ),
          if (heard.problem case final problem?) ...[
            const SizedBox(height: Space.s1),
            Text(
              problem,
              style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
            ),
          ],
          if (listed)
            Align(
              alignment: Alignment.centerLeft,
              child: TextButton(
                style: TextButton.styleFrom(padding: EdgeInsets.zero),
                onPressed: () => setState(() => _typing = false),
                child: const Text('Choose from the list'),
              ),
            ),
        ],
      );
    }

    return Semantics(
      button: true,
      label: name.isEmpty ? 'Choose a network' : 'Network $name. Choose another',
      excludeSemantics: true,
      child: InkWell(
        onTap: () => unawaited(_open()),
        borderRadius: BorderRadius.circular(Radii.control),
        child: InputDecorator(
          decoration: InputDecoration(
            prefixIcon: Padding(
              padding: const EdgeInsets.only(left: 14, right: 10),
              child: StrokeIcon(Glyph.wifi, size: 20, color: c.text),
            ),
            prefixIconConstraints: const BoxConstraints(),
            suffixIcon: heard.scanning
                ? const Padding(
                    padding: EdgeInsets.all(14),
                    child: SizedBox.square(dimension: 16, child: CircularProgressIndicator(strokeWidth: 2)),
                  )
                : Padding(
                    padding: const EdgeInsets.only(right: 12),
                    child: RotatedBox(quarterTurns: 1, child: StrokeIcon(Glyph.chevron, size: 18, color: c.textMuted)),
                  ),
            suffixIconConstraints: const BoxConstraints(),
          ),
          child: Text(
            name.isEmpty ? 'Choose a network' : name,
            style: TextStyle(fontSize: Sizes.md, color: name.isEmpty ? c.textMuted : c.text),
          ),
        ),
      ),
    );
  }
}

/// The sheet's answer for "Type a network name": no network is named this,
/// as an SSID is at most 32 bytes.
const _typeIt = '\u0000type';

class _Sheet extends StatelessWidget {
  const _Sheet({required this.heard, required this.selected, required this.onScan});

  final _Heard heard;
  final String selected;
  final VoidCallback onScan;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return SafeArea(
      child: ConstrainedBox(
        constraints: BoxConstraints(maxHeight: MediaQuery.sizeOf(context).height * 0.75),
        child: ListView(
          shrinkWrap: true,
          padding: const EdgeInsets.only(bottom: Space.s4),
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(Space.s5, 0, Space.s3, Space.s2),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      'Choose a network',
                      style: TextStyle(fontSize: Sizes.lg, fontWeight: Fonts.semibold, color: c.text),
                    ),
                  ),
                  TextButton(onPressed: heard.scanning ? null : onScan, child: const Text('Scan again')),
                ],
              ),
            ),
            if (heard.scanning)
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: Space.s5, vertical: Space.s3),
                child: Row(
                  children: [
                    const SizedBox.square(dimension: 18, child: CircularProgressIndicator(strokeWidth: 2)),
                    const SizedBox(width: Space.s3),
                    Text(
                      'Looking for networks…',
                      style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                    ),
                  ],
                ),
              ),
            if (heard.problem case final problem?)
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: Space.s5, vertical: Space.s3),
                child: Text(
                  problem,
                  style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                ),
              ),
            for (final choice in heard.choices) _Row(choice, selected: choice.ssid == selected),
            _Choice(
              glyph: Glyph.add,
              glyphColor: c.accent,
              title: 'Type a network name',
              note: 'For a hidden network',
              onTap: () => Navigator.pop(context, _typeIt),
            ),
          ],
        ),
      ),
    );
  }
}

class _Row extends StatelessWidget {
  const _Row(this.choice, {required this.selected});

  final NetworkChoice choice;
  final bool selected;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final String note = [
      if (choice.current) "This phone's network",
      if (!choice.joinable) "5 GHz only: the feeder can't join" else if (!choice.secured) 'No password',
      if (choice.joinable) ?_strength(choice.level),
    ].join(' · ');
    return _Choice(
      glyph: Glyph.wifi,
      glyphColor: switch (choice.level) {
        _ when !choice.joinable => c.lineStrong,
        final int level when level < _weak => c.textMuted,
        _ => c.text,
      },
      title: choice.ssid,
      note: note,
      selected: selected,
      onTap: choice.joinable ? () => Navigator.pop(context, choice.ssid) : null,
    );
  }
}

/// The signal the picker calls weak, in dBm: the feeder's own threshold.
const _weak = -72;

String? _strength(int? level) => switch (level) {
  null => null,
  >= -60 => 'Strong',
  >= _weak => 'Fair',
  _ => 'Weak',
};

class _Choice extends StatelessWidget {
  const _Choice({
    required this.glyph,
    required this.glyphColor,
    required this.title,
    required this.note,
    required this.onTap,
    this.selected = false,
  });

  final Glyph glyph;
  final Color glyphColor;
  final String title;
  final String note;
  final VoidCallback? onTap;
  final bool selected;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final bool enabled = onTap != null;
    return Semantics(
      container: true,
      selected: selected,
      child: InkWell(
        onTap: onTap,
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: 56),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: Space.s5, vertical: Space.s2),
            child: Row(
              children: [
                StrokeIcon(glyph, size: 22, color: glyphColor),
                const SizedBox(width: Space.s4),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        title,
                        style: TextStyle(
                          fontSize: Sizes.md,
                          fontWeight: selected ? Fonts.semibold : Fonts.body,
                          color: enabled ? c.text : c.textMuted,
                        ),
                      ),
                      if (note.isNotEmpty)
                        Text(
                          note,
                          style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
                        ),
                    ],
                  ),
                ),
                if (selected) StrokeIcon(Glyph.check, size: 20, color: c.accent),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

String _problemWords(ScanProblem problem) => switch (problem) {
  ScanProblem.permission => 'Allow location for Wellfed to list the networks nearby. Or type its name.',
  ScanProblem.wifiOff => "Turn on this phone's Wi-Fi to list the networks nearby. Or type its name.",
  ScanProblem.locationOff => 'Turn on location to list the networks nearby. Or type its name.',
  ScanProblem.unavailable => "This phone didn't list the networks nearby. Type its name.",
};
