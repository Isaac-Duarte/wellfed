import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/feeder_network.dart';
import 'package:wellfed/platform/wifi_name.dart';
import 'package:wellfed/state/install/install_state.dart' show FeederJoinProblem;

/// The computer as a test sets it: routes, addresses by interface, and each
/// interface's Wi-Fi name (absent where the system names none).
class _Routes {
  String? toFeeder;
  String? toAnywhere;
  Map<String, List<String>> addresses = {};
  Map<String, String> wifi = {};

  String? route(String to) => to == GuidedFeederNetwork.feeder ? toFeeder : toAnywhere;

  /// Runs while a name is read: a test may move the route, or hold the read.
  Future<void> Function()? reading;

  /// `address` on `interface` alone, which takes the route to the feeder.
  void on(String interface, String address, {String? named}) {
    addresses[interface] = [address];
    toFeeder = address;
    if (named != null) wifi[interface] = named;
  }

  GuidedFeederNetwork network() {
    final network = GuidedFeederNetwork(
      route: route,
      addresses: () async => {for (final MapEntry(:key, :value) in addresses.entries) key: List.of(value)},
      // Named as `iw` would name each interface's Wi-Fi.
      names: WifiNames(
        os: 'linux',
        run: (executable, arguments) async {
          await reading?.call();
          return switch ((executable, wifi[arguments[1]])) {
            ('iw', final String name) => '\tSSID: $name\n',
            _ => null,
          };
        },
      ),
    );
    _networks.add(network);
    return network;
  }
}

/// `join` started, its result or failure kept as they come.
class _Join {
  _Join(GuidedFeederNetwork network, String? home) {
    unawaited(network.join(home).then((from) => joined = from, onError: (Object error) => failed = error));
  }

  String? joined;
  Object? failed;

  FeederJoinProblem? get problem => switch (failed) {
    JoinRefused(:final FeederJoinProblem problem) => problem,
    _ => null,
  };
}

void main() {
  test('the home address is where the default route goes from', () {
    final routes = _Routes()..toAnywhere = '10.0.0.5';
    expect(routes.network().homeAddress(), '10.0.0.5');
  });

  testWidgets("a laptop on its home Wi-Fi joins when its Wi-Fi moves to a network named the feeder's", (tester) async {
    final routes = _Routes()..on('wlan0', '10.0.0.5', named: 'Maple St');
    final join = _Join(routes.network(), '10.0.0.5');
    await tester.pump(const Duration(seconds: 5));
    expect(join.joined, isNull);

    routes.on('wlan0', '192.168.1.100', named: 'PL-AF103-1A2B');
    await tester.pump(GuidedFeederNetwork.pollEvery);
    expect(join.joined, '192.168.1.100');
  });

  testWidgets('a wired computer joins with its Wi-Fi, the home address kept on the cable', (tester) async {
    final routes = _Routes()
      ..addresses = {
        'eth0': ['10.0.0.5'],
      }
      ..toFeeder = '10.0.0.5';
    final join = _Join(routes.network(), '10.0.0.5');
    // The watch reads where it starts.
    await tester.pump();
    routes
      ..addresses['wlan0'] = ['192.168.1.100']
      ..wifi['wlan0'] = 'PL-AF103-1A2B'
      ..toFeeder = '192.168.1.100';
    await tester.pump(GuidedFeederNetwork.pollEvery);
    expect(join.joined, '192.168.1.100');
  });

  testWidgets('a name the system will not give is a refusal, never a match', (tester) async {
    // A Mac without location, Windows 11 with location off.
    final routes = _Routes()..on('en0', '10.0.0.5');
    final join = _Join(routes.network(), '10.0.0.5');
    // The watch reads where it starts.
    await tester.pump();
    routes.on('en0', '192.168.1.100');
    await tester.pump(GuidedFeederNetwork.pollEvery);
    expect(join.joined, isNull);
    expect(join.problem, FeederJoinProblem.nameUnreadable);
  });

  testWidgets('a new 192.168.1.x network named otherwise is refused', (tester) async {
    final routes = _Routes()..on('wlan0', '10.0.0.5', named: 'Maple St');
    final join = _Join(routes.network(), '10.0.0.5');
    // The watch reads where it starts.
    await tester.pump();
    routes.on('wlan0', '192.168.1.57', named: 'Cafe');
    await tester.pump(GuidedFeederNetwork.pollEvery);
    expect(join.joined, isNull);
    expect(join.problem, FeederJoinProblem.otherNetwork);
  });

  testWidgets('another 192.168.1.x network already up is refused, never waited on', (tester) async {
    // Wired home at 10.0.0.5, and the Wi-Fi on a guest network or a travel
    // router, which takes the route to 192.168.1.1: the frame would go there.
    final routes = _Routes()
      ..addresses = {
        'eth0': ['10.0.0.5'],
        'wlan0': ['192.168.1.57'],
      }
      ..wifi['wlan0'] = 'Guest'
      ..toFeeder = '192.168.1.57';
    final join = _Join(routes.network(), '10.0.0.5');
    await tester.pump();
    expect(join.problem, FeederJoinProblem.otherNetwork);
  });

  testWidgets('a docked computer on a 192.168.1.x home network is not joined by unplugging its cable', (tester) async {
    // Cable and Wi-Fi both on the home network; the cable holds the route.
    final routes = _Routes()
      ..addresses = {
        'eth0': ['192.168.1.23'],
        'wlan0': ['192.168.1.24'],
      }
      ..wifi['wlan0'] = 'PL-AF103-spoofed-or-not'
      ..toFeeder = '192.168.1.23';
    final join = _Join(routes.network(), '192.168.1.23');
    await tester.pump(GuidedFeederNetwork.pollEvery);

    // Unplugged: the Wi-Fi's address, there all along, now takes the route.
    routes
      ..addresses.remove('eth0')
      ..toFeeder = '192.168.1.24';
    await tester.pump(const Duration(seconds: 30));
    expect(join.joined, isNull);
    expect(join.failed, isNull);
    _stopAll();
    await tester.pump(GuidedFeederNetwork.pollEvery);
  });

  testWidgets('a home network on 192.168.1.x over a cable is never taken for the feeder', (tester) async {
    // The route to 192.168.1.1 still goes over the cable to the home router,
    // even with the Wi-Fi on the feeder's network.
    final routes = _Routes()
      ..addresses = {
        'eth0': ['192.168.1.23'],
      }
      ..toFeeder = '192.168.1.23';
    final join = _Join(routes.network(), '192.168.1.23');
    // The watch reads where it starts.
    await tester.pump();
    routes
      ..addresses['wlan0'] = ['192.168.1.100']
      ..wifi['wlan0'] = 'PL-AF103-1A2B';
    await tester.pump(const Duration(minutes: 1));
    expect(join.joined, isNull);

    // The cable unplugged: the route goes over the feeder's Wi-Fi.
    routes
      ..addresses.remove('eth0')
      ..toFeeder = '192.168.1.100';
    await tester.pump(GuidedFeederNetwork.pollEvery);
    expect(join.joined, '192.168.1.100');
  });

  testWidgets("a computer already on the feeder's network as the handover starts is told to go home first", (
    tester,
  ) async {
    // The home address was read on the feeder's network.
    final routes = _Routes()..on('wlan0', '192.168.1.100', named: 'PL-AF103-1A2B');
    final join = _Join(routes.network(), '192.168.1.100');
    await tester.pump();
    expect(join.problem, FeederJoinProblem.alreadyOnFeeder);
  });

  testWidgets('a stop ends a name read that hangs', (tester) async {
    final routes = _Routes()
      ..on('wlan0', '10.0.0.5')
      ..reading = () => Completer<void>().future;
    final join = _Join(routes.network(), '10.0.0.5');
    await tester.pump();
    routes.on('wlan0', '192.168.1.100', named: 'PL-AF103-1A2B');
    await tester.pump(GuidedFeederNetwork.pollEvery);
    expect(join.failed, isNull);

    _stopAll();
    await tester.pump();
    expect(join.joined, isNull);
    expect(join.problem, FeederJoinProblem.notSeen);
    await tester.pump(GuidedFeederNetwork.pollEvery);
  });

  testWidgets('a route that moves while the name is read is read again, not answered', (tester) async {
    final routes = _Routes()..on('wlan0', '10.0.0.5');
    final join = _Join(routes.network(), '10.0.0.5');
    await tester.pump();
    // The name is the feeder's, but the route goes back home as it is read.
    routes
      ..on('wlan0', '192.168.1.100', named: 'PL-AF103-1A2B')
      ..addresses['eth0'] = ['10.0.0.5']
      ..reading = () async => routes.toFeeder = '10.0.0.5';
    await tester.pump(const Duration(seconds: 5));
    expect(join.joined, isNull);
    expect(join.failed, isNull);
    _stopAll();
    await tester.pump(GuidedFeederNetwork.pollEvery);
  });

  testWidgets('a home address on 192.168.1.x whose Wi-Fi has no name says the name could not be read', (tester) async {
    // Perhaps the feeder's own network, joined early, on a Mac without location.
    final routes = _Routes()..on('en0', '192.168.1.100');
    final join = _Join(routes.network(), '192.168.1.100');
    await tester.pump(const Duration(seconds: 10));
    _stopAll();
    await tester.pump();
    expect(join.problem, FeederJoinProblem.nameUnreadable);
    await tester.pump(GuidedFeederNetwork.pollEvery);
  });

  for (final String vpn in ['100.101.7.3', '8.8.4.4']) {
    testWidgets('a home address the feeder cannot reach ($vpn) is refused before anything is watched', (tester) async {
      final routes = _Routes()..toFeeder = '192.168.1.100';
      final join = _Join(routes.network(), vpn);
      await tester.pump();
      expect(join.problem, FeederJoinProblem.vpn);
    });
  }

  test('only private addresses are a home the feeder reaches', () {
    for (final String address in ['10.1.2.3', '172.16.0.1', '172.31.255.1', '192.168.0.9']) {
      expect(GuidedFeederNetwork.reachableHome(address), isTrue, reason: address);
    }
    for (final String address in ['172.32.0.1', '100.64.0.1', '8.8.8.8', '169.254.1.1', 'nonsense']) {
      expect(GuidedFeederNetwork.reachableHome(address), isFalse, reason: address);
    }
  });

  testWidgets('stopping the wait fails the join', (tester) async {
    final join = _Join(_Routes().network(), '10.0.0.5');
    await tester.pump(const Duration(seconds: 3));
    expect(join.failed, isNull);
    _stopAll();
    await tester.pump();
    expect(join.problem, FeederJoinProblem.notSeen);
    // The poll's last wait runs out with nobody on it.
    await tester.pump(GuidedFeederNetwork.pollEvery);
  });

  testWidgets('leaving waits for the default route to go home again', (tester) async {
    final routes = _Routes()..on('wlan0', '10.0.0.5');
    final GuidedFeederNetwork network = routes.network();
    final join = _Join(network, '10.0.0.5');
    // The watch reads where it starts.
    await tester.pump();
    routes.on('wlan0', '192.168.1.100', named: 'PL-AF103-1A2B');
    await tester.pump(GuidedFeederNetwork.pollEvery);
    expect(join.joined, '192.168.1.100');

    // On the feeder's network, which may or may not hand out a gateway.
    routes.toAnywhere = '192.168.1.100';
    String? back;
    unawaited(network.leave().then((at) => back = at));
    await tester.pump(const Duration(seconds: 10));
    expect(back, isNull);

    routes.toAnywhere = '10.0.0.5';
    await tester.pump(GuidedFeederNetwork.pollEvery);
    expect(back, '10.0.0.5');
  });

  testWidgets('a computer on a cable is home at once', (tester) async {
    final routes = _Routes()
      ..on('eth0', '10.0.0.5')
      ..toAnywhere = '10.0.0.5';
    final GuidedFeederNetwork network = routes.network();
    _Join(network, '10.0.0.5');
    // The watch reads where it starts.
    await tester.pump();
    routes
      ..addresses['wlan0'] = ['192.168.1.100']
      ..wifi['wlan0'] = 'PL-AF103-1A2B'
      ..toFeeder = '192.168.1.100';
    await tester.pump(GuidedFeederNetwork.pollEvery);
    String? back;
    unawaited(network.leave().then((at) => back = at));
    await tester.pump();
    expect(back, '10.0.0.5');
  });

  testWidgets('a computer that never comes back is given up on after the wait', (tester) async {
    var ended = false;
    String? back = 'unset';
    unawaited(
      _Routes().network().leave().then((at) {
        ended = true;
        back = at;
      }),
    );
    await tester.pump(GuidedFeederNetwork.leaveWait + GuidedFeederNetwork.pollEvery);
    expect(ended, isTrue);
    expect(back, isNull);
  });
}

/// The networks the tests made, so a test can stop the one it started.
final _networks = <GuidedFeederNetwork>[];

void _stopAll() {
  for (final GuidedFeederNetwork network in _networks) {
    network.stop();
  }
}
