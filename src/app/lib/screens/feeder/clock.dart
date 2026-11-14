import 'dart:async';

import 'package:clock/clock.dart';
import 'package:flutter/material.dart' hide Card;
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:wellfed/screens/feeder/ask_ntp.dart';
import 'package:wellfed/screens/feeder/today.dart' show clockWords;
import 'package:wellfed/screens/feeder/zone_picker.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/clock/clock.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/replies.dart';
import 'package:wellfed/widgets/settings_list.dart';
import 'package:wellfed/widgets/when.dart';

/// The feeder's clock (13.4): what it reads and how it was set, its zone, and
/// where it asks for the time.
///
/// The zone is this phone's, any IANA zone the owner picks, or none. With
/// none the feeder keeps the offset a phone's sync last gave it; the session
/// gives a feeder this phone's zone only just after pairing, so a zone
/// cleared here stays cleared.
class ClockScreen extends StatefulWidget {
  const ClockScreen({super.key, required this.feeder});

  final FeederController feeder;

  @override
  State<ClockScreen> createState() => _ClockScreenState();
}

class _ClockScreenState extends State<ClockScreen> {
  bool _asking = false;

  @override
  void initState() {
    super.initState();
    // The session keeps the answer: it comes back here as the card.
    unawaited(widget.feeder.ask(const FeederRequest.getTime()));
  }

  Future<void> _time({ZoneChange? zone, NtpChoice? ntp}) async {
    final TimeInfo? time = widget.feeder.state.card?.time;
    if (time == null) return;
    setState(() => _asking = true);
    final FeederReply reply = await widget.feeder.ask(
      FeederRequest.putTime(base: time.generation, zone: zone, ntp: ntp),
    );
    if (!mounted) return;
    showProblem(context, reply);

    // The feeder writes no event for a time setting, so nothing else would
    // tell the card: its choices would stay as they were, and its old
    // generation would get the next change refused as stale. The choices
    // wait for the read, as a second tap would go against the old one.
    await widget.feeder.ask(const FeederRequest.getTime());
    if (mounted) setState(() => _asking = false);
  }

  Future<void> _pickZone() async {
    final String? zone = await Navigator.of(context).push<String>(
      MaterialPageRoute<String>(
        builder: (_) => FormWidth(
          child: ZonePickerScreen(zones: widget.feeder.timeZones(), current: widget.feeder.state.card?.time?.phoneZone),
        ),
      ),
    );
    if (zone != null && mounted) await _time(zone: ZoneChange.named(name: zone));
  }

  Future<void> _address() async {
    final String? address = await askNtpServer(context);
    if (address != null && mounted) await _time(ntp: NtpChoice.address(address: address));
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.feeder,
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final Card? card = widget.feeder.state.card;
      final TimeInfo? time = card?.time;
      final bool can = !_asking && (time?.writable ?? false);

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
            'Clock',
            style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
          ),
        ),
        body: ListView(
          padding: EdgeInsets.fromLTRB(Space.s4, Space.s3, Space.s4, Space.s4 + MediaQuery.paddingOf(context).bottom),
          children: [
            _Readout(feeder: widget.feeder),
            const SizedBox(height: Space.s5),
            SettingsSection(
              'Time zone',
              Column(
                children: [
                  SettingsTile(
                    first: true,
                    child: Row(children: [Expanded(child: _ZoneWords(time))]),
                  ),
                  if (time case TimeInfo(:final String phoneZone, sameAsPhone: false))
                    _Action(
                      "Use this $deviceName's zone · ${placeName(phoneZone)}",
                      onTap: can ? () => unawaited(_time(zone: const ZoneChange.phone())) : null,
                    ),
                  _Action('Choose a zone…', onTap: can ? () => unawaited(_pickZone()) : null),
                  if (time?.zone != null)
                    _Action(
                      'Clear the zone',
                      onTap: can ? () => unawaited(_time(zone: const ZoneChange.clear())) : null,
                    ),
                ],
              ),
            ),
            SettingsSection(
              'Get the time from',
              Column(
                children: [
                  _Source(
                    'Your router',
                    "Asks your router's time server",
                    first: true,
                    chosen: time?.ntp is NtpView_Gateway,
                    onTap: can ? () => unawaited(_time(ntp: const NtpChoice.gateway())) : null,
                  ),
                  _Source(
                    'A time server',
                    switch (time?.ntp) {
                      NtpView_Address(:final String ip, :final int port) => '$ip:$port · tap to change',
                      _ => 'An address you choose',
                    },
                    chosen: time?.ntp is NtpView_Address,
                    onTap: can ? () => unawaited(_address()) : null,
                  ),
                  _Source(
                    'Phones only',
                    "Set by an owner's app when it checks in",
                    chosen: time?.ntp is NtpView_Off,
                    onTap: can ? () => unawaited(_time(ntp: const NtpChoice.off())) : null,
                  ),
                ],
              ),
              last: true,
            ),
          ],
        ),
      );
    },
  );
}

/// The feeder's time right now, large and ticking, and how far to trust it.
///
/// The card's clock is a reading; between readings the time runs on from it
/// by this phone's clock, as the session's own reckoning does.
class _Readout extends StatefulWidget {
  const _Readout({required this.feeder});

  final FeederController feeder;

  @override
  State<_Readout> createState() => _ReadoutState();
}

class _ReadoutState extends State<_Readout> {
  final SecondTicker _ticker = SecondTicker();

  /// The last reading, and when this phone saw it.
  int? _readMs;
  DateTime _seenAt = clock.now();

  @override
  void dispose() {
    _ticker.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: _ticker,
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final FeederController feeder = widget.feeder;
      final Card? card = feeder.state.card;
      final TimeInfo? time = card?.time;
      // An offline card's clock is an old reading, not the feeder's time now.
      final bool current = feeder.state.cardIsCurrent;
      final PlatformInt64? read = current ? time?.clockMs : null;
      if (read != _readMs) {
        _readMs = read;
        _seenAt = clock.now();
      }
      final int? now = read == null ? null : read + clock.now().difference(_seenAt).inMilliseconds;
      final int? error = current ? time?.errorMs : null;
      final String? zone = time?.zoneNow?.abbreviation;

      final (Color dot, String status) = switch (card) {
        _ when !current => (c.textMuted, "Can't reach the feeder right now"),
        null => (c.textMuted, 'Reading…'),
        Card(clockSet: false) => (c.warning, 'Not set: meals wait until it is'),
        final card => (c.ok, clockWords(context, card)),
      };

      return Panel(
        padding: const EdgeInsets.all(Space.s4),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              'Feeder time right now',
              style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
            ),
            const SizedBox(height: Space.s1),
            Text.rich(
              TextSpan(
                children: [
                  TextSpan(
                    text: now == null ? '—' : secondsText(context, DateTime.fromMillisecondsSinceEpoch(now)),
                    style: TextStyle(
                      fontSize: 36,
                      fontWeight: Fonts.semibold,
                      color: c.text,
                      fontFeatures: const [FontFeature.tabularFigures()],
                    ),
                  ),
                  if (now != null && zone != null)
                    TextSpan(
                      text: '  $zone',
                      style: TextStyle(fontSize: Sizes.md, color: c.textMuted),
                    ),
                ],
              ),
            ),
            if (now != null)
              Text(
                dayText(DateTime.fromMillisecondsSinceEpoch(now)),
                style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
              ),
            const SizedBox(height: Space.s3),
            Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Padding(
                  padding: const EdgeInsets.only(top: 6),
                  child: Container(
                    width: 8,
                    height: 8,
                    decoration: BoxDecoration(color: dot, shape: BoxShape.circle),
                  ),
                ),
                const SizedBox(width: Space.s2),
                Expanded(
                  child: Text(
                    [status, if (error != null) 'within ${_errorText(error)}'].join(' · '),
                    style: TextStyle(fontSize: Sizes.base, color: c.text),
                  ),
                ),
              ],
            ),
          ],
        ),
      );
    },
  );

  static String _errorText(int errorMs) => errorMs < 1000
      ? '1 s'
      : errorMs < 60000
      ? '${errorMs ~/ 1000} s'
      : '${errorMs ~/ 60000} min';
}

/// `3:00:12 PM`, or `15:00:12` where the phone uses a 24-hour clock.
String secondsText(BuildContext context, DateTime at) {
  final String ss = at.second.toString().padLeft(2, '0');
  final String mm = at.minute.toString().padLeft(2, '0');
  if (MediaQuery.alwaysUse24HourFormatOf(context)) return '${at.hour.toString().padLeft(2, '0')}:$mm:$ss';
  final int h = at.hour % 12 == 0 ? 12 : at.hour % 12;
  final MaterialLocalizations words = MaterialLocalizations.of(context);
  return '$h:$mm:$ss ${at.hour < 12 ? words.anteMeridiemAbbreviation : words.postMeridiemAbbreviation}';
}

/// The zone the feeder keeps, in words: a place when it is this phone's.
class _ZoneWords extends StatelessWidget {
  const _ZoneWords(this.time);

  final TimeInfo? time;

  @override
  Widget build(BuildContext context) {
    final TimeInfo? time = this.time;
    final ZoneView? zone = time?.zoneNow;
    if (time == null) return const SettingsWords('Reading…');
    if (zone == null) {
      return const SettingsWords('None kept', 'It uses the offset a phone last gave it, which a power cut loses');
    }
    final String offset = utcOffsetText(zone.offsetS);
    final String? phone = time.phoneZone;
    return SettingsWords(
      time.sameAsPhone == true && phone != null
          ? '${placeName(phone)} · ${zone.abbreviation}'
          : '${zone.abbreviation} · $offset',
      [
        if (time.sameAsPhone == true) 'Same as this $deviceName' else offset,
        if (zone.seasonal) 'follows daylight saving',
      ].join(' · '),
    );
  }
}

/// A change to the zone, as a row that says what it does.
class _Action extends StatelessWidget {
  const _Action(this.words, {required this.onTap});

  final String words;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) => SettingsTile(
    first: false,
    onTap: onTap,
    child: Row(
      children: [
        Expanded(
          child: Text(
            words,
            style: TextStyle(fontSize: 15, fontWeight: Fonts.semibold, color: context.colors.accent),
          ),
        ),
      ],
    ),
  );
}

/// One place the feeder can get the time from; the chosen one is ticked.
class _Source extends StatelessWidget {
  const _Source(this.name, this.note, {required this.chosen, required this.onTap, this.first = false});

  final String name;
  final String note;
  final bool chosen;
  final VoidCallback? onTap;
  final bool first;

  @override
  Widget build(BuildContext context) => Semantics(
    inMutuallyExclusiveGroup: true,
    checked: chosen,
    child: SettingsTile(
      first: first,
      onTap: onTap,
      child: Row(
        children: [
          Expanded(child: SettingsWords(name, note)),
          const SizedBox(width: Space.s2),
          if (chosen)
            StrokeIcon(Glyph.check, size: 20, color: context.colors.accent, weight: 2.2)
          else
            const SizedBox(width: 20),
        ],
      ),
    ),
  );
}

/// The place an IANA zone is named for: `America/Denver` → `Denver`.
String placeName(String zone) => zone.split('/').last.replaceAll('_', ' ');

/// `UTC−7`, `UTC+5:30`, or `UTC`.
String utcOffsetText(int seconds) {
  if (seconds == 0) return 'UTC';
  final String sign = seconds < 0 ? '−' : '+';
  final int minutes = seconds.abs() ~/ 60;
  final int m = minutes % 60;
  return 'UTC$sign${minutes ~/ 60}${m == 0 ? '' : ':${m.toString().padLeft(2, '0')}'}';
}
