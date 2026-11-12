import 'package:flutter/material.dart' show FilledButton, SizedBox;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/updates.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/update.dart';
import 'package:wellfed/state/feeder/feeder.dart';

import '../support/fakes.dart';
import '../support/harness.dart';

const _newBuild = 'cd00000000000000000000000000000000000000000000000000000000000000';

Future<(FakeFeederPorts, FakeSession)> _show(
  WidgetTester tester, {
  ImageCheck image = const ImageCheck.ready(provisioning: ImageProvisioning.consumer, build: _newBuild, version: 7),
  String build = 'ab',
  Bank0Check bank0 = const Bank0Check.unchecked(),
  int minVersion = 6,
  Skew skew = Skew.same,
  bool card = true,
  bool stateRead = true,
}) async {
  final book = FakeBook()..feeders = [kitchen];
  final ports = FakeFeederPorts()..image = image;
  final feeder = FeederController(book, ports, kitchen);
  ports.controller = feeder;
  addTearDown(feeder.dispose);
  await tester.pump();
  ports.session.opened(skew: skew);
  if (card) ports.session.status(fakeCard(build: build));
  await tester.pump();
  ports.session.replies.add(
    stateRead
        ? FeederReply.update(
            update: UpdateInfo(
              phase: UpdatePhase.idle,
              bank: 1,
              minVersion: minVersion,
              version: 0,
              imageLen: 0,
              written: 0,
              bank0: bank0,
            ),
          )
        : const FeederReply.unanswered(),
  );
  await showTab(tester, UpdatesScreen(feeder: feeder));
  ports.session.requests.clear();
  return (ports, ports.session);
}

void main() {
  testWidgets('offers what the app carries, and nothing it does not', (tester) async {
    await _show(tester);
    expect(find.text('7'), findsOneWidget);
    expect(find.text('Install on Kitchen'), findsOneWidget);
  });

  testWidgets('an app with no image offers nothing', (tester) async {
    await _show(tester, image: const ImageCheck.absent());
    expect(find.textContaining('carries no update'), findsOneWidget);
    expect(find.text('Install on Kitchen'), findsNothing);
  });

  // The app sends no downgrade statement, and a newer feeder is never offered
  // this app's older firmware: the app is the side to update.
  for (final (String why, int minVersion, Skew skew) in [
    ("a confirmed version at the image's", 7, Skew.same),
    ("a confirmed version past the image's", 9, Skew.same),
    ('a newer API revision', 6, Skew.feederAhead),
  ]) {
    testWidgets('a feeder with $why is offered no downgrade, and told to update the app', (tester) async {
      await _show(tester, minVersion: minVersion, skew: skew);
      expect(find.textContaining('newer firmware than this app carries'), findsOneWidget);
      expect(find.text('Install on Kitchen'), findsNothing);
    });
  }

  // A pre-revision build answers the update path alone and sends no status
  // this app reads: its open session is enough for the offer.
  testWidgets('a pre-revision build is offered the update, with no status card', (tester) async {
    await _show(tester, skew: Skew.legacy, card: false);
    expect(find.text('Install on Kitchen'), findsOneWidget);
    expect(find.textContaining('newer firmware than this app carries'), findsNothing);
  });

  testWidgets('the running build is up to date', (tester) async {
    await _show(tester, build: _newBuild);
    expect(find.textContaining('runs the version'), findsOneWidget);
    expect(find.text('Install on Kitchen'), findsNothing);
  });

  // With no update state, what runs and the skew still say the feeder is
  // current or ahead; anything else shows the image, and offers it once the
  // state says what the feeder confirmed.
  group('before the update state answers', () {
    testWidgets('the running build is up to date', (tester) async {
      await _show(tester, build: _newBuild, stateRead: false);
      expect(find.textContaining('runs the version'), findsOneWidget);
      expect(find.text('Install on Kitchen'), findsNothing);
    });

    testWidgets('a feeder on a newer API revision is told to update the app', (tester) async {
      await _show(tester, skew: Skew.feederAhead, stateRead: false);
      expect(find.textContaining('newer firmware than this app carries'), findsOneWidget);
      expect(find.text('Install on Kitchen'), findsNothing);
    });

    testWidgets('another build shows the image, and offers nothing yet', (tester) async {
      await _show(tester, stateRead: false);
      expect(find.text('7'), findsOneWidget);
      await tester.scrollUntilVisible(find.text('Install on Kitchen'), 200);
      final FilledButton install = tester.widget(
        find.ancestor(of: find.text('Install on Kitchen'), matching: find.byType(FilledButton)),
      );
      expect(install.onPressed, isNull, reason: 'nothing says what it confirmed');
    });
  });

  testWidgets('one press sends once and shows its progress', (tester) async {
    final (FakeFeederPorts ports, _) = await _show(tester);

    await tapIn(tester, find.text('Install on Kitchen'));
    expect(ports.pushes, ['lan:4747']);
    expect(find.text('Install on Kitchen'), findsNothing);

    ports.progress.add(const UpdateProgress.sending(written: 62, length: 100));
    await settle(tester);
    expect(find.text('Sending 62%'), findsOneWidget);

    ports.progress.add(const UpdateProgress.activated());
    await settle(tester);
    expect(find.text('Cancel update'), findsNothing);

    ports.progress.add(const UpdateProgress.ended(end: UpdateEnd.confirmed()));
    await settle(tester);
    expect(find.text('Updated'), findsOneWidget);
    expect(ports.pushes, hasLength(1));
  });

  testWidgets('cancel goes once; refused or unanswered, it says so', (tester) async {
    final (FakeFeederPorts ports, FakeSession session) = await _show(tester);
    await tapIn(tester, find.text('Install on Kitchen'));
    ports.progress.add(const UpdateProgress.sending(written: 1, length: 100));
    await settle(tester);

    session.replies.add(const FeederReply.unanswered());
    await tapIn(tester, find.text('Cancel update'));
    expect(session.requests.single, const FeederRequest.cancelUpdate());
    expect(find.textContaining('may or may not have'), findsOneWidget);
  });

  testWidgets('leaving mid-send and coming back shows the run, not an offer', (tester) async {
    final (FakeFeederPorts ports, _) = await _show(tester);
    final FeederController feeder = ports.controller;
    await tapIn(tester, find.text('Install on Kitchen'));
    ports.progress.add(const UpdateProgress.sending(written: 62, length: 100));
    await settle(tester);

    await showTab(tester, const SizedBox());
    await showTab(tester, UpdatesScreen(feeder: feeder));

    expect(find.text('Install on Kitchen'), findsNothing);
    expect(find.text('Sending 62%'), findsOneWidget);
    expect(ports.pushes, hasLength(1));
  });

  testWidgets('an uncertain end stays on coming back, until the feeder settles it', (tester) async {
    final (FakeFeederPorts ports, FakeSession session) = await _show(tester);
    final FeederController feeder = ports.controller;
    await tapIn(tester, find.text('Install on Kitchen'));
    ports.progress.add(const UpdateProgress.ended(end: UpdateEnd.uncertain()));
    await settle(tester);

    await showTab(tester, const SizedBox());
    await showTab(tester, UpdatesScreen(feeder: feeder));
    expect(find.text('Install on Kitchen'), findsNothing);
    expect(find.textContaining('may or may not have'), findsOneWidget);

    session.replies.add(const FeederReply.unanswered());
    await tapIn(tester, find.text('Look again'));
    expect(find.textContaining("hasn't settled"), findsOneWidget);

    session.replies.add(
      const FeederReply.update(
        update: UpdateInfo(
          phase: UpdatePhase.idle,
          bank: 0,
          minVersion: 6,
          version: 0,
          imageLen: 0,
          written: 0,
          bank0: Bank0Check.unchecked(),
        ),
      ),
    );
    await tapIn(tester, find.text('Look again'));
    expect(find.text('Install on Kitchen'), findsOneWidget);
    expect(ports.pushes, hasLength(1));
  });

  testWidgets('the served build past its version has kept the update', (tester) async {
    final (FakeFeederPorts ports, FakeSession session) = await _show(tester);
    await tapIn(tester, find.text('Install on Kitchen'));
    ports.progress.add(const UpdateProgress.ended(end: UpdateEnd.unconfirmed()));
    await settle(tester);

    session.status(fakeCard(build: _newBuild));
    await settle(tester);
    session.replies.add(
      const FeederReply.update(
        update: UpdateInfo(
          phase: UpdatePhase.idle,
          bank: 1,
          minVersion: 7,
          version: 0,
          imageLen: 0,
          written: 0,
          bank0: Bank0Check.unchecked(),
        ),
      ),
    );
    await tapIn(tester, find.text('Look again'));
    expect(find.text('Updated'), findsOneWidget);

    session.replies.add(
      const FeederReply.update(
        update: UpdateInfo(
          phase: UpdatePhase.idle,
          bank: 1,
          minVersion: 7,
          version: 0,
          imageLen: 0,
          written: 0,
          bank0: Bank0Check.unchecked(),
        ),
      ),
    );
    await tapIn(tester, find.text('Done'));
    expect(find.text('7'), findsOneWidget, reason: 'the floor, read again');
  });

  // A reinstall of a build the feeder confirmed before skips the trial and
  // leaves the floor below the version. A trial build takes no feed until it
  // confirms, so the served build ready to feed has kept the update, as the
  // installer's confirm step reads it; one not ready stays unknown.
  testWidgets('the served build ready to feed has kept the update, its floor below', (tester) async {
    final (FakeFeederPorts ports, FakeSession session) = await _show(tester);
    await tapIn(tester, find.text('Install on Kitchen'));
    ports.progress.add(const UpdateProgress.ended(end: UpdateEnd.unconfirmed()));
    await settle(tester);
    const below = FeederReply.update(
      update: UpdateInfo(
        phase: UpdatePhase.idle,
        bank: 1,
        minVersion: 6,
        version: 0,
        imageLen: 0,
        written: 0,
        bank0: Bank0Check.unchecked(),
      ),
    );

    session.status(fakeCard(build: _newBuild, ready: false));
    await settle(tester);
    session.replies.add(below);
    await tapIn(tester, find.text('Look again'));
    expect(find.textContaining("hasn't settled"), findsOneWidget);
    expect(find.text('Updated'), findsNothing);

    session.status(fakeCard(build: _newBuild));
    await settle(tester);
    session.replies.add(below);
    await tapIn(tester, find.text('Look again'));
    expect(find.text('Updated'), findsOneWidget);
    expect(ports.pushes, hasLength(1), reason: 'settled by reading, never sent again');
  });

  testWidgets('an offer held by an earlier stop offers its cancel, once', (tester) async {
    final (FakeFeederPorts ports, FakeSession session) = await _show(tester);
    await tapIn(tester, find.text('Install on Kitchen'));
    ports.progress.add(
      const UpdateProgress.ended(
        end: UpdateEnd.refused(refusal: FeederRefusal.update(reason: UpdateRefusal.pending())),
      ),
    );
    await settle(tester);
    expect(find.textContaining('Let it finish'), findsNothing);

    session.replies.add(const FeederReply.refused(refusal: FeederRefusal.busy()));
    await tapIn(tester, find.text('Cancel the earlier update'));
    expect(session.requests.single, const FeederRequest.cancelUpdate());
    expect(find.textContaining('busy'), findsOneWidget);

    await tapIn(tester, find.text('Cancel the earlier update'));
    expect(find.text('Install on Kitchen'), findsOneWidget);
  });

  testWidgets('holds the process for the run, and says if refused', (tester) async {
    final (FakeFeederPorts ports, _) = await _show(tester);
    ports.holdGranted = false;

    await tapIn(tester, find.text('Install on Kitchen'));
    expect(ports.holds, 1);
    expect(find.textContaining('Keep this app open'), findsOneWidget);
    expect(ports.releases, 0);

    ports.progress.add(const UpdateProgress.ended(end: UpdateEnd.confirmed()));
    await settle(tester);
    expect(ports.releases, 1);
  });

  testWidgets('a run that asks where the feeder is hears what NSD says', (tester) async {
    final (FakeFeederPorts ports, _) = await _show(tester);
    await tapIn(tester, find.text('Install on Kitchen'));

    ports.progress.add(const UpdateProgress.ask(id: 4, ask: InstallAsk.locate(identity: 'aa')));
    await settle(tester);
    expect(ports.located, [(4, 'lan:4747')]);
  });

  testWidgets('a recovery copy that has not checked itself in blocks updates, and says how to fix it', (tester) async {
    await _show(tester, bank0: const Bank0Check.blocked(Bank0Blocked.unrecorded));
    expect(find.textContaining('Updates are blocked.'), findsOneWidget);
    expect(find.textContaining("hasn't checked itself in"), findsOneWidget);
    expect(find.textContaining('recovery copy once (a bench step)'), findsOneWidget);
    await tester.scrollUntilVisible(find.text('Install on Kitchen'), 200);
    final FilledButton install = tester.widget(
      find.ancestor(of: find.text('Install on Kitchen'), matching: find.byType(FilledButton)),
    );
    expect(install.onPressed, isNull, reason: 'the feeder would refuse it');
  });

  testWidgets('a recovery copy that matches its record blocks nothing', (tester) async {
    await _show(tester, bank0: const Bank0Check.match());
    expect(find.textContaining('Updates are blocked'), findsNothing);
    expect(find.text('Install on Kitchen'), findsOneWidget);
  });

  group('how it ends', () {
    Future<void> end(WidgetTester tester, UpdateEnd end) async {
      final (FakeFeederPorts ports, _) = await _show(tester);
      await tapIn(tester, find.text('Install on Kitchen'));
      ports.progress.add(UpdateProgress.ended(end: end));
      await settle(tester);
    }

    testWidgets('on batteries', (tester) async {
      await end(tester, const UpdateEnd.refused(refusal: FeederRefusal.update(reason: UpdateRefusal.battery())));
      expect(find.text('Plug Kitchen in first'), findsOneWidget);
      expect(find.text('Try again'), findsNothing);
    });

    testWidgets('refused for the recovery copy, with what fixes it', (tester) async {
      await end(
        tester,
        const UpdateEnd.refused(
          refusal: FeederRefusal.update(reason: UpdateRefusal.fallback(reason: Bank0Blocked.changed)),
        ),
      );
      expect(find.text('Updates are blocked'), findsOneWidget);
      expect(find.textContaining('changed since it last checked itself in'), findsOneWidget);
      expect(find.textContaining('said no'), findsNothing);
    });

    testWidgets('an uncertain one is said, and offers nothing again', (tester) async {
      await end(tester, const UpdateEnd.uncertain());
      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(find.text('Try again'), findsNothing);
    });

    testWidgets('an end it can say may be offered again after Done', (tester) async {
      await end(tester, const UpdateEnd.notOffered());
      expect(find.textContaining('Nothing was sent'), findsOneWidget);
      await tapIn(tester, find.text('Done'));
      expect(find.text('Install on Kitchen'), findsOneWidget);
    });

    testWidgets('a cancel and a lost run say which copy it runs', (tester) async {
      await end(tester, const UpdateEnd.stopped(stop: UpdateStop.cancelled));
      expect(find.textContaining('Kitchen runs its other copy'), findsOneWidget);
      expect(find.textContaining('other copy of Wellfed'), findsOneWidget);
      expect(find.textContaining('still runs'), findsNothing);
      expect(find.text('Running now'), findsOneWidget);
    });

    // Bank 1 running some other build is not a fallback: the words must not
    // say the trial failed.
    testWidgets('another build on bank 1 says so, not that it went back', (tester) async {
      await end(tester, const UpdateEnd.wrongBuild());
      expect(find.text('Kitchen runs another version'), findsOneWidget);
      expect(find.textContaining("didn't send"), findsOneWidget);
      expect(find.textContaining("didn't start properly"), findsNothing);
    });

    testWidgets('a run that loses its stream is uncertain', (tester) async {
      final (FakeFeederPorts ports, _) = await _show(tester);
      await tapIn(tester, find.text('Install on Kitchen'));
      await ports.progress.close();
      await settle(tester);
      expect(find.textContaining('may or may not have'), findsOneWidget);
    });
  });
}
