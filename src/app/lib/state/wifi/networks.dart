/// One access point this phone heard. A network with several radios is
/// heard once for each.
class SeenNetwork {
  /// An access point heard on `frequencyMhz` at `level` dBm.
  const SeenNetwork({required this.ssid, required this.frequencyMhz, required this.level, required this.secured});

  /// The network's name.
  final String ssid;

  /// The channel's centre frequency: 2.4 GHz below 4900, 5 or 6 GHz above.
  final int frequencyMhz;

  /// How strongly this phone hears it, in dBm.
  final int level;

  /// It asks for a password.
  final bool secured;
}

/// Why this phone could not list the networks around it.
enum ScanProblem {
  /// Android lets no app list networks without location.
  permission,

  /// This phone's Wi-Fi is off.
  wifiOff,

  /// Location is off, which Android needs for a scan.
  locationOff,

  /// The phone answered nothing, as with no plugin under a test.
  unavailable,
}

/// What a scan of the networks around this phone found.
sealed class NetworkScan {
  const NetworkScan();
}

/// The networks heard, possibly none.
class NetworksSeen extends NetworkScan {
  /// The access points heard.
  const NetworksSeen(this.networks);

  /// Every access point, one entry for each radio.
  final List<SeenNetwork> networks;
}

/// No list: the name has to be typed.
class NetworksUnread extends NetworkScan {
  /// Unread for `problem`.
  const NetworksUnread(this.problem);

  /// Why.
  final ScanProblem problem;
}

/// A network to offer, its radios merged into one.
class NetworkChoice {
  /// The network `ssid`.
  const NetworkChoice({
    required this.ssid,
    required this.level,
    required this.secured,
    required this.joinable,
    required this.current,
  });

  /// The network's name.
  final String ssid;

  /// Its strongest radio the feeder could use, in dBm; null for this phone's
  /// network when the scan missed it.
  final int? level;

  /// It asks for a password.
  final bool secured;

  /// It has a 2.4 GHz radio, the only band the feeder joins.
  final bool joinable;

  /// The network this phone is on.
  final bool current;
}

/// The lowest 5 GHz channel's frequency: the feeder's radio stops below it.
const _fiveGhz = 4900;

/// Android quotes the network's name.
String unquotedNetwork(String name) => name.replaceAll(RegExp(r'^"|"$'), '');

/// `seen` as one choice per name: this phone's network first, then the ones
/// the feeder can join, each by signal, strongest first, then those it can't.
/// A network's level is its strongest 2.4 GHz radio, the one the feeder uses.
List<NetworkChoice> networkChoices(Iterable<SeenNetwork> seen, {String? current}) {
  final String? here = current == null ? null : unquotedNetwork(current);
  final byName = <String, List<SeenNetwork>>{};
  for (final network in seen) {
    if (network.ssid.isEmpty) continue;
    (byName[network.ssid] ??= []).add(network);
  }

  final choices = <NetworkChoice>[];
  for (final MapEntry(key: ssid, value: radios) in byName.entries) {
    final List<SeenNetwork> near = radios.where((radio) => radio.frequencyMhz < _fiveGhz).toList();
    final List<SeenNetwork> counted = near.isEmpty ? radios : near;
    choices.add(
      NetworkChoice(
        ssid: ssid,
        level: counted.map((radio) => radio.level).reduce((a, b) => a > b ? a : b),
        secured: radios.any((radio) => radio.secured),
        joinable: near.isNotEmpty,
        current: ssid == here,
      ),
    );
  }
  // The scan can miss the phone's own network; it is still the likeliest.
  if (here != null && here.isNotEmpty && !byName.containsKey(here)) {
    choices.add(NetworkChoice(ssid: here, level: null, secured: true, joinable: true, current: true));
  }
  choices.sort((a, b) {
    if (a.current != b.current) return a.current ? -1 : 1;
    if (a.joinable != b.joinable) return a.joinable ? -1 : 1;
    final int byLevel = (b.level ?? -200).compareTo(a.level ?? -200);
    return byLevel != 0 ? byLevel : a.ssid.compareTo(b.ssid);
  });
  return List.unmodifiable(choices);
}
