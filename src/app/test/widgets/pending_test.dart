import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/widgets/pending.dart';

import '../support/fakes.dart';

/// A switch on `card`, whose writes wait for the test to answer them.
class _Host {
  final card = ValueNotifier<bool>(false);

  /// The store's generation, for a card from a store that has one.
  final generation = ValueNotifier<int?>(null);
  final asks = <Completer<FeederReply>>[];

  Widget widget() => themed(
    Scaffold(
      body: ListenableBuilder(
        listenable: Listenable.merge([card, generation]),
        builder: (context, _) => Pending<bool>(
          card: card.value,
          generation: generation.value,
          builder: (context, shown, write) => Column(
            children: [
              Switch(
                value: shown,
                onChanged: (next) => unawaited(
                  write.ask(next, () {
                    final answer = Completer<FeederReply>();
                    asks.add(answer);
                    return answer.future;
                  }),
                ),
              ),
              TextButton(onPressed: () => write.hold(!shown), child: const Text('Hold')),
            ],
          ),
        ),
      ),
    ),
  );
}

bool _shown(WidgetTester tester) => tester.widget<Switch>(find.byType(Switch)).value;

Future<_Host> _host(WidgetTester tester) async {
  final host = _Host();
  addTearDown(host.card.dispose);
  addTearDown(host.generation.dispose);
  await tester.pumpWidget(host.widget());
  return host;
}

void main() {
  group('A pending control', () {
    testWidgets('a done the card never takes up ends at its generation', (tester) async {
      final _Host host = await _host(tester);
      host.generation.value = 5;
      await tester.pump();
      await tester.tap(find.byType(Switch));
      await settle(tester);

      host.asks.single.complete(const FeederReply.done(generation: 6));
      await settle(tester);
      expect(_shown(tester), isTrue);

      // The read shows the store at that generation without the change.
      host.generation.value = 6;
      await settle(tester);
      expect(_shown(tester), isFalse);
    });

    testWidgets('a done naming the generation the card already holds moves it back', (tester) async {
      final _Host host = await _host(tester);
      host.generation.value = 5;
      await tester.pump();
      await tester.tap(find.byType(Switch));
      await settle(tester);

      // The writer could not make it: the store stayed where it was.
      host.asks.single.complete(const FeederReply.done(generation: 5));
      await settle(tester);
      expect(_shown(tester), isFalse);
    });

    testWidgets('moves on tap and stays while the write is out', (tester) async {
      final _Host host = await _host(tester);

      await tester.tap(find.byType(Switch));
      await settle(tester);
      expect(_shown(tester), isTrue);
      expect(host.asks, hasLength(1));

      await tester.pump(const Duration(seconds: 5));
      expect(_shown(tester), isTrue);
    });

    testWidgets('never jumps back once the card agrees and the write is done', (tester) async {
      final _Host host = await _host(tester);
      await tester.tap(find.byType(Switch));
      await settle(tester);

      host.card.value = true;
      await tester.pump();
      expect(_shown(tester), isTrue);
      host.asks.single.complete(const FeederReply.done(generation: 6));
      await settle(tester);
      expect(_shown(tester), isTrue);
      expect(find.byType(SnackBar), findsNothing);

      // Settled: what the card says next shows.
      host.card.value = false;
      await settle(tester);
      expect(_shown(tester), isFalse);
    });

    testWidgets('a done before the card catches up waits for the card', (tester) async {
      final _Host host = await _host(tester);
      await tester.tap(find.byType(Switch));
      await settle(tester);

      host.asks.single.complete(const FeederReply.done(generation: 6));
      await settle(tester);
      expect(_shown(tester), isTrue);

      host.card.value = true;
      await settle(tester);
      expect(_shown(tester), isTrue);
    });

    testWidgets('moves back and says why when refused', (tester) async {
      final _Host host = await _host(tester);
      await tester.tap(find.byType(Switch));
      await settle(tester);

      host.asks.single.complete(const FeederReply.refused(refusal: FeederRefusal.busy()));
      await settle(tester);
      expect(_shown(tester), isFalse);
      expect(find.textContaining("It's busy"), findsOneWidget);
    });

    testWidgets('a write that throws reads as unanswered, and the card settles it after', (tester) async {
      final _Host host = await _host(tester);
      await tester.tap(find.byType(Switch));
      await settle(tester);
      host.asks.single.completeError(StateError('a bridge panic'));
      await settle(tester);
      expect(_shown(tester), isFalse);
      expect(find.textContaining('may or may not have'), findsOneWidget);

      host.card.value = true;
      await settle(tester);
      expect(_shown(tester), isTrue, reason: 'nothing is left out, so the card shows');
    });

    testWidgets('shows the card and says so when unanswered or never sent', (tester) async {
      final _Host host = await _host(tester);
      await tester.tap(find.byType(Switch));
      await settle(tester);
      host.asks.single.complete(const FeederReply.unanswered());
      await settle(tester);
      expect(_shown(tester), isFalse);
      expect(find.textContaining('may or may not have'), findsOneWidget);

      await tester.tap(find.byType(Switch));
      await settle(tester);
      host.asks.last.complete(const FeederReply.notAsked(reason: NotAsked.held));
      await settle(tester);
      expect(_shown(tester), isFalse);
      expect(find.textContaining("wasn't confirmed"), findsOneWidget);
    });

    testWidgets('stays usable while a write is out, and the newest ask is shown', (tester) async {
      final _Host host = await _host(tester);
      await tester.tap(find.byType(Switch));
      await settle(tester);
      await tester.tap(find.byType(Switch));
      await settle(tester);
      expect(host.asks, hasLength(2));
      expect(_shown(tester), isFalse);

      // The older write's answer does not settle the newer one.
      host.asks.first.complete(const FeederReply.done(generation: 6));
      await settle(tester);
      expect(_shown(tester), isFalse);
    });

    testWidgets('on then off, tapped quickly, never shows the first on the way', (tester) async {
      final _Host host = await _host(tester);
      await tester.tap(find.byType(Switch));
      await settle(tester);
      await tester.tap(find.byType(Switch));
      await settle(tester);
      expect(_shown(tester), isFalse);

      // The card agreed with the second tap all along, then took the first.
      host.card.value = true;
      await settle(tester);
      expect(_shown(tester), isFalse);
      host.asks.first.complete(const FeederReply.done(generation: 6));
      await settle(tester);
      expect(_shown(tester), isFalse);

      host.card.value = false;
      await settle(tester);
      expect(_shown(tester), isFalse);
      host.asks.last.complete(const FeederReply.done(generation: 7));
      await settle(tester);
      expect(_shown(tester), isFalse);

      // Settled: the card shows again.
      host.card.value = true;
      await settle(tester);
      expect(_shown(tester), isTrue);
    });

    testWidgets('holds a value without asking for it', (tester) async {
      final _Host host = await _host(tester);

      await tester.tap(find.text('Hold'));
      await settle(tester);
      expect(_shown(tester), isTrue);
      expect(host.asks, isEmpty);
    });
  });
}
