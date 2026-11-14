import 'package:flutter/material.dart';
import 'package:wellfed/screens/feeder/clock.dart' show placeName;
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';

/// Pick the feeder's time zone from [zones], IANA names, by typing part of a
/// place or area. Pops with the name chosen, or nothing when the owner backs
/// out. Whether the feeder can follow it is checked when it is sent.
class ZonePickerScreen extends StatefulWidget {
  const ZonePickerScreen({super.key, required this.zones, this.current});

  final List<String> zones;

  /// The phone's zone, listed first while nothing is typed.
  final String? current;

  @override
  State<ZonePickerScreen> createState() => _ZonePickerScreenState();
}

class _ZonePickerScreenState extends State<ZonePickerScreen> {
  String _typed = '';

  /// The zones matching what is typed: each word typed in the name, with
  /// spaces and underscores alike, in any case.
  List<String> get _shown {
    final List<String> words = _typed.toLowerCase().split(RegExp(r'[\s_/]+')).where((w) => w.isNotEmpty).toList();
    final String? current = widget.current;
    return [
      if (words.isEmpty && current != null && widget.zones.contains(current)) current,
      for (final String zone in widget.zones)
        if (words.every(zone.toLowerCase().replaceAll('_', ' ').contains) && (words.isNotEmpty || zone != current))
          zone,
    ];
  }

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final List<String> shown = _shown;
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
          'Time zone',
          style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
        ),
      ),
      body: Column(
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(Space.s4, Space.s2, Space.s4, Space.s2),
            child: TextField(
              autofocus: true,
              decoration: const InputDecoration(labelText: 'Search for a city or region'),
              onChanged: (typed) => setState(() => _typed = typed),
            ),
          ),
          Expanded(
            child: shown.isEmpty
                ? Padding(
                    padding: const EdgeInsets.all(Space.s5),
                    child: Text(
                      'No time zone matches. Try the nearest large city.',
                      textAlign: TextAlign.center,
                      style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                    ),
                  )
                : ListView.builder(
                    padding: EdgeInsets.only(bottom: Space.s4 + MediaQuery.paddingOf(context).bottom),
                    itemCount: shown.length,
                    itemBuilder: (context, i) => ListTile(
                      minTileHeight: 56,
                      title: Text(placeName(shown[i])),
                      subtitle: Text(
                        shown[i] == widget.current ? '${shown[i]} · this $deviceName' : shown[i],
                        style: TextStyle(color: c.textMuted),
                      ),
                      onTap: () => Navigator.pop(context, shown[i]),
                    ),
                  ),
          ),
        ],
      ),
    );
  }
}
