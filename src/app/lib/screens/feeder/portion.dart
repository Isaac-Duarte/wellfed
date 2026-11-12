import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/src/limits.g.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/ask_text.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/replies.dart';
import 'package:wellfed/widgets/when.dart';

/// What a portion weighs (13.4), so feeds and meals can say grams: set by
/// hand a gram at a time, or from weighing ten portions.
class PortionScreen extends StatefulWidget {
  const PortionScreen({super.key, required this.feeder});

  final FeederController feeder;

  @override
  State<PortionScreen> createState() => _PortionScreenState();
}

class _PortionScreenState extends State<PortionScreen> {
  /// Whole grams the steppers go between, inside the feeder's limits.
  static const int _gramsLeast = Limits.portionMgLeast ~/ 1000;
  static const int _gramsMost = Limits.portionMgMost ~/ 1000;

  /// What the ten-portion weighing divides by.
  static const _weighed = 10;

  bool _asking = false;

  @override
  void initState() {
    super.initState();
    // Read each time: another phone may have changed it. The state keeps it.
    unawaited(widget.feeder.loadCalibration());
  }

  /// One write, then a fresh reading of the portion.
  Future<void> _portion(PortionChange change) async {
    final CalibrationInfo? calibration = widget.feeder.state.calibration;
    if (calibration == null) return;
    setState(() => _asking = true);
    final FeederReply reply = await widget.feeder.ask(
      FeederRequest.putCalibration(base: calibration.generation, portion: change, bowl: calibration.bowl),
    );
    if (!mounted) return;
    setState(() => _asking = false);
    showProblem(context, reply);
    await widget.feeder.loadCalibration();
  }

  Future<void> _weigh() async {
    final String? grams = await askText(
      context,
      title: 'What did $_weighed portions weigh?',
      label: 'Grams',
      keyboard: const TextInputType.numberWithOptions(decimal: true),
    );
    if (grams != null && mounted) {
      await _portion(PortionChange.weighed(grams: grams, portions: _weighed));
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.feeder,
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final CalibrationInfo? calibration = widget.feeder.state.calibration;
      final int? mg = calibration?.portionMg;
      final int? grams = mg == null ? null : (mg / 1000).round();
      final bool can = !_asking && (calibration?.writable ?? false);
      void step(int next) => unawaited(_portion(PortionChange.milligrams(mg: next * 1000)));

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
            'Portion size',
            style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
          ),
        ),
        body: ListView(
          padding: EdgeInsets.fromLTRB(Space.s4, Space.s3, Space.s4, Space.s4 + MediaQuery.paddingOf(context).bottom),
          children: [
            Panel(
              padding: const EdgeInsets.symmetric(horizontal: Space.s4, vertical: Space.s5),
              child: Column(
                children: [
                  Text(
                    'One portion weighs',
                    style: TextStyle(fontSize: Sizes.base, color: c.textMuted),
                  ),
                  const SizedBox(height: Space.s3),
                  Row(
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      IconButton.outlined(
                        tooltip: 'A gram less',
                        onPressed: can && grams != null && grams > _gramsLeast ? () => step(grams - 1) : null,
                        icon: const StrokeIcon(Glyph.minus),
                      ),
                      Expanded(
                        child: Text(
                          switch (mg) {
                            _ when calibration == null => '…',
                            null => '— g',
                            final mg => '${gramsText(mg)} g',
                          },
                          textAlign: TextAlign.center,
                          style: TextStyle(fontSize: 40, fontWeight: Fonts.semibold, color: c.text),
                        ),
                      ),
                      IconButton.outlined(
                        tooltip: 'A gram more',
                        onPressed: can && (grams ?? 0) < _gramsMost ? () => step((grams ?? 9) + 1) : null,
                        icon: const StrokeIcon(Glyph.add),
                      ),
                    ],
                  ),
                  const SizedBox(height: Space.s3),
                  Text(
                    switch (mg) {
                      _ when calibration == null => 'Reading…',
                      null => 'Not measured yet. Feeds count portions until it is.',
                      final mg => 'A meal of 2 portions is about ${gramsText(mg * 2)} g.',
                    },
                    textAlign: TextAlign.center,
                    style: TextStyle(fontSize: Sizes.sm, height: 1.5, color: c.textMuted),
                  ),
                ],
              ),
            ),
            const SizedBox(height: Space.s5),
            const Padding(
              padding: EdgeInsets.only(left: Space.s1, bottom: Space.s2),
              child: SectionLabel('Measure it'),
            ),
            Panel(
              padding: const EdgeInsets.all(Space.s4),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const NumberedSteps([
                    'Empty the bowl.',
                    'Feed **$_weighed portions** with **Feed now**.',
                    'Weigh what came out, and enter it here.',
                  ]),
                  const SizedBox(height: Space.s4),
                  OutlinedButton(
                    onPressed: can ? () => unawaited(_weigh()) : null,
                    child: const Text('Enter what $_weighed portions weighed'),
                  ),
                ],
              ),
            ),
            const SizedBox(height: Space.s3),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: Space.s1),
              child: Text(
                'Every food weighs differently: measure again when you change it.',
                style: TextStyle(fontSize: Sizes.sm, height: 1.5, color: c.textMuted),
              ),
            ),
          ],
        ),
      );
    },
  );
}
