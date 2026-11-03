import 'dart:typed_data';

import 'package:clock/clock.dart';
import 'package:flutter/material.dart' hide Card;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/join_ports.dart';
import 'package:wellfed/screens/feeder/clock_portion.dart';
import 'package:wellfed/screens/feeder/diagnostics.dart';
import 'package:wellfed/screens/feeder/invite.dart';
import 'package:wellfed/screens/feeder/lights_help.dart';
import 'package:wellfed/screens/feeder/meal_editor.dart';
import 'package:wellfed/screens/feeder/people.dart';
import 'package:wellfed/screens/feeder/tabs.dart';
import 'package:wellfed/screens/feeder/updates.dart';
import 'package:wellfed/screens/home/home.dart';
import 'package:wellfed/screens/install/install_progress.dart';
import 'package:wellfed/screens/setup/join.dart';
import 'package:wellfed/screens/setup/pairing_view.dart';
import 'package:wellfed/screens/setup/recover_screen.dart';
import 'package:wellfed/screens/setup/setup_screen.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/join.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/fleet.dart';
import 'package:wellfed/state/install/install.dart';
import 'package:wellfed/state/join/join.dart';
import 'package:wellfed/state/pairing/pairing.dart';
import 'package:wellfed/state/setup/setup.dart';

import '../support/fake_install.dart';
import '../support/fakes.dart';
import '../support/harness.dart';
import 'golden_support.dart';

/// The build the app carries, newer than the fixture's.
const _carried = 'cd00000000000000000000000000000000000000000000000000000000000000';

/// `card` as it would read running `build`, or with its clock read as `time`.
Card _changed(Card card, {String? build, TimeInfo? time}) => Card(
  build: build ?? card.build,
  food: card.food,
  locked: card.locked,
  ready: card.ready,
  blocked: card.blocked,
  adapter: card.adapter,
  batteryPercent: card.batteryPercent,
  clockSet: card.clockSet,
  rssi: card.rssi,
  fault: card.fault,
  desiccant: card.desiccant,
  uptimeMs: card.uptimeMs,
  bank: card.bank,
  nextMeal: card.nextMeal,
  lastFed: card.lastFed,
  meals: card.meals,
  time: time ?? card.time,
  settings: card.settings,
  plans: card.plans,
  wifiGeneration: card.wifiGeneration,
);

/// The fixture's feeder with an update screen to read: the app carries
/// version 7, and the feeder runs `build`.
Future<FeederController> _updating(WidgetTester tester, String build) async {
  final FakeFeederPorts ports = fixturePhone()
    ..image = const ImageCheck.ready(provisioning: ImageProvisioning.consumer, build: _carried, version: 7);
  final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: ports);
  ports.controller = feeder;
  session.status(_changed(fullCard(), build: build));
  await tester.pump();
  session.replies.add(
    const FeederReply.update(
      update: UpdateInfo(phase: UpdatePhase.idle, bank: 1, minVersion: 6, version: 0, imageLen: 0, written: 0),
    ),
  );
  return feeder;
}

const _bootCounts = Counts(
  uptimeS: 3 * 86400 + 4 * 3600,
  feeds: 9,
  feedsFailed: 1,
  portions: 18,
  presses: 2,
  sessions: 14,
  reconnects: 1,
  dhcpRenewals: 6,
  outages: 0,
  batteryS: 0,
);

const _lifetimeCounts = Counts(
  uptimeS: 41 * 86400,
  feeds: 143,
  feedsFailed: 3,
  portions: 290,
  presses: 31,
  sessions: 220,
  reconnects: 12,
  dhcpRenewals: 80,
  outages: 2,
  batteryS: 5400,
);

/// What a feeder three days up tells Diagnostics, in the order it asks.
List<FeederReply> _diagnostics() => [
  const FeederReply.stats(
    stats: Stats.counters(
      Counters(
        boot: _bootCounts,
        reset: ResetCause.powerOn,
        lifetime: _lifetimeCounts,
        resets: Resets(powerOn: 40, watchdog: 1, software: 3, brownout: 0, other: 0),
        handshakeMaxMs: 420,
        handshakeMeanMs: 180,
      ),
    ),
  ),
  const FeederReply.stats(stats: Stats.memory(Memory(heapFree: 200000, heapMin: 133120, tasks: [], tasksTotal: 0))),
  FeederReply.stats(
    stats: Stats.flash(
      Flash(
        boot: Erases(storage: Uint32List.fromList([1]), update: 0),
        lifetime: Erases(storage: Uint32List.fromList([3, 40, 7]), update: 2),
        eventsKept: 412,
        eventsCapacity: 1024,
      ),
    ),
  ),
];

/// An owner, the phone this is, and two more: one member and one owner.
FeederReply _people() => FeederReply.clients(
  clients: [
    ClientView(key: Uint8List.fromList([1]), fingerprint: '01', label: 'Pixel', role: ClientRole.owner, you: true),
    ClientView(
      key: Uint8List.fromList([2]),
      fingerprint: '02',
      label: "Sam's phone",
      role: ClientRole.member,
      you: false,
    ),
    ClientView(
      key: Uint8List.fromList([3]),
      fingerprint: '03',
      label: 'Kitchen tablet',
      role: ClientRole.owner,
      you: false,
    ),
  ],
);

class _NoJoin implements JoinPorts {
  const _NoJoin();

  @override
  String? inviteFeeder(String text) => null;

  @override
  Future<JoinAnswer> join(String invite, String label, String address) => throw UnimplementedError();

  @override
  Future<String?> locate(String identity, Duration wait) async => null;
}

/// `time` with the feeder's clock read at the fixture's moment, a third of
/// a second out at most.
TimeInfo? _readClock(TimeInfo? time) => switch (time) {
  null => null,
  final time => TimeInfo(
    generation: time.generation,
    writable: time.writable,
    zone: time.zone,
    ntp: time.ntp,
    clockMs: fixtureNow.millisecondsSinceEpoch,
    errorMs: 300,
    offsetS: time.offsetS,
    syncs: time.syncs,
    last: time.last,
    lastAtMs: time.lastAtMs,
    ntpLast: time.ntpLast,
  ),
};

const _hallway = KnownFeeder(identity: 'bb', name: 'Hallway', address: 'lan:4748');

/// The fixture's two meals: breakfast at 8 every day, and dinner at 6 paused.
final _plans = PlansView(
  generation: 3,
  writable: true,
  plans: [
    const PlanView(minute: 8 * 60, portions: 2, days: Weekday.values, enabled: true, call: 0),
    const PlanView(minute: 18 * 60, portions: 1, days: Weekday.values, enabled: false, call: 2),
    ...List<PlanView?>.filled(8, null),
  ],
);

PairingView _pairing(PairingState state) => PairingView(
  flow: PairingFlow(FakeBook(), FakePairingPorts())..show(state),
  ble: true,
  onRetry: () {},
  onNewCode: () {},
);

void main() {
  setUpGoldens('golden_screens_test.dart');

  for (final Brightness brightness in Brightness.values) {
    group(brightness.name, () {
      for (final (String name, int? slot) in [('meal_new', null), ('meal_edit', 0)]) {
        golden(name, (tester) async {
          final FeederController feeder = await goldenFeeder(tester);
          await showPhone(tester, overBlank(MealEditor(feeder: feeder, held: _plans, slot: slot)), brightness);
          await matchesGolden(name, brightness);
        });
      }

      golden('people', (tester) async {
        final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
        answerAsFull(session);
        final Future<FeederReply> Function(FeederRequest)? full = session.answering;
        session.answering = (request) async => switch ((request, full)) {
          (FeederRequest_ListClients(), _) => _people(),
          (_, final answer?) => answer(request),
          _ => const FeederReply.done(),
        };
        session.status(fullCard());
        await tester.pump();
        await showPhone(tester, overBlank(PeopleScreen(feeder: feeder)), brightness);
        await matchesGolden('people', brightness);
      });

      golden('clock and portion size', (tester) async {
        final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
        answerAsFull(session);
        session.status(_changed(fullCard(), time: _readClock(fullCard().time)));
        await tester.pump();
        await showPhone(tester, overBlank(ClockPortionScreen(feeder: feeder)), brightness);
        await matchesGolden('clock_portion', brightness);
      });

      golden('diagnostics', (tester) async {
        final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
        session.status(fullCard());
        await tester.pump();
        session.replies.addAll(_diagnostics());
        await showPhone(tester, overBlank(DiagnosticsScreen(feeder: feeder)), brightness);
        await matchesGolden('diagnostics', brightness);
      });

      for (final (String name, String build) in [
        ('updates_offered', fullCard().build),
        ('updates_current', _carried),
      ]) {
        golden(name, (tester) async {
          final FeederController feeder = await _updating(tester, build);
          await showPhone(tester, overBlank(UpdatesScreen(feeder: feeder)), brightness);
          await matchesGolden(name, brightness);
        });
      }

      golden('lights help', (tester) async {
        await showPhone(tester, overBlank(const LightsHelp()), brightness);
        await decodeImages(tester);
        await matchesGolden('lights_help', brightness);
      });

      golden('joining', (tester) async {
        await showPhone(
          tester,
          overBlank(
            JoinScreen(
              newFlow: () => JoinFlow(FakeBook(), const _NoJoin()),
              // The camera's place, as a phone shows it before the picture comes.
              scanner: (_) => const ColoredBox(color: Color(0xFF000000)),
            ),
          ),
          brightness,
        );
        await matchesGolden('join', brightness);
      });

      golden('recovery', (tester) async {
        final book = FakeBook()..label = 'Pixel';
        await showPhone(
          tester,
          overBlank(RecoverScreen(newFlow: () => RecoverFlow(book, FakePairingPorts(), FakePhonePorts()))),
          brightness,
        );
        await decodeImages(tester);
        await matchesGolden('recover', brightness);
      });

      golden('an invite', (tester) async {
        await showPhone(
          tester,
          overBlank(
            InviteScreen(
              invite: const InviteView(
                text: 'wellfed-invite:1:8f3a92c1d07e4b55a6c2f18e9d3b70a4',
                seconds: 600,
                role: ClientRole.member,
              ),
              share: (_) async {},
            ),
          ),
          brightness,
        );
        await matchesGolden('invite', brightness);
      });

      golden("setup's Wi-Fi step", (tester) async {
        final FakeFeederPorts phone = fixturePhone();
        await showPhone(
          tester,
          overBlank(
            WifiStep(
              initial: null,
              currentNetwork: phone.currentNetwork,
              scanNetworks: phone.scanNetworks,
              stepLabel: 'Step 2 of 4',
              progress: 0.5,
              onBack: () {},
              onDone: (_) {},
            ),
          ),
          brightness,
        );
        await matchesGolden('setup_wifi', brightness);
      });

      golden("setup's code", (tester) async {
        await showPhone(
          tester,
          overBlank(
            _pairing(
              PairingCode(
                code: const [4, 1, 2, 3, 3, 2],
                deadline: clock.now().add(const Duration(seconds: 48)),
                feederName: 'Kitchen',
              ),
            ),
          ),
          brightness,
        );
        await matchesGolden('setup_code', brightness);
      });

      golden('an install sending', (tester) async {
        await showPhone(
          tester,
          overBlank(
            InstallSendingPage(
              flow: InstallFlow(FakeBook(), FakeInstallPorts()),
              state: const InstallSending(InstallFacts(), holdRefused: false, percent: 40, restarting: false),
            ),
          ),
          brightness,
        );
        await decodeImages(tester);
        await matchesGolden('install_sending', brightness);
      });

      golden('today offline', (tester) async {
        final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
        answerAsFull(session);
        session.status(fullCard());
        await tester.pump();
        session.updates.add(
          const SessionUpdate.closed(
            error: BridgeError(kind: ErrorKind.closed, message: 'gone'),
          ),
        );
        await session.updates.close();
        await tester.pump();
        await showPhone(tester, shell(feeder, todayTab), brightness);
        await matchesGolden('today_offline', brightness);
        // Let the next try start, so no backoff is left pending at the end.
        await tester.pump(const Duration(minutes: 1));
      });

      golden('home with two feeders', (tester) async {
        final book = FakeBook()..feeders = [kitchen, _hallway];
        final ports = FakeFeederPorts();
        final fleet = FeederFleet(book, ports);
        addTearDown(fleet.dispose);
        await tester.pump();
        final [FakeSession first, FakeSession second] = ports.sessions;
        for (final FakeSession session in ports.sessions) {
          session.opened();
          answerAsFull(session);
        }
        await tester.pump();
        first.status(fullCard());
        second.status(fakeCard(food: FoodLevel.low, nextMeal: fullCard().nextMeal, lastFed: fixtureFed));
        await tester.pump();
        await showPhone(
          tester,
          HomeScreen(fleet: fleet, earlierInstall: () async => PendingInstall.nothing),
          brightness,
        );
        await decodeImages(tester);
        await matchesGolden('home_two', brightness);
      });
    });
  }
}
