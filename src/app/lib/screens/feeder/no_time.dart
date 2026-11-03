import 'package:flutter/material.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/session.dart' as api;
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/replies.dart';

/// The feeder has no clock, so its meals wait: say so, and offer this phone's.
class NoTimeNotice extends StatefulWidget {
  const NoTimeNotice({super.key, required this.feeder});

  final FeederController feeder;

  @override
  State<NoTimeNotice> createState() => _NoTimeNoticeState();
}

class _NoTimeNoticeState extends State<NoTimeNotice> {
  bool _asking = false;

  Future<void> _sync() async {
    setState(() => _asking = true);
    final FeederReply reply = await widget.feeder.ask(const FeederRequest.syncClock());
    if (!mounted) return;
    setState(() => _asking = false);
    showProblem(context, reply);
  }

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      const Notice(
        tone: Tone.warning,
        lead: 'Meals are on hold.',
        text:
            "The feeder doesn't know the time yet, so it won't guess when to "
            'feed. Feed now still works.',
      ),
      const SizedBox(height: Space.s2),
      OutlinedButton(onPressed: _asking ? null : _sync, child: Text('Set the time from this $deviceName')),
    ],
  );
}

/// Whether the feeder holds its meals for want of a local offset: the clock
/// is right, but no zone is set, and the offset a phone gave it went with a
/// power cut. A session gives it this phone's as soon as it reads the time,
/// so this shows only until then, or when that sync was refused.
bool holdsForNoZone(api.Card card) {
  final TimeInfo? time = card.time;
  return card.clockSet && time != null && time.offsetS == null && (time.zone ?? '').isEmpty;
}

/// The clock is right but the feeder does not know local time, so its meals
/// wait: say so, and give it this phone's zone, or this phone's time.
class NoZoneNotice extends StatefulWidget {
  const NoZoneNotice({super.key, required this.feeder, required this.time});

  final FeederController feeder;
  final TimeInfo time;

  @override
  State<NoZoneNotice> createState() => _NoZoneNoticeState();
}

class _NoZoneNoticeState extends State<NoZoneNotice> {
  bool _asking = false;

  /// An owner stores the zone, which survives a power cut; anyone else gives
  /// this phone's time, whose offset lasts until the next one.
  Future<void> _fix() async {
    setState(() => _asking = true);
    final FeederRequest request = widget.time.writable
        ? FeederRequest.putTime(base: widget.time.generation, zone: const ZoneChange.phone())
        : const FeederRequest.syncClock();
    final FeederReply reply = await widget.feeder.ask(request);
    if (!mounted) return;
    showProblem(context, reply);

    await widget.feeder.ask(const FeederRequest.getTime());
    if (mounted) setState(() => _asking = false);
  }

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      const Notice(
        tone: Tone.warning,
        lead: 'Meals are on hold.',
        text:
            "The feeder has the right time but not your time zone, so it won't "
            'guess when a meal is due. Feed now still works.',
      ),
      const SizedBox(height: Space.s2),
      OutlinedButton(
        onPressed: _asking ? null : _fix,
        child: Text(widget.time.writable ? "Use this $deviceName's zone" : 'Set the time from this $deviceName'),
      ),
    ],
  );
}
