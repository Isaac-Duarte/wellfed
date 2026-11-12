import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:wellfed/src/rust/api/error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/src/rust/api/store.dart';
import 'package:wellfed/state/fleet/app_state.dart';

/// The feeders the app knows, in memory.
class FakeBook extends ChangeNotifier implements FeederBook {
  FakeBook({this.label = 'Pixel'});
  @override
  Appearance appearance = Appearance.system;

  @override
  Future<void> setAppearance(Appearance next) async {
    appearance = next;
    notifyListeners();
  }

  @override
  String label;

  @override
  List<KnownFeeder> feeders = [];

  final remembered = <(KnownFeeder, bool)>[];

  /// What the next save throws, as a full or read-only disk would.
  Object? saveFails;

  /// While set, a label is saved only once it completes: a slow disk.
  Completer<void>? labelGate;

  @override
  Future<void> setLabel(String label) async {
    await labelGate?.future;
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (saveFails case final error?) throw error;
    this.label = label;
    notifyListeners();
  }

  @override
  Future<void> remember(KnownFeeder feeder, {bool paired = false, bool keepName = false}) async {
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (saveFails case final error?) throw error;
    remembered.add((feeder, paired));
    final Iterable<KnownFeeder> known = feeders.where((f) => f.identity == feeder.identity);
    feeders = [
      for (final other in feeders)
        if (other.identity != feeder.identity) other,
      KnownFeeder(
        identity: feeder.identity,
        name: keepName && known.isNotEmpty ? known.first.name : feeder.name,
        address: feeder.address,
        manualAddress: feeder.manualAddress ?? known.firstOrNull?.manualAddress,
      ),
    ];
    notifyListeners();
  }

  /// Every address a session answered at, in order.
  final answered = <String>[];

  @override
  Future<void> answeredAt(String identity, String address) async => answered.add(address);

  @override
  Future<void> setManualAddress(String identity, String? address) async {
    // ignore: only_throw_errors, this stands in for the bridge, whose errors aren't Exceptions.
    if (saveFails case final error?) throw error;
    feeders = [
      for (final feeder in feeders)
        if (feeder.identity == identity)
          KnownFeeder(identity: feeder.identity, name: feeder.name, address: feeder.address, manualAddress: address)
        else
          feeder,
    ];
    notifyListeners();
  }

  @override
  Future<void> forget(String identity) async {
    feeders = [
      for (final feeder in feeders)
        if (feeder.identity != identity) feeder,
    ];
    notifyListeners();
  }

  /// Set to make the next save fail, as a full disk would.
  BridgeError? renameFails;

  @override
  Future<void> rename(String identity, String name) async {
    if (renameFails case final failure?) throw failure;
    feeders = [
      for (final feeder in feeders)
        if (feeder.identity == identity)
          KnownFeeder(
            identity: feeder.identity,
            name: name,
            address: feeder.address,
            manualAddress: feeder.manualAddress,
          )
        else
          feeder,
    ];
    notifyListeners();
  }

  final _clockDue = <String>{};

  void clockOwed(String identity) => _clockDue.add(identity);

  @override
  bool clockDue(String identity) => _clockDue.contains(identity);

  @override
  void clockSynced(String identity) => _clockDue.remove(identity);

  @override
  KnownServer? server;

  /// While set, the next write of [server] waits for it: a slow disk.
  Future<void>? serverWrite;

  @override
  Future<void> setServer(KnownServer? server) async {
    // Only the next write is slow.
    final Future<void>? slow = serverWrite;
    serverWrite = null;
    await slow;
    this.server = server;
    notifyListeners();
  }
}
