import 'dart:async';
import 'dart:io';

import 'package:clock/clock.dart';
import 'package:wellfed/platform/wifi_name.dart';
import 'package:wellfed/src/rust/api/install.dart' show routeSource;
import 'package:wellfed/state/install/install_state.dart' show FeederJoinProblem;

/// Why a computer would not be counted as on the feeder's network.
class JoinRefused implements Exception {
  /// Refused for `problem`.
  const JoinRefused(this.problem);

  /// What the owner has to change.
  final FeederJoinProblem problem;

  @override
  String toString() => switch (problem) {
    FeederJoinProblem.notSeen => "this computer didn't join the feeder's network",
    FeederJoinProblem.otherNetwork => 'this computer is on another 192.168.1.x network',
    FeederJoinProblem.vpn => "this computer's address is one the feeder can't reach, as a VPN's is",
    FeederJoinProblem.nameUnreadable => "this computer didn't say which Wi-Fi it joined",
    FeederJoinProblem.alreadyOnFeeder => "this computer was already on the feeder's network",
  };
}

/// This computer's IPv4 addresses, by interface name.
typedef ReadAddresses = Future<Map<String, List<String>>> Function();

/// The feeder's setup network on a computer, which no desktop lets an app join
/// in a portable way. The owner picks it in the computer's own Wi-Fi menu, and
/// this sees it happen. The setup frame carries the home Wi-Fi's password, so
/// the join fails closed: the frame goes only once all of these hold.
///
/// - The route toward the feeder, 192.168.1.1, leaves from an address on
///   192.168.1.0/24 that no interface had when the watch began, and that is
///   not the home address the frame names. A docked computer whose cable and
///   Wi-Fi are both on a home network on those addresses already has its
///   Wi-Fi's address, so unplugging the cable is not a join.
/// - The interface holding that address is on a Wi-Fi network the system
///   names, and the name starts `PL-AF103-`. A name the system won't give
///   (macOS without location, Windows 11 with location off, an interface
///   that isn't Wi-Fi) is a refusal that says why, never a match.
///
/// A route that already leaves from another 192.168.1.x address when the
/// watch begins (a guest network, an ISP's or a travel router) is refused, as
/// is a new one on a network named otherwise, since the frame would reach that
/// router. A matching name is no proof either: anyone can name a network
/// `PL-AF103-…`, as on a phone. The feeder is not asked anything to tell:
/// stock takes one frame, so nothing else is sent to it.
class GuidedFeederNetwork {
  /// Reads routes through `route`, addresses through `addresses` and Wi-Fi
  /// names through `names`, the system's unless a test gives others.
  GuidedFeederNetwork({String? Function(String to)? route, ReadAddresses? addresses, WifiNames? names})
    : _route = route ?? _bridgeRoute,
      _addresses = addresses ?? _systemAddresses,
      _names = names ?? WifiNames();

  final String? Function(String to) _route;
  final ReadAddresses _addresses;
  final WifiNames _names;

  /// The feeder's address on its setup network.
  static const feeder = '192.168.1.1';

  /// The feeder's setup network hands out addresses in 192.168.1.0/24.
  static const setupSubnet = '192.168.1.';

  /// How the feeder's setup network is named: its model, then the end of its
  /// MAC address.
  static const feederPrefix = 'PL-AF103-';

  /// Any address off every local link, so its route is the default one: the
  /// way this computer reaches its home network. TEST-NET-1, which nothing
  /// answers; nothing is sent to it.
  static const _anywhere = '192.0.2.1';

  /// How often the routes are read while the owner switches networks. A
  /// computer takes a few seconds to get an address on a network it joins.
  static const pollEvery = Duration(seconds: 1);

  /// How long the owner has to come back home after the frame. The run waits
  /// two minutes for stock after the frame before it stops, so a later return
  /// finds nothing waiting.
  static const leaveWait = Duration(minutes: 2);

  /// This computer's address on the feeder's link, once joined.
  String? _feederLink;
  Completer<void>? _stop;

  /// Where the feeder reaches this computer: the address its default route
  /// goes from, or `null` with no network.
  String? homeAddress() => _route(_anywhere);

  /// The Wi-Fi the home address is on, when the system names it: the network
  /// to go back to. Never throws.
  Future<String?> homeNetwork() async {
    try {
      final String? home = homeAddress();
      if (home == null) return null;
      return switch (await _nameOf(home)) {
        WifiNamed(:final String name) => name,
        WifiUnnamed() => null,
      };
    } on Object catch (_) {
      // Only words on the screen hang on this: with none, it says "your home
      // network".
      return null;
    }
  }

  /// Wait until this computer is on the feeder's network; its address there.
  /// `home` is the address the frame names, where the feeder reaches this
  /// computer. Throws [JoinRefused] with what the owner has to change, and
  /// once [stop] is called.
  Future<String> join(String? home) async {
    final stop = Completer<void>();
    _stop = stop;
    try {
      if (home != null && !reachableHome(home)) throw const JoinRefused(FeederJoinProblem.vpn);
      final Set<String> had = {for (final List<String> addresses in (await _addresses()).values) ...addresses};
      final String? before = _route(feeder);
      if (before != null && before.startsWith(setupSubnet)) {
        if (before != home) throw const JoinRefused(FeederJoinProblem.otherNetwork);
        // The home address itself on the feeder's network: the owner joined
        // it before the handover read where home is.
        if (await _nameOf(before) case WifiNamed(:final name) when name.startsWith(feederPrefix)) {
          throw const JoinRefused(FeederJoinProblem.alreadyOnFeeder);
        }
      }
      while (!stop.isCompleted) {
        final String? from = _route(feeder);
        if (from != null && from.startsWith(setupSubnet) && from != home && !had.contains(from)) {
          final WifiName named = await _nameOf(from);
          // The name was read across a wait: the owner may have stopped.
          if (stop.isCompleted) break;
          switch (named) {
            case WifiNamed(:final name) when name.startsWith(feederPrefix):
              _feederLink = from;
              return from;
            case WifiNamed():
              throw const JoinRefused(FeederJoinProblem.otherNetwork);
            case WifiUnnamed():
              throw const JoinRefused(FeederJoinProblem.nameUnreadable);
          }
        }
        await Future.any([Future<void>.delayed(pollEvery), stop.future]);
      }
      throw const JoinRefused(FeederJoinProblem.notSeen);
    } finally {
      if (identical(_stop, stop)) _stop = null;
    }
  }

  /// The Wi-Fi of the interface holding `address`.
  Future<WifiName> _nameOf(String address) async {
    for (final MapEntry(key: String interface, value: List<String> addresses) in (await _addresses()).entries) {
      if (addresses.contains(address)) return _names.of(interface);
    }
    return const WifiUnnamed();
  }

  static Future<Map<String, List<String>>> _systemAddresses() async => {
    for (final NetworkInterface interface in await NetworkInterface.list(type: InternetAddressType.IPv4))
      interface.name: [for (final InternetAddress address in interface.addresses) address.address],
  };

  /// Whether a feeder on the home network can reach `address`: a private
  /// address (RFC 1918). A VPN's tunnel address, a public one or one in the
  /// carrier-grade range 100.64.0.0/10 (Tailscale's, say) it can't.
  static bool reachableHome(String address) {
    final List<int>? octets = _octets(address);
    if (octets == null) return false;
    return switch (octets) {
      [10, _, _, _] => true,
      [172, final int b, _, _] => b >= 16 && b <= 31,
      [192, 168, _, _] => true,
      _ => false,
    };
  }

  static List<int>? _octets(String address) {
    final List<int?> parts = [for (final String part in address.split('.')) int.tryParse(part)];
    if (parts.length != 4 || parts.any((part) => part == null || part < 0 || part > 255)) return null;
    return [for (final int? part in parts) part ?? 0];
  }

  /// Stop waiting for a join.
  void stop() {
    final Completer<void>? stop = _stop;
    if (stop != null && !stop.isCompleted) stop.complete();
  }

  /// Wait until this computer is off the feeder's link, back on its home
  /// network: the address its default route then goes from, or `null` if it
  /// does not come back within [leaveWait]. A computer that kept its home
  /// network on a cable is back at once.
  Future<String?> leave() async {
    final String? link = _feederLink;
    _feederLink = null;
    final DateTime until = clock.now().add(leaveWait);
    while (clock.now().isBefore(until)) {
      final String? at = homeAddress();
      if (at != null && at != link) return at;
      await Future<void>.delayed(pollEvery);
    }
    return null;
  }

  static String? _bridgeRoute(String to) => routeSource(to: to);
}
