import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:wellfed/screens/feeder/update_end.dart';
import 'package:wellfed/screens/restore/restore_screen.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/src/rust/api/update.dart'
    show UpdateOffer, UpdateOffer_Blocked, UpdateOffer_Current, UpdateOffer_Newer, UpdateOffer_Older;
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart' show Connection;
import 'package:wellfed/state/update/update.dart';
import 'package:wellfed/state/update/update_state.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/replies.dart';

/// The firmware on this feeder and the update this app carries (12.1, 12.2).
/// One press offers it once. The run belongs to the feeder's controller, so
/// leaving and coming back shows it as it stands rather than a second offer.
class UpdatesScreen extends StatefulWidget {
  const UpdatesScreen({super.key, required this.feeder});

  final FeederController feeder;

  @override
  State<UpdatesScreen> createState() => _UpdatesScreenState();
}

class _UpdatesScreenState extends State<UpdatesScreen> {
  ImageCheck? _image;
  UpdateInfo? _state;
  bool _ended = false;

  FeederController get _feeder => widget.feeder;
  UpdateFlow get _flow => widget.feeder.update;

  @override
  void initState() {
    super.initState();
    _flow.addListener(_flowChanged);
    if (_flow.state is! UpdateUnavailable) unawaited(_read());
  }

  @override
  void dispose() {
    _flow.removeListener(_flowChanged);
    super.dispose();
  }

  // An update moves the floor this screen shows, so leaving its end reads it
  // again.
  void _flowChanged() {
    final ended = _flow.state is UpdateEnded;
    if (_ended && _flow.state is UpdateIdle) unawaited(_read());
    _ended = ended;
  }

  Future<void> _read() async {
    final ImageCheck image = await _feeder.updateImage();
    final FeederReply reply = await _feeder.ask(const FeederRequest.updateState());
    if (!mounted) return;
    setState(() {
      _image = image;
      if (reply is FeederReply_Update) _state = reply.update;
    });
  }

  Future<void> _cancel() async {
    final FeederReply reply = await _flow.cancel();
    if (mounted) showProblem(context, reply);
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([_flow, _feeder]),
    builder: (context, _) {
      final UpdateState update = _flow.state;
      if (update is UpdateEnded) return UpdateEndScreen(feeder: _feeder, ended: update);
      if (update is UpdateUnavailable) return const _Unavailable();
      final offering = update is UpdateIdle;
      return Scaffold(
        appBar: AppBar(
          toolbarHeight: 64,
          automaticallyImplyLeading: false,
          titleSpacing: 4,
          leading: IconButton(
            tooltip: 'Back',
            onPressed: () => Navigator.pop(context),
            icon: StrokeIcon(Glyph.back, color: context.colors.text),
          ),
          title: Text(
            offering ? 'Firmware' : 'Updating ${_feeder.state.known.name}',
            style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: context.colors.text),
          ),
        ),
        body: offering
            ? Builder(
                builder: (context) {
                  final ThemeTokens c = context.colors;
                  final String name = _feeder.state.known.name;
                  final Card? card = _feeder.state.card;
                  final String? build = card?.build;
                  final UpdateInfo? state = _state;
                  final (String, int)? offered = switch (_image) {
                    ImageCheck_Ready(:final String build, :final int version) => (build, version),
                    _ => null,
                  };
                  // What to offer is the rule every client shares, decided in
                  // Rust: never the build it runs, never an image at or below
                  // what it confirmed or to a feeder ahead of this app, and
                  // nothing while bank 0 blocks updates. Before the update
                  // state is read, or when its read failed, what runs and the
                  // skew still decide the first two.
                  final UpdateOffer? verdict = offered != null
                      ? _flow.offer(build: offered.$1, version: offered.$2, update: state)
                      : null;
                  final bool current = verdict is UpdateOffer_Current;
                  final bool older = verdict is UpdateOffer_Older;
                  // Why bank 0 would refuse this image, as the rule says it.
                  final Bank0Blocked? blocked = switch (verdict) {
                    UpdateOffer_Blocked(:final Bank0Blocked reason) => reason,
                    _ => null,
                  };
                  // A pre-revision build sends no status this app reads, so
                  // its open session stands in for a current card: the update
                  // is all it takes, and what this screen is for.
                  final bool legacy =
                      _feeder.state.skew == Skew.legacy && _feeder.state.connection == Connection.online;
                  final UpdateOffer_Newer? newer = switch (verdict) {
                    final UpdateOffer_Newer newer when _feeder.state.cardIsCurrent || legacy => newer,
                    _ => null,
                  };

                  return ListView(
                    padding: const EdgeInsets.all(Space.s4),
                    children: [
                      const SectionLabel('Firmware'),
                      const SizedBox(height: Space.s2),
                      FactList([
                        // The floor, not what runs: a feeder on bank 0 may run older.
                        if (state != null && state.minVersion > 0)
                          ('Newest confirmed version', Text('${state.minVersion}')),
                        if (build != null) ('Build', Text(shortBuild(build))),
                        if (card != null) ('Running from', Text('Bank ${card.bank}')),
                      ]),
                      const SizedBox(height: Space.s4),
                      if (blocked != null) ...[
                        Notice(
                          tone: Tone.warning,
                          glyph: Glyph.warning,
                          text: 'Updates are blocked. ${recoveryWords(name, blocked)}',
                        ),
                        const SizedBox(height: Space.s4),
                      ],
                      if (_image == null)
                        Text(
                          'Looking at what this app carries…',
                          style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                        )
                      else if (current)
                        Notice(tone: Tone.neutral, glyph: Glyph.check, text: '$name runs the version this app carries.')
                      else if (older)
                        Notice(
                          tone: Tone.warning,
                          glyph: Glyph.info,
                          text: '$name runs newer firmware than this app carries. Update this app.',
                        )
                      else if (offered == null)
                        const Notice(
                          tone: Tone.neutral,
                          glyph: Glyph.info,
                          text: 'This app carries no update it can install.',
                        )
                      else ...[
                        const SectionLabel('New'),
                        const SizedBox(height: Space.s2),
                        FactList([('Version', Text('${offered.$2}')), ('Build', Text(shortBuild(offered.$1)))]),
                        const SizedBox(height: Space.s3),
                        Text(
                          'Takes a few minutes. Feeding waits while it installs, and meals '
                          "due meanwhile run when it's done.",
                          style: TextStyle(fontSize: 15, height: 1.5, color: c.text),
                        ),
                        const SizedBox(height: Space.s2),
                        Text(
                          "If the new version doesn't start properly, $name goes to its "
                          'other copy of Wellfed by itself. That copy can feed, and may be '
                          'older than the one it runs now.',
                          style: TextStyle(fontSize: 15, height: 1.5, color: c.text),
                        ),
                        const SizedBox(height: Space.s4),
                        FilledButton(
                          onPressed: newer != null ? () => unawaited(_flow.start(newer)) : null,
                          child: Text('Install on $name'),
                        ),
                      ],
                      const SizedBox(height: Space.s5),
                      const SectionLabel('Original firmware'),
                      const SizedBox(height: Space.s2),
                      OutlinedButton(
                        onPressed: () => unawaited(
                          Navigator.of(
                            context,
                          ).push(MaterialPageRoute<void>(builder: (_) => RestoreScreen(newFlow: _feeder.newRestore))),
                        ),
                        child: const Text('Restore original firmware'),
                      ),
                    ],
                  );
                },
              )
            : Builder(
                builder: (context) {
                  final ThemeTokens c = context.colors;
                  final UpdateState update = _flow.state;
                  final UpdateRun? run = update is UpdateRun ? update : null;
                  final String name = _feeder.state.known.name;
                  final sending = update is UpdateSending;
                  final int length = run?.length ?? 0;
                  final bool accepted = length > 0;
                  final int percent = accepted ? ((run?.written ?? 0) * 100 ~/ length) : 0;
                  return ListView(
                    padding: const EdgeInsets.all(Space.s4),
                    children: [
                      _Line(
                        done: accepted || !sending,
                        active: !accepted && sending,
                        text: '$name accepted the update',
                      ),
                      _Line(
                        done: !sending,
                        active: accepted && sending,
                        text: sending ? ((run?.reconnecting ?? false) ? 'Reconnecting…' : 'Sending $percent%') : 'Sent',
                      ),
                      _Line(done: false, active: !sending, text: 'Restart on the new version'),
                      const _Line(done: false, active: false, text: 'Check it started properly, then keep it'),
                      const SizedBox(height: Space.s4),
                      Text(
                        'Keep the feeder plugged in. If this $deviceName loses the connection, the '
                        'feeder runs its other copy of Wellfed, which can feed and may be '
                        'older than the one it ran.',
                        style: TextStyle(fontSize: Sizes.sm, height: 1.5, color: c.textMuted),
                      ),
                      if (run?.holdRefused ?? false) ...[
                        const SizedBox(height: Space.s3),
                        const Notice(
                          tone: Tone.warning,
                          glyph: Glyph.warning,
                          text: 'Keep this app open until it finishes.',
                        ),
                      ],
                      if (sending) ...[
                        const SizedBox(height: Space.s4),
                        OutlinedButton(
                          onPressed: (run?.cancelling ?? false) ? null : () => unawaited(_cancel()),
                          child: const Text('Cancel update'),
                        ),
                      ],
                    ],
                  );
                },
              ),
      );
    },
  );
}

/// A build id as short as the screens show it.
String shortBuild(String build) => build.length > 12 ? build.substring(0, 12) : build;

class _Line extends StatelessWidget {
  const _Line({required this.done, required this.active, required this.text});

  final bool done;
  final bool active;
  final String text;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Space.s2),
      child: Row(
        children: [
          SizedBox(
            width: 24,
            height: 24,
            child: done
                ? StrokeIcon(Glyph.check, size: 20, color: c.ok)
                : active
                ? const CircularProgressIndicator(strokeWidth: 2)
                : null,
          ),
          const SizedBox(width: Space.s3),
          Expanded(
            child: Text(text, style: TextStyle(fontSize: 15, color: done || active ? c.text : c.textMuted)),
          ),
        ],
      ),
    );
  }
}

/// The firmware screen on a device that may not update a feeder.
class _Unavailable extends StatelessWidget {
  const _Unavailable();

  @override
  Widget build(BuildContext context) => StepScaffold(
    title: 'Firmware',
    content: [
      Notice(
        tone: Tone.fault,
        lead: "This $deviceName cannot update a feeder's firmware.",
        text: 'Only the Android app can. Update from an Android phone that owns this feeder.',
      ),
    ],
  );
}
