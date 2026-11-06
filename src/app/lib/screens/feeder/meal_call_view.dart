part of 'meal_call.dart';

/// The two round buttons share one spot, so the stop lands where the record was.
const double _ring = 104;
const double _button = 88;

/// What the feeder holds, and a play button once there is something to hear.
class _ClipCard extends StatelessWidget {
  const _ClipCard({required this.call, required this.reading, required this.offerPlay, required this.onPlay});

  final CallInfo? call;
  final bool reading;
  final bool offerPlay;
  final VoidCallback? onPlay;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final (String title, String? length) = switch (call?.clip) {
      CallClip_Stored() => ('Your recording', '${((call?.ms ?? 0) / 1000).toStringAsFixed(1)} s'),
      CallClip_Receiving() => ("An upload didn't finish", null),
      CallClip_None() => ('No recording yet', null),
      null => (reading ? 'Reading…' : "Can't read it right now", null),
    };
    final bool stored = call?.clip is CallClip_Stored;

    return Panel(
      padding: const EdgeInsets.all(Space.s4),
      child: Row(
        children: [
          Container(
            width: 48,
            height: 48,
            decoration: BoxDecoration(
              color: stored ? c.accentSoft : c.sunken,
              borderRadius: BorderRadius.circular(Radii.icon),
            ),
            alignment: Alignment.center,
            child: StrokeIcon(Glyph.wave, size: 26, color: stored ? c.accent : c.textMuted, weight: 2),
          ),
          const SizedBox(width: 14),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  title,
                  style: TextStyle(fontSize: Sizes.md, fontWeight: Fonts.semibold, color: c.text),
                ),
                if (length != null)
                  Text(
                    length,
                    style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                  ),
              ],
            ),
          ),
          if (offerPlay)
            IconButton(
              tooltip: 'Play on the feeder',
              onPressed: onPlay,
              style: IconButton.styleFrom(
                backgroundColor: c.accent,
                disabledBackgroundColor: c.sunken,
                minimumSize: const Size.square(48),
              ),
              icon: StrokeIcon(Glyph.play, size: 22, color: onPlay == null ? c.textMuted : c.accentText, weight: 2),
            ),
        ],
      ),
    );
  }
}

/// The main action: a round record button and its limit.
class _RecordButton extends StatelessWidget {
  const _RecordButton({required this.onRecord});

  final VoidCallback? onRecord;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final bool on = onRecord != null;
    return Column(
      children: [
        SizedBox.square(
          dimension: _ring,
          child: Center(
            child: _Round(
              tooltip: 'Record on this $deviceName',
              color: on ? c.accent : c.sunken,
              onTap: onRecord,
              child: StrokeIcon(Glyph.mic, size: 38, color: on ? c.accentText : c.textMuted, weight: 2),
            ),
          ),
        ),
        const SizedBox(height: Space.s2),
        Text(
          'Tap to record',
          style: TextStyle(fontSize: Sizes.md, fontWeight: Fonts.semibold, color: on ? c.text : c.textMuted),
        ),
        Text(
          'Up to ${CallRecording.most.inSeconds} s',
          style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
        ),
      ],
    );
  }
}

/// Recording: the same spot turns red, a ring fills to the limit, and a tap
/// stops and sends.
class _Recording extends StatelessWidget {
  const _Recording({required this.seconds, required this.onStop});

  final int seconds;
  final VoidCallback onStop;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final int most = CallRecording.most.inSeconds;
    return Column(
      children: [
        SizedBox.square(
          dimension: _ring,
          child: Stack(
            alignment: Alignment.center,
            children: [
              SizedBox.square(
                dimension: _ring,
                child: CircularProgressIndicator(
                  value: seconds / most,
                  strokeWidth: 4,
                  color: c.fault,
                  backgroundColor: c.faultSoft,
                ),
              ),
              _Round(
                tooltip: 'Stop and send',
                color: c.fault,
                // Stop sits where Record was: a quick second tap must not send
                // a near-empty clip over the one stored.
                onTap: seconds >= 1 ? onStop : null,
                child: StrokeIcon(Glyph.stop, size: 30, color: c.background, weight: 2.4),
              ),
            ],
          ),
        ),
        const SizedBox(height: Space.s2),
        Text(
          '$seconds s',
          style: TextStyle(
            fontSize: Sizes.xl,
            fontWeight: Fonts.display,
            color: c.text,
            fontFeatures: const [FontFeature.tabularFigures()],
          ),
        ),
        Text(
          'Recording · stops at $most s',
          style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
        ),
      ],
    );
  }
}

/// A round button with its name for a screen reader, as it shows only a glyph.
class _Round extends StatelessWidget {
  const _Round({required this.tooltip, required this.color, required this.onTap, required this.child});

  final String tooltip;
  final Color color;
  final VoidCallback? onTap;
  final Widget child;

  @override
  Widget build(BuildContext context) => Tooltip(
    message: tooltip,
    child: Semantics(
      button: true,
      enabled: onTap != null,
      child: SizedBox.square(
        dimension: _button,
        child: Material(
          color: color,
          shape: const CircleBorder(),
          clipBehavior: Clip.antiAlias,
          child: InkWell(
            onTap: onTap,
            child: Center(child: child),
          ),
        ),
      ),
    ),
  );
}
