import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart' show RustOpaqueInterface;
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/restore_ports.dart';
import 'package:wellfed/screens/restore/restore_screen.dart';
import 'package:wellfed/src/rust/api/install.dart';
import 'package:wellfed/src/rust/api/restore.dart';
import 'package:wellfed/state/restore/restore.dart';

import '../support/fakes.dart';

/// A run the test drives.
class FakeRestoreRun implements RestoreRun, RustOpaqueInterface {
  // ignore: the test ends with it
  final updates = StreamController<RestoreUpdate>();
  final answers = <int, InstallAnswer>{};
  int stops = 0;

  @override
  Stream<RestoreUpdate> run() => updates.stream;

  @override
  void answer({required int id, required InstallAnswer answer}) => answers[id] = answer;

  @override
  void stop() => stops++;

  @override
  void dispose() {}

  @override
  bool get isDisposed => false;

  void event(InstallEvent event) => updates.add(RestoreUpdate.event(event: event));

  void end(RestoreEnd end) => updates.add(RestoreUpdate.ended(end: end));

  void stopped(RestoreStop stop) => end(RestoreEnd.stopped(case_: stop, detail: 'small print'));
}

class FakeRestorePorts implements RestorePorts {
  /// False to stand in for a desktop.
  @override
  bool restoresFeeder = true;

  String? kept = '/kept/stock-1.1.15.bin';
  final checks = <String, BackupCheck>{'/kept/stock-1.1.15.bin': const BackupCheck.ready(version: '1.1.15')};
  String? picks;
  bool holds = true;
  int released = 0;
  int forgotten = 0;
  final held = <String>[];
  final specs = <RestoreSpec>[];
  final runs = <FakeRestoreRun>[];

  @override
  Future<String> dataDir() async => '/data';

  @override
  String? keptBackup(String dataDir, List<int> identity) => kept;

  @override
  BackupCheck checkBackup(String path) => checks[path] ?? const BackupCheck.missing();

  @override
  Future<DateTime?> backupDate(String path) async => DateTime(2026, 10, 28);

  @override
  Future<String?> pickBackup() async => picks;

  @override
  RestoreRun newRun(RestoreSpec spec) {
    specs.add(spec);
    final run = FakeRestoreRun();
    runs.add(run);
    return run;
  }

  @override
  Future<String?> locate(String identity) async => located;

  /// What a read over a session says.
  RestoreSeen seen = RestoreSeen.idle;

  @override
  Future<RestoreSeen> readFeeder(List<int> identity, String address) async => seen;

  /// Where the feeder shows up on the network, if it does.
  String? located;

  // ignore: close_sinks, the test ends with it
  final heard = StreamController<void>.broadcast();

  @override
  Stream<void> setupHeard() => heard.stream;

  @override
  void forgetKey(String dataDir, List<int> identity) => forgotten++;

  /// Lets a test hold [holdProcess] open.
  Completer<void>? holding;

  @override
  Future<bool> holdProcess(String text) async {
    held.add(text);
    await holding?.future;
    return holds;
  }

  @override
  Future<void> releaseProcess() async => released++;
}

final restoreFlowIdentity = List<int>.filled(32, 0xab);

RestoreFlow restoreFlow(FakeRestorePorts ports) =>
    RestoreFlow(ports, identity: restoreFlowIdentity, feederName: 'Kitchen', address: '10.0.0.5:7700');

/// The screen's stages, drawn from the flow itself: no Rust behind
/// [identityBytes], which [RestoreScreen] calls.
class RestoreHarness extends StatelessWidget {
  const RestoreHarness(this.flow, {super.key, this.brightness = Brightness.light});

  final RestoreFlow flow;
  final Brightness brightness;

  @override
  Widget build(BuildContext context) => themed(RestoreView(flow: flow), brightness: brightness);
}

Future<RestoreFlow> restoreOpen(
  WidgetTester tester,
  FakeRestorePorts ports, {
  Brightness brightness = Brightness.light,
}) async {
  tester.view.physicalSize = const Size(1236, 2745);
  tester.view.devicePixelRatio = 3;
  addTearDown(tester.view.reset);
  final RestoreFlow flow = restoreFlow(ports);
  await flow.load();
  await tester.pumpWidget(RestoreHarness(flow, brightness: brightness));
  await settle(tester);
  return flow;
}

Future<FakeRestoreRun> restoreStarted(WidgetTester tester, FakeRestorePorts ports) async {
  await tester.tap(find.text('I understand'));
  await settle(tester);
  await tester.tap(find.widgetWithText(FilledButton, 'Restore original firmware'));
  await settle(tester);
  return ports.runs.last;
}
